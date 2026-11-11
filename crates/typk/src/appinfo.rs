//! Which program is asking: its name from the process tree (skipping shells and
//! wrappers), and its icon from the desktop entries, when it has one.

use std::fs;
use std::path::{Path, PathBuf};

/// Processes that only run something else; the app is further up the tree.
const WRAPPERS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "nu",
    "timeout",
    "env",
    "nice",
    "ionice",
    "sudo",
    "doas",
    "nix",
    "nix-shell",
    "direnv",
    "script",
    "xargs",
    "make",
    "just",
];
const MAX_DEPTH: u32 = 12;

/// An asking program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub pid: u32,
    /// Shown as the prompt's title, e.g. `Claude`.
    pub name: String,
}

impl App {
    /// The nearest ancestor of `pid` (itself included) that is not a shell or wrapper.
    pub fn of_process(pid: u32) -> Option<Self> {
        let mut current = pid;
        for _ in 0..MAX_DEPTH {
            let comm = process_name(current)?;
            if !WRAPPERS.contains(&comm.as_str()) {
                return Some(Self {
                    pid: current,
                    name: pretty(&comm),
                });
            }
            current = parent_pid(current)?;
            if current <= 1 {
                return None;
            }
        }
        None
    }

    /// The icon from a matching desktop entry: a theme icon name or a file path.
    pub fn icon(&self) -> Option<String> {
        icon_for(&self.name)
    }
}

/// `/proc/<pid>/comm`.
pub fn process_name(pid: u32) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let comm = comm.trim();
    (!comm.is_empty()).then(|| comm.to_string())
}

/// Field 4 of `/proc/<pid>/stat`.
pub fn parent_pid(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, after_comm) = stat.rsplit_once(')')?;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

/// `.claude-wrapped` -> `Claude`, `gnome-disks` -> `Gnome-disks`.
pub fn pretty(comm: &str) -> String {
    let base = comm.trim_start_matches('.');
    let base = base.strip_suffix("-wrapped").unwrap_or(base);
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => comm.to_string(),
    }
}

/// The `Icon=` of the first desktop entry for `app`: by `Name`, by its file name, or by
/// the program `Exec` runs (all case-insensitive).
pub fn icon_for(app: &str) -> Option<String> {
    let wanted = app.to_lowercase();
    for dir in desktop_dirs() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "desktop"))
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let entry = DesktopEntry::parse(&text);
            if entry.matches(&wanted, &path) {
                if let Some(icon) = entry.icon {
                    return Some(icon);
                }
            }
        }
    }
    None
}

/// `$XDG_DATA_HOME/applications` and every `$XDG_DATA_DIRS/applications`.
fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
        dirs.push(Path::new(dir).join("applications"));
    }
    dirs
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DesktopEntry {
    name: Option<String>,
    icon: Option<String>,
    /// The program `Exec` runs, without its directory.
    exec: Option<String>,
}

impl DesktopEntry {
    /// The main `[Desktop Entry]` group only.
    fn parse(text: &str) -> Self {
        let mut entry = Self::default();
        let mut in_main = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_main = line == "[Desktop Entry]";
                continue;
            }
            if !in_main {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "Name" => entry.name = Some(value.to_string()),
                "Icon" => entry.icon = Some(value.to_string()),
                "Exec" => entry.exec = exec_program(value),
                _ => {}
            }
        }
        entry
    }

    fn matches(&self, wanted: &str, path: &Path) -> bool {
        let name = self.name.as_deref().unwrap_or("").to_lowercase();
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_lowercase();
        name == wanted
            || name.starts_with(&format!("{wanted} "))
            || stem == wanted
            || stem.starts_with(&format!("{wanted}-"))
            || self
                .exec
                .as_deref()
                .is_some_and(|exec| pretty(exec).to_lowercase() == wanted)
    }
}

/// The program of an `Exec=` line: the first word, unquoted, without its directory.
fn exec_program(exec: &str) -> Option<String> {
    let exec = exec.trim();
    let first = if let Some(rest) = exec.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        exec.split_whitespace().next()?
    };
    let program = first.rsplit('/').next()?;
    (!program.is_empty()).then(|| program.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_this_process() {
        let app = App::of_process(std::process::id()).expect("own process");
        assert!(!app.name.is_empty());
        assert!(parent_pid(std::process::id()).unwrap() > 0);
    }

    #[test]
    fn prettifies_names() {
        assert_eq!(pretty(".claude-wrapped"), "Claude");
        assert_eq!(pretty("pkexec"), "Pkexec");
        assert_eq!(pretty(""), "");
    }

    #[test]
    fn parses_desktop_entries() {
        let entry = DesktopEntry::parse(
            "[Desktop Entry]\nName=Claude Code\nIcon=claude\nExec=\"/nix/store/x/bin/.claude-wrapped\" --handle %u\n[Other]\nName=Nope\n",
        );
        assert_eq!(entry.name.as_deref(), Some("Claude Code"));
        assert_eq!(entry.icon.as_deref(), Some("claude"));
        assert_eq!(entry.exec.as_deref(), Some(".claude-wrapped"));
        assert!(entry.matches("claude", Path::new("/x/whatever.desktop")));
        assert!(entry.matches("other", Path::new("/x/other-thing.desktop")));
        assert!(!entry.matches("firefox", Path::new("/x/whatever.desktop")));
        assert_eq!(exec_program("firefox %u"), Some("firefox".into()));
    }
}
