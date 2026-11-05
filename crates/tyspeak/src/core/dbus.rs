//! The D-Bus API on the session bus, served by the running daemon and used by the CLI.
//!
//! Bus name, object path and interface are all `io.github.tylergets.Tyspeak`
//! (path `/io/github/tylergets/Tyspeak`). Owning the bus name also makes the daemon
//! single-instance.

use std::rc::Rc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gtk::gio::{self, prelude::*};
use gtk::glib::{self, thread_guard::ThreadGuard, Variant};

use crate::core::status::Status;

pub const BUS_NAME: &str = "io.github.tylergets.Tyspeak";
pub const OBJECT_PATH: &str = "/io/github/tylergets/Tyspeak";
pub const INTERFACE: &str = "io.github.tylergets.Tyspeak";
const ERROR_NAME: &str = "io.github.tylergets.Tyspeak.Error.Failed";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub const INTROSPECTION: &str = r#"<node>
  <interface name="io.github.tylergets.Tyspeak">
    <!-- Start recording; the text is typed/copied when recording stops. -->
    <method name="Start"/>
    <!-- Stop recording and transcribe. -->
    <method name="Stop"/>
    <!-- Stop if recording, otherwise start. -->
    <method name="Toggle"/>
    <!-- Stop recording and throw the audio away. -->
    <method name="Cancel"/>
    <!-- The daemon status as JSON. -->
    <method name="Status">
      <arg name="status" type="s" direction="out"/>
    </method>
    <!-- The model catalog as JSON: id, name, size, downloaded, selected. -->
    <method name="ListModels">
      <arg name="models" type="s" direction="out"/>
    </method>
    <!-- Select a model by id; it is downloaded if needed, then loaded. -->
    <method name="SetModel">
      <arg name="id" type="s" direction="in"/>
    </method>
    <!-- Download (or load) the selected model again, e.g. after a failed download. -->
    <method name="Retry"/>
    <!-- Emitted whenever the status changes (JSON, as returned by Status). -->
    <signal name="StatusChanged">
      <arg name="status" type="s"/>
    </signal>
    <!-- Emitted with each finished transcription. -->
    <signal name="Transcribed">
      <arg name="text" type="s"/>
    </signal>
  </interface>
</node>"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Start,
    Stop,
    Toggle,
    Cancel,
    Status,
    ListModels,
    SetModel(String),
    Retry,
}

impl Request {
    fn parse(method: &str, parameters: &Variant) -> Option<Self> {
        Some(match method {
            "Start" => Self::Start,
            "Stop" => Self::Stop,
            "Toggle" => Self::Toggle,
            "Cancel" => Self::Cancel,
            "Status" => Self::Status,
            "ListModels" => Self::ListModels,
            "SetModel" => Self::SetModel(parameters.get::<(String,)>()?.0),
            "Retry" => Self::Retry,
            _ => return None,
        })
    }
}

/// What a handled request answers with.
pub enum Reply {
    Empty,
    Text(String),
}

/// The daemon's end of the bus: owns the name, serves the object and emits signals.
pub struct Service {
    connection: gio::DBusConnection,
    _registration: gio::RegistrationId,
}

impl Service {
    /// Claim the bus name and serve requests. `handle` runs on the GTK thread.
    pub fn start(handle: impl Fn(Request) -> Result<Reply> + 'static) -> Result<Self> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .context("failed to connect to the D-Bus session bus")?;
        claim_name(&connection)?;

        let node = gio::DBusNodeInfo::for_xml(INTROSPECTION)?;
        let interface = node
            .lookup_interface(INTERFACE)
            .context("introspection data lacks the interface")?;
        // GDBus invokes the handler on this (the GTK) thread; the guard lets the closure hold
        // the app's non-Send state.
        let handle = MainThreadOnly(ThreadGuard::new(
            Rc::new(handle) as Rc<dyn Fn(Request) -> Result<Reply>>
        ));
        let registration = connection
            .register_object(
                OBJECT_PATH,
                &interface,
                move |_, _, _, _, method, parameters, invocation| {
                    let Some(request) = Request::parse(method, &parameters) else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.UnknownMethod",
                            &format!("unknown method {method}"),
                        );
                        return;
                    };
                    match (handle.get())(request) {
                        Ok(Reply::Empty) => invocation.return_value(None),
                        Ok(Reply::Text(text)) => {
                            invocation.return_value(Some(&(text,).to_variant()))
                        }
                        Err(err) => invocation.return_dbus_error(ERROR_NAME, &format!("{err:#}")),
                    }
                },
                |_, _, _, _, _| unreachable!("the interface has no properties"),
                |_, _, _, _, _, _| false,
            )
            .map_err(|err| anyhow!("failed to register D-Bus object: {err}"))?;
        Ok(Self {
            connection,
            _registration: registration,
        })
    }

    pub fn emit_status(&self, status: &Status) {
        let json = serde_json::to_string(status).unwrap_or_default();
        self.emit("StatusChanged", &json);
    }

    pub fn emit_transcribed(&self, text: &str) {
        self.emit("Transcribed", text);
    }

    fn emit(&self, signal: &str, text: &str) {
        if let Err(err) = self.connection.emit_signal(
            None,
            OBJECT_PATH,
            INTERFACE,
            signal,
            Some(&(text,).to_variant()),
        ) {
            eprintln!("warning: failed to emit {signal}: {err}");
        }
    }
}

