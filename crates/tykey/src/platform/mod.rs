use std::time::Duration;

#[cfg(target_os = "linux")]
pub mod app_indicator;
pub mod keyboard;

const NOTIFICATION_TIMEOUT: Duration = Duration::from_millis(2000);

/// Fire-and-forget desktop notification from tykey.
pub fn notify(summary: &str, body: &str) {
    ty_common::notify::notify(crate::config::APP_NAME, summary, body, NOTIFICATION_TIMEOUT);
}
