use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use glib::variant::ToVariant;
use gtk::{gio, glib};

use crate::config::{Markup, MouseAction};
use crate::model::{Colors, Notification, RawImage, State};

pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const CONTROL: &str = "tools.tys.tynotif";
pub type Shared = Arc<Mutex<State>>;

const XML: &str = r#"<node>
<interface name="org.freedesktop.Notifications">
 <method name="GetCapabilities"><arg type="as" direction="out"/></method>
 <method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
 <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
 <method name="CloseNotification"><arg type="u" direction="in"/></method>
 <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
 <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
 <signal name="NotificationReplied"><arg type="u"/><arg type="s"/></signal>
</interface>
<interface name="tools.tys.tynotif">
 <method name="Command"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
</interface>
</node>"#;

pub fn connect() -> Result<gio::DBusConnection> {
    Ok(gio::bus_get_sync(
        gio::BusType::Session,
        gio::Cancellable::NONE,
    )?)
}

pub fn serve(state: Shared, replace: bool, force: bool) -> Result<gio::DBusConnection> {
    let connection = connect()?;
    // Subscribe before requesting the name so a rapid subsequent takeover isn't missed.
    // GIO dispatches this on the registering thread's main context (the GTK thread).
    connection.signal_subscribe(
        Some("org.freedesktop.DBus"),
        Some("org.freedesktop.DBus"),
        Some("NameLost"),
        Some("/org/freedesktop/DBus"),
        Some(NAME),
        gio::DBusSignalFlags::NONE,
        |_, _, _, _, _, _| {
            eprintln!("tynotif was replaced by another notification daemon; exiting");
            gtk::main_quit();
        },
    );
    // ALLOW_REPLACEMENT | DO_NOT_QUEUE, optionally REPLACE_EXISTING. Never leave
    // a hidden daemon queued to unexpectedly acquire the name later.
    let flags = 1u32 | 4u32 | if replace || force { 2u32 } else { 0u32 };
    let request = || {
        connection.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(NAME, flags).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
    };
    let mut reply = request()?;
    if reply.get::<(u32,)>() != Some((1,)) && force {
        stop_dunst_owner(&connection)?;
        reply = request()?;
    }
    if reply.get::<(u32,)>() != Some((1,)) {
        if replace || force {
            bail!("the current owner of {NAME} does not allow replacement; stop it first (for Dunst: systemctl --user stop dunst.service), then rerun tynotif");
        }
        bail!("another notification daemon owns {NAME}; use tynotif run --force for Dunst, --replace for a replaceable daemon, or stop it first");
    }
    let node = gio::DBusNodeInfo::for_xml(XML)?;
    for interface in [NAME, CONTROL] {
        let state = state.clone();
        connection.register_object(
            PATH,
            &node
                .lookup_interface(interface)
                .context("missing interface")?,
            move |connection, sender, _, interface, method, parameters, invocation| {
                let mut state = state.lock().unwrap();
                let result = if interface == CONTROL {
                    parameters
                        .get::<(String, String)>()
                        .ok_or_else(|| "invalid command arguments".to_string())
                        .and_then(|(command, value)| {
                            command_call(&connection, &mut state, &command, &value)
                                .map(|s| (s,).to_variant())
                        })
                } else {
                    notification_call(&connection, &mut state, sender, method, &parameters)
                };
                match result {
                    Ok(reply) => invocation.return_value(Some(&reply)),
                    Err(error) => invocation
                        .return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", &error),
                }
            },
            |_, _, _, _, _| ().to_variant(),
            |_, _, _, _, _, _| false,
        )?;
    }
    Ok(connection)
}

fn stop_dunst_owner(connection: &gio::DBusConnection) -> Result<()> {
    let owner = connection.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        Some(&(NAME,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )?;
    let (owner,) = owner
        .get::<(String,)>()
        .context("invalid name owner reply")?;
    let pid = connection.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        Some(&(owner,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    )?;
    let (pid,) = pid.get::<(u32,)>().context("invalid owner PID reply")?;
    let unit = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "dunst.service",
            "--property=MainPID",
            "--value",
        ])
        .output()
        .context("--force needs systemctl to stop the Dunst user service")?;
    let service_pid = String::from_utf8_lossy(&unit.stdout)
        .trim()
        .parse::<u32>()
        .ok();
    if !unit.status.success() || service_pid != Some(pid) || pid == 0 {
        bail!("the notification owner is not the running dunst.service; stop your notification daemon manually, then rerun tynotif");
    }
    eprintln!("Stopping dunst.service to take over notifications");
    let status = Command::new("systemctl")
        .args(["--user", "stop", "dunst.service"])
        .status()
        .context("failed to stop dunst.service")?;
    if !status.success() {
        bail!("could not stop dunst.service ({status})");
    }
    Ok(())
}

