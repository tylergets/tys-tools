use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const PAUSE_FILE_SUFFIX: &str = "paused";

pub fn is_capture_paused() -> Result<bool> {
    Ok(pause_path().exists())
}

pub fn set_capture_paused(paused: bool) -> Result<()> {
    set_capture_paused_at_path(&pause_path(), paused)
}

fn set_capture_paused_at_path(path: &Path, paused: bool) -> Result<()> {
    if paused {
        fs::write(path, b"paused\n").with_context(|| {
            format!(
                "failed to write tyboard capture pause marker: {}",
                path.display()
            )
        })?;
        return Ok(());
    }

    if path.exists() {
        fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove tyboard capture pause marker: {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

/// The marker file that exists while capture is paused.
pub fn pause_path() -> PathBuf {
    ty_common::runtime_dir().join(format!(
        "tyboard-{}-{PAUSE_FILE_SUFFIX}",
        ty_common::user_hint()
    ))
}

#[cfg(test)]
mod tests {
    use super::{is_capture_paused, set_capture_paused_at_path};
    use std::path::PathBuf;

    fn test_pause_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tyboard-test-paused-{}-{}",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn writes_pause_marker_when_paused() {
        let path = test_pause_path("pause");
        set_capture_paused_at_path(&path, true).expect("pause marker write should succeed");
        assert!(path.exists(), "pause marker should exist");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn removes_pause_marker_when_resumed() {
        let path = test_pause_path("resume");
        std::fs::write(&path, b"paused\n").expect("seed pause marker");
        set_capture_paused_at_path(&path, false).expect("pause marker removal should succeed");
        assert!(!path.exists(), "pause marker should be removed");
    }

    #[test]
    fn reports_default_state_without_crashing() {
        let _ = is_capture_paused().expect("default paused lookup should not fail");
    }
}
