use std::process::Command;

use serde_json::Value;

use crate::config::ActiveWindowBackend;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveWindowContext {
    pub backend: String,
    pub title: String,
    pub app_id: Option<String>,
    pub initial_app_id: Option<String>,
    pub initial_title: Option<String>,
    pub window_id: Option<String>,
    pub pid: Option<i64>,
    pub workspace_id: Option<i64>,
    pub workspace_name: Option<String>,
    pub is_xwayland: Option<bool>,
}

pub trait ActiveWindowProvider: Send + Sync {
    fn capture(&self) -> Option<ActiveWindowContext>;
}

pub struct DisabledActiveWindowProvider;

impl ActiveWindowProvider for DisabledActiveWindowProvider {
    fn capture(&self) -> Option<ActiveWindowContext> {
        None
    }
}

pub struct CommandActiveWindowProvider {
    program: String,
    args: Vec<String>,
    parser: fn(&str) -> Option<ActiveWindowContext>,
}

impl CommandActiveWindowProvider {
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        parser: fn(&str) -> Option<ActiveWindowContext>,
    ) -> Self {
        Self {
            program: program.into(),
            args,
            parser,
        }
    }
}

impl ActiveWindowProvider for CommandActiveWindowProvider {
    fn capture(&self) -> Option<ActiveWindowContext> {
        let output = Command::new(&self.program).args(&self.args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let raw = String::from_utf8(output.stdout).ok()?;
        (self.parser)(raw.trim())
    }
}

/// The focused window from sway's (or i3's) IPC socket.
pub struct IpcActiveWindowProvider;

impl ActiveWindowProvider for IpcActiveWindowProvider {
    fn capture(&self) -> Option<ActiveWindowContext> {
        let window = ty_common::wm::active_window()?;
        if window.title.trim().is_empty() {
            return None;
        }
        Some(ActiveWindowContext {
            backend: "ipc".to_string(),
            title: window.title,
            app_id: window.app_id,
            initial_app_id: None,
            initial_title: None,
            window_id: window.window_id,
            pid: window.pid,
            workspace_id: None,
            workspace_name: window.workspace_name,
            is_xwayland: window.is_xwayland,
        })
    }
}

pub struct AutoActiveWindowProvider {
    providers: Vec<Box<dyn ActiveWindowProvider>>,
}

impl AutoActiveWindowProvider {
    pub fn new() -> Self {
        let mut providers: Vec<Box<dyn ActiveWindowProvider>> =
            vec![Box::new(CommandActiveWindowProvider::new(
                "hyprctl",
                vec!["activewindow".into(), "-j".into()],
                parse_hyprctl_active_window,
            ))];
        // On Wayland xdotool only sees XWayland windows, so ask sway first.
        if ty_common::session::is_wayland() {
            providers.push(Box::new(IpcActiveWindowProvider));
        }
        providers.push(Box::new(CommandActiveWindowProvider::new(
            "sh",
            vec![
                "-c".into(),
                "window_id=$(xdotool getactivewindow 2>/dev/null) || exit 1; \
title=$(xdotool getwindowname \"$window_id\" 2>/dev/null || true); \
app_id=$(xdotool getwindowclassname \"$window_id\" 2>/dev/null || true); \
pid=$(xdotool getwindowpid \"$window_id\" 2>/dev/null || true); \
workspace_id=$(xdotool get_desktop_for_window \"$window_id\" 2>/dev/null || true); \
printf 'window_id=%s\\n' \"$window_id\"; \
printf 'title=%s\\n' \"$title\"; \
printf 'app_id=%s\\n' \"$app_id\"; \
printf 'pid=%s\\n' \"$pid\"; \
printf 'workspace_id=%s\\n' \"$workspace_id\";"
                    .into(),
            ],
            parse_xdotool_active_window,
        )));
        Self { providers }
    }
}

impl Default for AutoActiveWindowProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveWindowProvider for AutoActiveWindowProvider {
    fn capture(&self) -> Option<ActiveWindowContext> {
        for provider in &self.providers {
            if let Some(context) = provider.capture() {
                return Some(context);
            }
        }
        None
    }
}

pub fn provider_from_config(config: &ActiveWindowBackend) -> Box<dyn ActiveWindowProvider> {
    match config {
        ActiveWindowBackend::Disabled => Box::new(DisabledActiveWindowProvider),
        ActiveWindowBackend::Command { program, args } => {
            Box::new(CommandActiveWindowProvider::new(
                program.clone(),
                args.clone(),
                parse_command_active_window,
            ))
        }
        ActiveWindowBackend::Auto => Box::new(AutoActiveWindowProvider::new()),
    }
}

fn parse_hyprctl_active_window(raw: &str) -> Option<ActiveWindowContext> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let title = parsed.get("title")?.as_str()?.trim();
    if title.is_empty() {
        return None;
    }

    let app_id = optional_trimmed_string(parsed.get("class"));
    let initial_app_id = optional_trimmed_string(parsed.get("initialClass"));
    let initial_title = optional_trimmed_string(parsed.get("initialTitle"));
    let window_id = optional_trimmed_string(parsed.get("address"));
    let pid = parsed.get("pid").and_then(|value| value.as_i64());

    let workspace_id = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("id"))
        .and_then(|value| value.as_i64());
    let workspace_name = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("name"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let is_xwayland = parsed.get("xwayland").and_then(|value| value.as_bool());

    Some(ActiveWindowContext {
        backend: "hyprctl".to_string(),
        title: title.to_string(),
        app_id,
        initial_app_id,
        initial_title,
        window_id,
        pid,
        workspace_id,
        workspace_name,
        is_xwayland,
    })
}

fn parse_xdotool_active_window(raw: &str) -> Option<ActiveWindowContext> {
    let mut title: Option<String> = None;
    let mut app_id: Option<String> = None;
    let mut window_id: Option<String> = None;
    let mut pid: Option<i64> = None;
    let mut workspace_id: Option<i64> = None;

    for line in raw.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "title" => {
                if !value.is_empty() {
                    title = Some(value.to_string());
                }
            }
            "app_id" => {
                if !value.is_empty() {
                    app_id = Some(value.to_string());
                }
            }
            "window_id" => {
                if !value.is_empty() {
                    window_id = Some(value.to_string());
                }
            }
            "pid" => {
                pid = value.parse::<i64>().ok();
            }
            "workspace_id" => {
                workspace_id = value.parse::<i64>().ok();
            }
            _ => {}
        }
    }

    let title = title?;
    Some(ActiveWindowContext {
        backend: "xdotool".to_string(),
        title,
        app_id,
        initial_app_id: None,
        initial_title: None,
        window_id,
        pid,
        workspace_id,
        workspace_name: None,
        is_xwayland: None,
    })
}

fn parse_command_active_window(raw: &str) -> Option<ActiveWindowContext> {
    parse_title_with_backend("command", raw)
}

fn parse_title_with_backend(backend: &str, raw: &str) -> Option<ActiveWindowContext> {
    let title = raw.trim();
    if title.is_empty() {
        return None;
    }
    Some(ActiveWindowContext {
        backend: backend.to_string(),
        title: title.to_string(),
        app_id: None,
        initial_app_id: None,
        initial_title: None,
        window_id: None,
        pid: None,
        workspace_id: None,
        workspace_name: None,
        is_xwayland: None,
    })
}

fn optional_trimmed_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
