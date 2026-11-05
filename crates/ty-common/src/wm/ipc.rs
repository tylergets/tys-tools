//! Minimal client for the i3 IPC protocol (https://i3wm.org/docs/ipc.html), spoken by both i3
//! (X11) and sway (Wayland).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::{ActiveWindow, Window, WindowId};
use crate::screen::ScreenRect;
use crate::session;

const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER_LEN: usize = MAGIC.len() + 8;
const RUN_COMMAND: u32 = 0;
const GET_WORKSPACES: u32 = 1;
const GET_OUTPUTS: u32 = 3;
const GET_TREE: u32 = 4;

pub struct IpcConnection {
    stream: UnixStream,
}

#[derive(Debug, Deserialize)]
struct CommandOutcome {
    success: bool,
    #[serde(default)]
    error: Option<String>,
}

impl IpcConnection {
    /// Connect to sway (on Wayland) or i3 (on X11).
    pub fn connect() -> Result<Self> {
        let path = socket_path()?;
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("failed to connect to IPC socket: {}", path.display()))?;
        Ok(Self { stream })
    }

    /// Run a command string (same syntax as `i3-msg`/`swaymsg`), failing if any part fails.
    pub fn run_command(&mut self, command: &str) -> Result<()> {
        let payload = self.request(RUN_COMMAND, command.as_bytes())?;
        let outcomes: Vec<CommandOutcome> =
            serde_json::from_slice(&payload).context("failed to parse IPC command reply")?;
        for outcome in outcomes {
            if !outcome.success {
                bail!(
                    "window manager command failed: {} ({command})",
                    outcome.error.unwrap_or_else(|| "unknown error".into())
                );
            }
        }
        Ok(())
    }

    fn focused_workspace(&mut self) -> Result<Option<String>> {
        let payload = self.request(GET_WORKSPACES, b"")?;
        let workspaces: Vec<Value> =
            serde_json::from_slice(&payload).context("failed to parse IPC workspaces")?;
        Ok(workspaces
            .iter()
            .find(|workspace| workspace["focused"].as_bool() == Some(true))
            .and_then(|workspace| workspace["name"].as_str())
            .map(str::to_string))
    }

    /// The focused output's rectangle (absolute layout coordinates).
    pub fn focused_output(&mut self) -> Result<Option<ScreenRect>> {
        let payload = self.request(GET_OUTPUTS, b"")?;
        let outputs: Vec<Value> =
            serde_json::from_slice(&payload).context("failed to parse IPC outputs")?;
        Ok(outputs
            .iter()
            .find(|output| output["focused"].as_bool() == Some(true))
            .and_then(|output| rect_of(&output["rect"])))
    }

    pub fn tree(&mut self) -> Result<Value> {
        let payload = self.request(GET_TREE, b"")?;
        serde_json::from_slice(&payload).context("failed to parse IPC tree")
    }

    fn request(&mut self, message_type: u32, payload: &[u8]) -> Result<Vec<u8>> {
        self.stream
            .write_all(&encode_message(message_type, payload))?;

        let mut header = [0u8; HEADER_LEN];
        self.stream.read_exact(&mut header)?;
        let (reply_type, len) = decode_header(&header)?;
        if reply_type != message_type {
            bail!("unexpected IPC reply type {reply_type} (expected {message_type})");
        }

        let mut reply = vec![0u8; len];
        self.stream.read_exact(&mut reply)?;
        Ok(reply)
    }
}

/// Whether an i3/sway IPC socket can be found.
pub fn available() -> bool {
    socket_path().is_ok()
}

/// Where a window was before it was staged for recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPlacement {
    pub window: WindowId,
    pub workspace: String,
    /// Workspace that had focus, switched back to on restore.
    pub focused_workspace: Option<String>,
    pub border: String,
    pub border_width: i64,
    /// Floating geometry `(x, y, width, height)`, or `None` if the window was tiled.
    pub floating_rect: Option<(i64, i64, i64, i64)>,
}

/// Move `window` to its own workspace, make it a borderless floating window of the given size,
/// and switch to that workspace. Returns the previous placement for [`restore_window`].
pub fn stage_window_for_recording(
    window: &WindowId,
    width: u32,
    height: u32,
    workspace: &str,
) -> Result<Option<WindowPlacement>> {
    let mut conn = IpcConnection::connect()?;
    let focused = conn.focused_workspace().unwrap_or(None);
    let placement = conn
        .tree()
        .ok()
        .and_then(|tree| find_placement(&tree, window, None, false))
        .map(|placement| WindowPlacement {
            focused_workspace: focused,
            ..placement
        });
    conn.run_command(&stage_window_command(window, width, height, workspace)?)?;
    Ok(placement)
}

