//! Desktop notifications through `notify-send` (libnotify).

use std::process::Command;
use std::time::Duration;

/// Fire-and-forget notification from `app`. Failures are logged, never fatal.
pub fn notify(app: &str, summary: &str, body: &str, timeout: Duration) {
    notify_with_icon(app, summary, body, timeout, None);
}

/// Like [`notify`], with an icon (name or file path).
pub fn notify_with_icon(
    app: &str,
    summary: &str,
    body: &str,
    timeout: Duration,
    icon: Option<&str>,
) {
    let mut command = Command::new("notify-send");
    command.args(["-a", app, "-t", &timeout.as_millis().to_string()]);
    if let Some(icon) = icon {
        command.args(["-i", icon]);
    }
    command.args([summary, body]);
    if let Err(err) = command.spawn() {
        eprintln!("warning: failed to run notify-send: {err}");
    }
}

/// Arguments for a notification with action buttons. `notify-send` blocks until the
/// notification closes and prints the chosen action key.
pub fn action_args(
    app: &str,
    summary: &str,
    body: &str,
    timeout: Duration,
    actions: &[(&str, &str)],
) -> Vec<String> {
    let mut args = vec![
        "-a".to_string(),
        app.to_string(),
        "-t".to_string(),
        timeout.as_millis().to_string(),
    ];
    args.extend(
        actions
            .iter()
            .map(|(key, label)| format!("--action={key}={label}")),
    );
    args.extend([summary.to_string(), body.to_string()]);
    args
}

/// Send a notification with actions and block until it closes. Returns the chosen action
/// key (`"default"` for a body click). Falls back to a plain notification when
/// `notify-send` predates `--action` (libnotify < 0.7.10).
pub fn notify_with_actions_blocking(
    app: &str,
    summary: &str,
    body: &str,
    timeout: Duration,
    actions: &[(&str, &str)],
) -> Option<String> {
    let args = action_args(app, summary, body, timeout, actions);
    match Command::new("notify-send").args(&args).output() {
        Ok(output) if output.status.success() => {
            let key = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!key.is_empty()).then_some(key)
        }
        Ok(_) => {
            notify(app, summary, body, timeout);
            None
        }
        Err(err) => {
            eprintln!("warning: failed to send notification: {err}");
            None
        }
    }
}