type NotifyArgs = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, glib::Variant>,
    i32,
);

fn notification_call(
    connection: &gio::DBusConnection,
    state: &mut State,
    sender: &str,
    method: &str,
    parameters: &glib::Variant,
) -> Result<glib::Variant, String> {
    match method {
        "GetCapabilities" => {
            let mut caps = vec![
                "body",
                "actions",
                "persistence",
                "inline-reply",
                "icon-static",
            ];
            if state.config.markup == Markup::Full {
                caps.push("body-markup");
            }
            Ok((caps,).to_variant())
        }
        "GetServerInformation" => {
            Ok(("tynotif", "tys-tools", env!("CARGO_PKG_VERSION"), "1.2").to_variant())
        }
        "CloseNotification" => {
            let (id,) = parameters
                .get::<(u32,)>()
                .ok_or("invalid notification id")?;
            close(connection, state, id, 3);
            Ok(().to_variant())
        }
        "Notify" => {
            let (app, replaces, icon, summary, body, actions, hints, timeout) = parameters
                .get::<NotifyArgs>()
                .ok_or("invalid Notify arguments")?;
            if actions.len() % 2 != 0
                || actions.len() > 64
                || [app.len(), icon.len(), summary.len(), body.len()]
                    .iter()
                    .any(|n| *n > 65536)
            {
                return Err("notification text or actions exceed limits".into());
            }
            let urgency = hints
                .get("urgency")
                .and_then(|v| v.get::<u8>())
                .unwrap_or(1)
                .min(2);
            let duration = if timeout < 0 {
                match urgency {
                    0 => state.config.low_timeout_ms,
                    2 => 0,
                    _ => state.config.timeout_ms,
                }
            } else {
                timeout as u64
            };
            let string_hint = |name| {
                hints
                    .get(name)
                    .and_then(|v| v.get::<String>())
                    .unwrap_or_default()
            };
            let bool_hint = |name| {
                hints
                    .get(name)
                    .and_then(|v| v.get::<bool>().or_else(|| integer_hint(v).map(|v| v != 0)))
                    .unwrap_or(false)
            };
            let tag = [
                "x-dunst-stack-tag",
                "x-canonical-private-synchronous",
                "synchronous",
                "private-synchronous",
            ]
            .iter()
            .map(|name| string_hint(name))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
            let image = ["image-path", "image_path"]
                .iter()
                .map(|name| string_hint(name))
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            let raw = ["image-data", "image_data"]
                .iter()
                .find_map(|key| hints.get(*key).and_then(raw_image))
                .or_else(|| {
                    if image.is_empty() && icon.is_empty() {
                        hints.get("icon_data").and_then(raw_image)
                    } else {
                        None
                    }
                });
            let highlight = hints
                .get("hlcolor")
                .map(|v| {
                    v.get::<Vec<String>>()
                        .unwrap_or_else(|| {
                            string_hint("hlcolor")
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .collect()
                        })
                        .iter()
                        .filter_map(|s| crate::content::color(s))
                        .collect()
                })
                .unwrap_or_default();
            let notification = Notification {
                generation: 0,
                category: string_hint("category"),
                desktop_entry: string_hint("desktop-entry"),
                image: raw,
                colors: Colors {
                    foreground: crate::content::color(&string_hint("fgcolor")),
                    background: crate::content::color(&string_hint("bgcolor")),
                    frame: crate::content::color(&string_hint("frcolor")),
                    highlight,
                },
                id: 0,
                app,
                summary,
                body,
                icon: if image.is_empty() { icon } else { image },
                urgency,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                actions,
                reply_placeholder: string_hint("x-kde-reply-placeholder-text"),
                reply_submit_text: string_hint("x-kde-reply-submit-button-text"),
                reply_submit_icon: string_hint("x-kde-reply-submit-button-icon-name"),
                progress: hints
                    .get("value")
                    .and_then(integer_hint)
                    .map(|v| v.clamp(0, 100) as i32),
                sender: sender.into(),
                tag,
                resident: bool_hint("resident"),
                transient: bool_hint("transient"),
                remaining: (duration != 0).then_some(duration),
            };
            state
                .notify(notification, replaces)
                .map(|id| (id,).to_variant())
                .map_err(str::to_string)
        }
        _ => Err("unknown method".into()),
    }
}