/// Put a window back where [`stage_window_for_recording`] found it.
pub fn restore_window(placement: &WindowPlacement) -> Result<()> {
    IpcConnection::connect()?.run_command(&restore_command(placement)?)
}

/// Visible windows (sway), topmost first: fullscreen, then floating, then tiled.
pub fn visible_windows() -> Result<Vec<Window>> {
    let tree = IpcConnection::connect()?.tree()?;
    let mut fullscreen = Vec::new();
    let mut floating = Vec::new();
    let mut tiled = Vec::new();
    collect_views(&tree, false, &mut |node, is_floating| {
        if node["visible"].as_bool() != Some(true) {
            return;
        }
        let (Some(id), Some(rect)) = (node["id"].as_i64(), outer_rect(node)) else {
            return;
        };
        let window = Window {
            id: WindowId::Sway(id),
            rect,
        };
        if node["fullscreen_mode"]
            .as_i64()
            .is_some_and(|mode| mode > 0)
        {
            fullscreen.push(window);
        } else if is_floating {
            floating.push(window);
        } else {
            tiled.push(window);
        }
    });
    // Later floating windows are stacked above earlier ones.
    floating.reverse();
    Ok(fullscreen
        .into_iter()
        .chain(floating)
        .chain(tiled)
        .collect())
}

/// Geometry of a window, from the i3/sway tree.
pub fn window_rect(window: &WindowId) -> Result<ScreenRect> {
    let tree = IpcConnection::connect()?.tree()?;
    let mut found = None;
    collect_views(&tree, false, &mut |node, _| {
        if found.is_none() && matches_window(node, window) {
            found = rect_of(&node["rect"]);
        }
    });
    found.context("window not found")
}

/// The focused window (sway or i3).
pub fn active_window() -> Option<ActiveWindow> {
    let tree = IpcConnection::connect().ok()?.tree().ok()?;
    find_focused(&tree, None)
}

fn find_focused(node: &Value, workspace: Option<&str>) -> Option<ActiveWindow> {
    let workspace = match node["type"].as_str() {
        Some("workspace") => node["name"].as_str(),
        _ => workspace,
    };
    if node["focused"].as_bool() == Some(true) && is_view(node) {
        let properties = &node["window_properties"];
        let app_id = node["app_id"]
            .as_str()
            .or_else(|| properties["class"].as_str())
            .map(str::to_string);
        let x11_window = node["window"].as_u64();
        return Some(ActiveWindow {
            title: node["name"].as_str().unwrap_or_default().to_string(),
            app_id,
            pid: node["pid"].as_i64(),
            window_id: node["id"]
                .as_i64()
                .map(|id| id.to_string())
                .or_else(|| x11_window.map(|id| id.to_string())),
            workspace_name: workspace.map(str::to_string),
            is_xwayland: session::is_wayland()
                .then(|| node["shell"].as_str() == Some("xwayland") || x11_window.is_some()),
        });
    }
    children(node).find_map(|(child, _)| find_focused(child, workspace))
}

/// A leaf container holding an application window.
fn is_view(node: &Value) -> bool {
    matches!(node["type"].as_str(), Some("con" | "floating_con"))
        && node["nodes"].as_array().is_none_or(Vec::is_empty)
        && (node["window"].is_u64() || node["app_id"].is_string() || node["pid"].is_i64())
}

/// Call `visit` with every view in the tree, and whether it is floating.
fn collect_views(node: &Value, in_floating: bool, visit: &mut impl FnMut(&Value, bool)) {
    if is_view(node) {
        visit(node, in_floating);
        return;
    }
    for (child, floating) in children(node) {
        collect_views(child, in_floating || floating, visit);
    }
}

/// Tiled children, then floating ones (with `true`).
fn children(node: &Value) -> impl Iterator<Item = (&Value, bool)> {
    let tiled = node["nodes"].as_array().into_iter().flatten();
    let floating = node["floating_nodes"].as_array().into_iter().flatten();
    tiled
        .map(|child| (child, false))
        .chain(floating.map(|child| (child, true)))
}