/// A value only ever touched on the thread that created it. `ThreadGuard` checks that on
/// every access (panicking elsewhere), which makes sharing references across threads sound.
struct MainThreadOnly<T>(ThreadGuard<T>);

// SAFETY: every access goes through `ThreadGuard::get_ref`, which panics off the owning thread.
unsafe impl<T> Sync for MainThreadOnly<T> {}

impl<T> MainThreadOnly<T> {
    fn get(&self) -> &T {
        self.0.get_ref()
    }
}

/// Request the well-known name without queueing, so a second daemon fails fast.
fn claim_name(connection: &gio::DBusConnection) -> Result<()> {
    const DO_NOT_QUEUE: u32 = 4;
    const PRIMARY_OWNER: u32 = 1;
    const ALREADY_OWNER: u32 = 4;
    let reply = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(BUS_NAME, DO_NOT_QUEUE).to_variant()),
            Some(glib::VariantTy::new("(u)").unwrap()),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT.as_millis() as i32,
            gio::Cancellable::NONE,
        )
        .context("failed to request the D-Bus name")?;
    match reply.get::<(u32,)>().map(|(code,)| code) {
        Some(PRIMARY_OWNER | ALREADY_OWNER) => Ok(()),
        _ => bail!("another tyspeak instance is already running (owns {BUS_NAME})"),
    }
}

// ---- Client ------------------------------------------------------------------------------

fn session_bus() -> Result<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .context("failed to connect to the D-Bus session bus")
}

/// Call `method` on the running daemon. Returns its string reply, if any.
pub fn call(method: &str, args: Option<Variant>) -> Result<Option<String>> {
    call_on(&session_bus()?, method, args)
}

fn call_on(
    connection: &gio::DBusConnection,
    method: &str,
    args: Option<Variant>,
) -> Result<Option<String>> {
    let reply = connection
        .call_sync(
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            method,
            args.as_ref(),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT.as_millis() as i32,
            gio::Cancellable::NONE,
        )
        .map_err(client_error)?;
    Ok(reply.get::<(String,)>().map(|(text,)| text))
}

fn client_error(err: glib::Error) -> anyhow::Error {
    let message = err.message();
    if message.contains("ServiceUnknown") || message.contains("NameHasNoOwner") {
        return anyhow!("tyspeak is not running (nothing owns {BUS_NAME} on the session bus)");
    }
    anyhow!("{}", strip_remote_error(message))
}

/// `GDBus.Error:some.Name: message` → `message`.
fn strip_remote_error(message: &str) -> &str {
    message
        .strip_prefix("GDBus.Error:")
        .and_then(|rest| rest.split_once(": "))
        .map_or(message, |(_, text)| text)
}

pub fn status() -> Result<Status> {
    parse_status(&call("Status", None)?.unwrap_or_default())
}

fn parse_status(json: &str) -> Result<Status> {
    serde_json::from_str(json).context("invalid status from daemon")
}

/// Call `on_status` now and on every change, forever; `Status::offline()` while the daemon is
/// not running.
pub fn follow(on_status: impl Fn(&Status) + Clone + Send + Sync + 'static) -> Result<()> {
    let connection = session_bus()?;
    {
        let on_status = on_status.clone();
        connection.signal_subscribe(
            Some(BUS_NAME),
            Some(INTERFACE),
            Some("StatusChanged"),
            Some(OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                if let Some(status) = parameters
                    .get::<(String,)>()
                    .and_then(|(json,)| parse_status(&json).ok())
                {
                    on_status(&status);
                }
            },
        );
    }
    let appeared = on_status.clone();
    let _watch = gio::bus_watch_name_on_connection(
        &connection,
        BUS_NAME,
        gio::BusNameWatcherFlags::NONE,
        move |connection, _, _| {
            let status = call_on(&connection, "Status", None)
                .ok()
                .flatten()
                .and_then(|json| parse_status(&json).ok())
                .unwrap_or_else(Status::offline);
            appeared(&status);
        },
        move |_, _| on_status(&Status::offline()),
    );
    glib::MainLoop::new(None, false).run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{strip_remote_error, Request, INTERFACE, INTROSPECTION};
    use gtk::gio;
    use gtk::glib::prelude::*;

    #[test]
    fn introspection_parses() {
        let node = gio::DBusNodeInfo::for_xml(INTROSPECTION).unwrap();
        assert!(node.lookup_interface(INTERFACE).is_some());
    }

    #[test]
    fn parses_requests() {
        let none = ().to_variant();
        assert_eq!(Request::parse("Toggle", &none), Some(Request::Toggle));
        assert_eq!(
            Request::parse("SetModel", &("parakeet-v2",).to_variant()),
            Some(Request::SetModel("parakeet-v2".into()))
        );
        assert_eq!(Request::parse("SetModel", &none), None);
        assert_eq!(Request::parse("Retry", &none), Some(Request::Retry));
        assert_eq!(Request::parse("Explode", &none), None);
    }

    #[test]
    fn strips_remote_error_names() {
        assert_eq!(
            strip_remote_error("GDBus.Error:io.github.tylergets.Tyspeak.Error.Failed: no model"),
            "no model"
        );
        assert_eq!(strip_remote_error("plain"), "plain");
    }
}