pub fn closed(connection: &gio::DBusConnection, notification: &Notification, reason: u32) {
    if !notification.sender.is_empty() {
        if let Err(error) = connection.emit_signal(
            Some(&notification.sender),
            PATH,
            NAME,
            "NotificationClosed",
            Some(&(notification.id, reason).to_variant()),
        ) {
            eprintln!("NotificationClosed: {error}");
        }
    }
}

pub fn close(connection: &gio::DBusConnection, state: &mut State, id: u32, reason: u32) {
    if let Some(notification) = state.close(id) {
        closed(connection, &notification, reason);
    }
}

pub fn close_all(connection: &gio::DBusConnection, state: &mut State) {
    let ids: Vec<_> = state.active.iter().map(|n| n.id).collect();
    for id in ids {
        close(connection, state, id, 2);
    }
}

pub fn action(
    connection: &gio::DBusConnection,
    state: &mut State,
    id: u32,
    key: &str,
) -> Result<(), String> {
    if key == "inline-reply" {
        return Err("inline replies require text; use the notification's reply field".into());
    }
    let notification = state
        .active
        .iter()
        .find(|n| n.id == id)
        .ok_or("notification not active")?;
    if !notification
        .actions
        .chunks_exact(2)
        .any(|pair| pair[0] == key)
        || notification.sender.is_empty()
    {
        return Err("action not available".into());
    }
    connection
        .emit_signal(
            Some(&notification.sender),
            PATH,
            NAME,
            "ActionInvoked",
            Some(&(id, key).to_variant()),
        )
        .map_err(|e| e.to_string())?;
    if !notification.resident {
        close(connection, state, id, 2);
    }
    Ok(())
}

pub fn reply(
    connection: &gio::DBusConnection,
    state: &mut State,
    id: u32,
    text: &str,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("reply cannot be empty".into());
    }
    let notification = state
        .active
        .iter()
        .find(|n| n.id == id)
        .ok_or("notification not active")?;
    if notification.sender.is_empty()
        || !notification
            .actions
            .chunks_exact(2)
            .any(|pair| pair[0] == "inline-reply")
    {
        return Err("inline reply not available".into());
    }
    connection
        .emit_signal(
            Some(&notification.sender),
            PATH,
            NAME,
            "NotificationReplied",
            Some(&(id, text).to_variant()),
        )
        .map_err(|e| e.to_string())?;
    if !notification.resident {
        close(connection, state, id, 2);
    }
    Ok(())
}