/// A view's rectangle including its title bar, which sway's `rect` leaves out (`deco_rect`
/// holds it, just above).
fn outer_rect(node: &Value) -> Option<ScreenRect> {
    let rect = rect_of(&node["rect"])?;
    let title = node["deco_rect"]["height"]
        .as_u64()
        .filter(|_| session::is_wayland())
        .unwrap_or(0) as u32;
    Some(ScreenRect::new(
        rect.x,
        rect.y - title as i32,
        rect.width,
        rect.height + title,
    ))
}

fn rect_of(rect: &Value) -> Option<ScreenRect> {
    let width = rect["width"].as_u64()?;
    let height = rect["height"].as_u64()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(ScreenRect::new(
        rect["x"].as_i64()? as i32,
        rect["y"].as_i64()? as i32,
        width as u32,
        height as u32,
    ))
}

fn matches_window(node: &Value, window: &WindowId) -> bool {
    match window {
        WindowId::X11(id) => node["window"].as_u64() == Some(u64::from(*id)),
        WindowId::Sway(id) => node["id"].as_i64() == Some(*id),
        WindowId::Hyprland(_) => false,
    }
}

/// i3/sway criteria selecting `window`.
fn criteria(window: &WindowId) -> Result<String> {
    match window {
        WindowId::X11(id) => Ok(format!("[id=\"{id}\"]")),
        WindowId::Sway(id) => Ok(format!("[con_id={id}]")),
        WindowId::Hyprland(_) => bail!("not an i3/sway window"),
    }
}

fn restore_command(placement: &WindowPlacement) -> Result<String> {
    let target = criteria(&placement.window)?;
    let border = match placement.border.as_str() {
        "none" => "border none".to_string(),
        style => format!("border {style} {}", placement.border_width),
    };
    let layout = match placement.floating_rect {
        Some((x, y, width, height)) => format!(
            "floating enable, resize set {width} px {height} px, move position {x} px {y} px"
        ),
        None => "floating disable".to_string(),
    };
    let switch_to = placement
        .focused_workspace
        .as_deref()
        .unwrap_or(&placement.workspace);
    Ok(format!(
        "{target} move container to workspace {}; {target} {layout}, {border}; workspace {}",
        quote(&placement.workspace),
        quote(switch_to)
    ))
}

/// Find `window` in an i3/sway tree, noting its workspace, border and floating geometry.
fn find_placement(
    node: &Value,
    window: &WindowId,
    workspace: Option<&str>,
    in_floating: bool,
) -> Option<WindowPlacement> {
    let workspace = match node["type"].as_str() {
        Some("workspace") => node["name"].as_str(),
        _ => workspace,
    };
    if matches_window(node, window) {
        let rect = &node["rect"];
        let floating =
            in_floating || matches!(node["floating"].as_str(), Some("user_on" | "auto_on"));
        return Some(WindowPlacement {
            window: window.clone(),
            workspace: workspace?.to_string(),
            focused_workspace: None,
            border: node["border"].as_str().unwrap_or("pixel").to_string(),
            border_width: node["current_border_width"].as_i64().unwrap_or(2),
            floating_rect: floating.then(|| {
                (
                    rect["x"].as_i64().unwrap_or(0),
                    rect["y"].as_i64().unwrap_or(0),
                    rect["width"].as_i64().unwrap_or(800),
                    rect["height"].as_i64().unwrap_or(600),
                )
            }),
        });
    }

    children(node).find_map(|(child, floating)| {
        find_placement(child, window, workspace, in_floating || floating)
    })
}

