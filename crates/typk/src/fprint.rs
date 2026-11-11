//! Asking fprintd whether a user has fingerprints enrolled, so the prompt can show a
//! fingerprint straight away instead of waiting for PAM to get to pam_fprintd.

use std::time::Duration;

use dbus::blocking::Connection;

const SERVICE: &str = "net.reactivated.Fprint";
const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
const MANAGER: &str = "net.reactivated.Fprint.Manager";
const DEVICE: &str = "net.reactivated.Fprint.Device";
/// fprintd is bus-activated, so the first call may start it.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `user` has at least one finger enrolled on the default reader. `false` when
/// there is no fprintd, no reader or no prints.
pub fn enrolled(user: &str) -> bool {
    let Ok(conn) = Connection::new_system() else {
        return false;
    };
    let manager = conn.with_proxy(SERVICE, MANAGER_PATH, TIMEOUT);
    let device: Result<(dbus::Path<'static>,), _> =
        manager.method_call(MANAGER, "GetDefaultDevice", ());
    let Ok((path,)) = device else {
        return false;
    };
    let device = conn.with_proxy(SERVICE, path, TIMEOUT);
    let fingers: Result<(Vec<String>,), _> =
        device.method_call(DEVICE, "ListEnrolledFingers", (user,));
    matches!(fingers, Ok((fingers,)) if !fingers.is_empty())
}
