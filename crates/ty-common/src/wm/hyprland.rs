//! Hyprland's request socket (what `hyprctl` talks to).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::{Window, WindowId};
use crate::screen::ScreenRect;

/// Send one request (e.g. `j/clients`) and return the reply.
fn request(command: &str) -> Result<String> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("failed to connect to Hyprland: {}", path.display()))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

fn request_json(command: &str) -> Result<Value> {
    let reply = request(&format!("j/{command}"))?;
    serde_json::from_str(&reply).with_context(|| format!("failed to parse Hyprland {command}"))
}

fn socket_path() -> Result<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
    let candidates = [
        crate::runtime_dir().join("hypr"),
        PathBuf::from("/tmp/hypr"),
    ];
    for dir in candidates {
        let path = dir.join(&signature).join(".socket.sock");
        if path.exists() {
            return Ok(path);
        }
    }
    bail!("Hyprland socket not found")
}

/// Visible windows, topmost first: fullscreen, then floating, then tiled, each most recently
/// focused first.
pub fn visible_windows() -> Result<Vec<Window>> {
    let monitors = request_json("monitors")?;
    let clients = request_json("clients")?;
    Ok(visible_from(&monitors, &clients))
}

fn visible_from(monitors: &Value, clients: &Value) -> Vec<Window> {
    let shown: Vec<i64> = monitors
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|monitor| {
            [
                monitor["activeWorkspace"]["id"].as_i64(),
                monitor["specialWorkspace"]["id"]
                    .as_i64()
                    .filter(|&id| id != 0),
            ]
        })
        .flatten()
        .collect();
    let mut windows: Vec<(u8, i64, Window)> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|client| client["mapped"].as_bool() != Some(false))
        .filter(|client| client["hidden"].as_bool() != Some(true))
        .filter(|client| {
            client["workspace"]["id"]
                .as_i64()
                .is_some_and(|id| shown.contains(&id))
        })
        .filter_map(|client| {
            let window = Window {
                id: WindowId::Hyprland(client["address"].as_str()?.to_string()),
                rect: client_rect(client)?,
            };
            let fullscreen = match &client["fullscreen"] {
                Value::Bool(value) => *value,
                value => value.as_i64().unwrap_or(0) > 0,
            };
            let layer = if fullscreen {
                0
            } else if client["floating"].as_bool() == Some(true) {
                1
            } else {
                2
            };
            let recency = client["focusHistoryID"].as_i64().unwrap_or(i64::MAX);
            Some((layer, recency, window))
        })
        .collect();
    windows.sort_by_key(|(layer, recency, _)| (*layer, *recency));
    windows.into_iter().map(|(_, _, window)| window).collect()
}

fn client_rect(client: &Value) -> Option<ScreenRect> {
    let at = client["at"].as_array()?;
    let size = client["size"].as_array()?;
    let (width, height) = (size.first()?.as_u64()?, size.get(1)?.as_u64()?);
    if width == 0 || height == 0 {
        return None;
    }
    Some(ScreenRect::new(
        at.first()?.as_i64()? as i32,
        at.get(1)?.as_i64()? as i32,
        width as u32,
        height as u32,
    ))
}

/// The focused monitor's rectangle in layout coordinates.
pub fn focused_monitor() -> Result<Option<ScreenRect>> {
    let monitors = request_json("monitors")?;
    Ok(monitors
        .as_array()
        .into_iter()
        .flatten()
        .find(|monitor| monitor["focused"].as_bool() == Some(true))
        .and_then(|monitor| {
            // Hyprland reports the mode in physical pixels.
            let scale = monitor["scale"]
                .as_f64()
                .filter(|&s| s > 0.0)
                .unwrap_or(1.0);
            let logical = |key: &str| Some((monitor[key].as_f64()? / scale).round() as u32);
            Some(ScreenRect::new(
                monitor["x"].as_i64()? as i32,
                monitor["y"].as_i64()? as i32,
                logical("width")?,
                logical("height")?,
            ))
        }))
}

/// Geometry of a window.
pub fn window_rect(address: &str) -> Result<ScreenRect> {
    request_json("clients")?
        .as_array()
        .into_iter()
        .flatten()
        .find(|client| client["address"].as_str() == Some(address))
        .and_then(client_rect)
        .context("window not found")
}

/// The pointer position in layout coordinates.
pub fn cursor_position() -> Result<(i32, i32)> {
    let reply = request("cursorpos")?;
    parse_cursor_position(&reply).with_context(|| format!("unexpected cursorpos reply: {reply}"))
}

fn parse_cursor_position(reply: &str) -> Option<(i32, i32)> {
    let (x, y) = reply.trim().split_once(',')?;
    Some((
        x.trim().parse::<f64>().ok()?.round() as i32,
        y.trim().parse::<f64>().ok()?.round() as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_position() {
        assert_eq!(parse_cursor_position("120, -4\n"), Some((120, -4)));
        assert_eq!(parse_cursor_position("ok"), None);
    }

    #[test]
    fn orders_visible_windows() {
        let monitors = serde_json::json!([
            {"activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}}
        ]);
        let clients = serde_json::json!([
            {"address": "0xa", "mapped": true, "hidden": false, "at": [0, 0],
             "size": [800, 600], "workspace": {"id": 1}, "floating": false,
             "fullscreen": 0, "focusHistoryID": 0},
            {"address": "0xb", "mapped": true, "hidden": false, "at": [10, 10],
             "size": [300, 200], "workspace": {"id": 1}, "floating": true,
             "fullscreen": 0, "focusHistoryID": 1},
            {"address": "0xc", "mapped": true, "hidden": false, "at": [10, 10],
             "size": [300, 200], "workspace": {"id": 2}, "floating": true,
             "fullscreen": 0, "focusHistoryID": 2}
        ]);
        let ids: Vec<_> = visible_from(&monitors, &clients)
            .into_iter()
            .map(|window| window.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                WindowId::Hyprland("0xb".into()),
                WindowId::Hyprland("0xa".into())
            ]
        );
    }
}