fn stage_window_command(
    window: &WindowId,
    width: u32,
    height: u32,
    workspace: &str,
) -> Result<String> {
    let workspace = quote(workspace);
    let target = criteria(window)?;
    Ok(format!(
        "{target} move container to workspace {workspace}; workspace {workspace}; \
{target} floating enable, border none, resize set {width} px {height} px, move position center, focus"
    ))
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn encode_message(message_type: u32, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER_LEN + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    message.extend_from_slice(&message_type.to_ne_bytes());
    message.extend_from_slice(payload);
    message
}

fn decode_header(header: &[u8; HEADER_LEN]) -> Result<(u32, usize)> {
    if &header[..MAGIC.len()] != MAGIC {
        bail!("invalid IPC reply magic");
    }
    let len = u32::from_ne_bytes(header[6..10].try_into()?);
    let reply_type = u32::from_ne_bytes(header[10..14].try_into()?);
    Ok((reply_type, len as usize))
}

/// `$SWAYSOCK` on Wayland; `$I3SOCK` or `i3 --get-socketpath` on X11.
fn socket_path() -> Result<PathBuf> {
    let from_env = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if session::is_wayland() {
        return from_env("SWAYSOCK").context("SWAYSOCK is not set; is sway running?");
    }
    if let Some(path) = from_env("I3SOCK") {
        return Ok(path);
    }

    let output = Command::new("i3")
        .arg("--get-socketpath")
        .output()
        .context("I3SOCK is not set and `i3 --get-socketpath` could not be run")?;
    let path = String::from_utf8(output.stdout)?.trim().to_string();
    if !output.status.success() || path.is_empty() {
        bail!("unable to determine i3 IPC socket path; is i3 running?");
    }
    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_header() {
        let message = encode_message(0, b"nop");
        assert_eq!(&message[..6], b"i3-ipc");
        let header: [u8; HEADER_LEN] = message[..HEADER_LEN].try_into().unwrap();
        assert_eq!(decode_header(&header).unwrap(), (0, 3));
        assert_eq!(&message[HEADER_LEN..], b"nop");
    }

    #[test]
    fn rejects_bad_magic() {
        let header = [0u8; HEADER_LEN];
        assert!(decode_header(&header).is_err());
    }

    #[test]
    fn builds_stage_command_with_quoted_workspace() {
        let command = stage_window_command(&WindowId::X11(42), 1920, 1080, "rec \"1\"").unwrap();
        assert_eq!(
            command,
            "[id=\"42\"] move container to workspace \"rec \\\"1\\\"\"; workspace \"rec \\\"1\\\"\"; \
[id=\"42\"] floating enable, border none, resize set 1920 px 1080 px, move position center, focus"
        );
    }

    #[test]
    fn targets_sway_windows_by_con_id() {
        let command = stage_window_command(&WindowId::Sway(5), 640, 480, "rec").unwrap();
        assert!(command.starts_with("[con_id=5] move container"));
    }

    fn tree() -> Value {
        serde_json::json!({
            "type": "root", "nodes": [{
                "type": "output", "nodes": [{
                    "type": "workspace", "name": "2",
                    "nodes": [{
                        "type": "con", "id": 3, "window": 7, "border": "pixel",
                        "current_border_width": 3, "floating": "auto_off", "visible": true,
                        "focused": true, "name": "Terminal", "pid": 100,
                        "window_properties": {"class": "XTerm"},
                        "rect": {"x": 5, "y": 41, "width": 100, "height": 100}
                    }],
                    "floating_nodes": [{
                        "type": "floating_con", "id": 4, "app_id": "foot", "pid": 101,
                        "floating": "user_on", "visible": true, "name": "foot",
                        "border": "normal", "current_border_width": 2, "nodes": [],
                        "rect": {"x": 10, "y": 20, "width": 300, "height": 200}
                    }]
                }]
            }]
        })
    }

    #[test]
    fn finds_tiled_window() {
        let placement = find_placement(&tree(), &WindowId::X11(7), None, false).unwrap();
        assert_eq!(placement.workspace, "2");
        assert_eq!(placement.border, "pixel");
        assert_eq!(placement.border_width, 3);
        assert_eq!(placement.floating_rect, None);
    }

    #[test]
    fn finds_floating_window() {
        let placement = find_placement(&tree(), &WindowId::Sway(4), None, false).unwrap();
        assert_eq!(placement.floating_rect, Some((10, 20, 300, 200)));
        assert!(find_placement(&tree(), &WindowId::X11(42), None, false).is_none());
    }

    #[test]
    fn builds_restore_command() {
        let placement = WindowPlacement {
            window: WindowId::X11(7),
            workspace: "2".into(),
            focused_workspace: Some("1".into()),
            border: "pixel".into(),
            border_width: 3,
            floating_rect: None,
        };
        assert_eq!(
            restore_command(&placement).unwrap(),
            "[id=\"7\"] move container to workspace \"2\"; \
[id=\"7\"] floating disable, border pixel 3; workspace \"1\""
        );
    }

    #[test]
    fn finds_focused_window() {
        let active = find_focused(&tree(), None).unwrap();
        assert_eq!(active.title, "Terminal");
        assert_eq!(active.app_id.as_deref(), Some("XTerm"));
        assert_eq!(active.pid, Some(100));
        assert_eq!(active.workspace_name.as_deref(), Some("2"));
    }

    #[test]
    fn collects_views_with_floating_flag() {
        let mut seen = Vec::new();
        collect_views(&tree(), false, &mut |node, floating| {
            seen.push((node["id"].as_i64().unwrap(), floating));
        });
        assert_eq!(seen, vec![(3, false), (4, true)]);
    }
}