fn command_call(
    connection: &gio::DBusConnection,
    state: &mut State,
    command: &str,
    value: &str,
) -> Result<String, String> {
    match command {
        "history" => return serde_json::to_string(&state.history).map_err(|e| e.to_string()),
        "list" => return serde_json::to_string(&state.active).map_err(|e| e.to_string()),
        "status" => return Ok(serde_json::json!({"paused": state.paused, "snooze_remaining_seconds": state.snooze_remaining().map(|d| d.as_secs()), "active": state.active.len(), "history": state.history.len()}).to_string()),
        "is-paused" => return Ok(state.paused.to_string()),
        "set-paused" => {
            let paused = match value { "true" => true, "false" => false, "toggle" => !state.paused, _ => return Err("expected true, false or toggle".into()) };
            state.set_paused(paused);
        }
        "snooze" => {
            let minutes: u64 = value.parse().map_err(|_| "expected minutes between 1 and 1440")?;
            if !(1..=1440).contains(&minutes) { return Err("expected minutes between 1 and 1440".into()); }
            state.snooze(std::time::Duration::from_secs(minutes * 60));
        }
        "close" => {
            let id = if value.is_empty() { state.visible().first().map(|n| n.id).unwrap_or(0) } else { value.parse().map_err(|_| "invalid id")? };
            close(connection, state, id, 2);
        }
        "close-all" => close_all(connection, state),
        "history-clear" => state.clear_history(),
        "history-pop" => {
            let id = if value.is_empty() { 0 } else { value.parse().map_err(|_| "invalid id")? };
            return state.replay(id).map(|id| id.to_string()).map_err(str::to_string);
        }
        "action" => {
            if let Some((id, key)) = value.split_once(':') {
                action(connection, state, id.parse().map_err(|_| "invalid id")?, key)?;
            } else {
                enqueue(state, value, MouseAction::DoAction)?;
            }
        }
        "context" => enqueue(state, value, if value.is_empty() { MouseAction::ContextAll } else { MouseAction::Context })?,
        "open-url" => enqueue(state, value, MouseAction::OpenUrl)?,
        _ => return Err("unknown command".into()),
    }
    Ok(String::new())
}

fn enqueue(state: &mut State, value: &str, action: MouseAction) -> Result<(), String> {
    let id = if value.is_empty() {
        None
    } else {
        let id = value.parse().map_err(|_| "invalid id")?;
        if !state.active.iter().any(|n| n.id == id) {
            return Err("notification not active".into());
        }
        Some(id)
    };
    if state.interactions.len() >= 64 {
        return Err("interaction queue is full".into());
    }
    state.interactions.push_back((id, vec![action]));
    Ok(())
}

fn integer_hint(v: &glib::Variant) -> Option<i64> {
    v.get::<i32>()
        .map(i64::from)
        .or_else(|| v.get::<u32>().map(i64::from))
        .or_else(|| v.get::<u8>().map(i64::from))
}

fn raw_image(v: &glib::Variant) -> Option<RawImage> {
    let (width, height, stride, alpha, bits, channels, pixels) =
        v.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
    // Validate before constructing a pixbuf: the final row need not include padding.
    if width <= 0
        || height <= 0
        || width > 4096
        || height > 4096
        || bits != 8
        || channels != if alpha { 4 } else { 3 }
    {
        return None;
    }
    let row = width.checked_mul(channels)?;
    if stride < row {
        return None;
    }
    let required = (height as usize - 1)
        .checked_mul(stride as usize)?
        .checked_add(row as usize)?;
    if pixels.len() < required || pixels.len() > 16 * 1024 * 1024 {
        return None;
    }
    Some(RawImage {
        width,
        height,
        stride,
        alpha,
        pixels: Arc::new(pixels),
    })
}

#[cfg(test)]
mod hint_tests {
    use super::*;
    #[test]
    fn raw_images_require_consistent_dimensions_and_backing_memory() {
        assert!(
            raw_image(&(2i32, 2i32, 8i32, true, 8i32, 4i32, vec![0u8; 16]).to_variant()).is_some()
        );
        assert!(
            raw_image(&(2i32, 2i32, 8i32, true, 8i32, 4i32, vec![0u8; 15]).to_variant()).is_none()
        );
        assert!(
            raw_image(&(2i32, 2i32, 4i32, true, 8i32, 4i32, vec![0u8; 16]).to_variant()).is_none()
        );
        assert!(
            raw_image(&(i32::MAX, 2i32, 8i32, true, 8i32, 4i32, vec![0u8; 16]).to_variant())
                .is_none()
        );
        assert!(
            raw_image(&(1i32, 2i32, 4i32, false, 8i32, 3i32, vec![0u8; 7]).to_variant()).is_some()
        );
    }
    #[test]
    fn progress_accepts_unsigned_hints() {
        assert_eq!(integer_hint(&100u32.to_variant()), Some(100));
    }
}

pub fn command(command: &str, value: &str) -> Result<()> {
    let reply = connect()?
        .call_sync(
            Some(NAME),
            PATH,
            CONTROL,
            "Command",
            Some(&(command, value).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
            gio::Cancellable::NONE,
        )
        .context("cannot reach tynotif; is it running?")?;
    let (output,) = reply.get::<(String,)>().context("invalid command reply")?;
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}
