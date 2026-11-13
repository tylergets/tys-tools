//! Detect whitelisted apps even when their windows are minimized or closed to the tray.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Context, Result};

/// Exact, case-insensitive process names or executable basenames owned by this user.
/// Processes may exit while being inspected; unreadable entries are skipped.
pub fn running(whitelist: &[String]) -> Result<Vec<String>> {
    running_in(Path::new("/proc"), unsafe { libc::geteuid() }, whitelist)
}

fn running_in(root: &Path, uid: u32, whitelist: &[String]) -> Result<Vec<String>> {
    if whitelist.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = vec![false; whitelist.len()];
    for entry in fs::read_dir(root).context("reading processes for app whitelist")? {
        let Ok(entry) = entry else { continue };
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let path = entry.path();
        if !path.metadata().is_ok_and(|metadata| metadata.uid() == uid) {
            continue;
        }
        let comm = fs::read_to_string(path.join("comm")).ok();
        let exe = fs::read_link(path.join("exe")).ok();
        let executable = exe
            .as_deref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        for (app, found) in whitelist.iter().zip(&mut found) {
            *found |= comm
                .as_deref()
                .is_some_and(|name| name.trim_end_matches('\n').eq_ignore_ascii_case(app))
                || executable.is_some_and(|name| name.eq_ignore_ascii_case(app));
        }
    }
    Ok(whitelist
        .iter()
        .zip(found)
        .filter(|(_, found)| *found)
        .map(|(app, _)| app.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ProcDir(PathBuf);

    impl Drop for ProcDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn detects_owned_apps_by_exact_name_until_the_last_process_exits() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = ProcDir(
            std::env::temp_dir().join(format!("tylight-apps-{}-{nonce}", std::process::id())),
        );
        for pid in ["1", "2", "3", "self"] {
            fs::create_dir_all(root.0.join(pid)).unwrap();
        }
        fs::write(root.0.join("1/comm"), "Steam\n").unwrap();
        fs::write(root.0.join("2/comm"), "renamed\n").unwrap();
        symlink("/opt/steam/steam", root.0.join("2/exe")).unwrap();
        fs::write(root.0.join("3/comm"), "steamwebhelper\n").unwrap();
        fs::write(root.0.join("self/comm"), "steam\n").unwrap();
        let uid = root.0.metadata().unwrap().uid();
        let whitelist = vec!["steam".into()];
        assert_eq!(running_in(&root.0, uid, &whitelist).unwrap(), whitelist);
        assert!(running_in(&root.0, uid.wrapping_add(1), &whitelist)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(root.0.join("1")).unwrap();
        assert_eq!(running_in(&root.0, uid, &whitelist).unwrap(), whitelist);
        fs::remove_dir_all(root.0.join("2")).unwrap();
        assert!(running_in(&root.0, uid, &whitelist).unwrap().is_empty());
    }
}
