//! The agent process: the GTK main loop, with polkitd's calls dispatched straight from the
//! system bus file descriptor (nothing polls), and one window per request.

use std::ffi::CString;
use std::os::unix::io::AsRawFd;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::channel::{BusType, Channel, MatchingReceiver, Sender};
use dbus::message::MatchRule;
use dbus::strings::ErrorName;
use dbus::Message;
use gtk::glib::{self, ControlFlow, IOCondition};
use ty_common::lock::InstanceLock;
use ty_common::session;

use crate::config::{AppConfig, APP_NAME};
use crate::dialog::Dialogs;
use crate::polkit::{self, Bus, Request, AGENT_PATH};

pub fn run(config: AppConfig) -> Result<()> {
    let lock = InstanceLock::acquire(APP_NAME)?;
    session::require(APP_NAME)?;
    gtk::init().context("failed to initialize GTK")?;

    let mut channel =
        Channel::get_private(BusType::System).context("failed to connect to the system bus")?;
    channel.set_watch_enabled(true);
    let fd = channel.watch().fd;
    let conn = Rc::new(Bus::from(channel));
    let subject = polkit::Subject::for_session(&conn)?;
    polkit::register(&conn, &subject)?;
    println!("Registered with polkit for {subject}");

    let dialogs = Dialogs::new(config, conn.clone());

    // `typk verify --reason` announces itself on the instance socket.
    let listener = lock.listener().try_clone()?;
    listener
        .set_nonblocking(true)
        .context("failed to make the instance socket non-blocking")?;
    let announced = dialogs.clone();
    glib::source::unix_fd_add_local(listener.as_raw_fd(), IOCondition::IN, move |_, _| {
        announced.reasons.borrow_mut().accept(&listener);
        ControlFlow::Continue
    });
    conn.start_receive(
        MatchRule::new_method_call().with_path(AGENT_PATH),
        Box::new(move |msg, conn| {
            handle(msg, conn, &dialogs);
            true
        }),
    );

    // Read and dispatch whenever the bus socket has data.
    let bus = conn.clone();
    glib::source::unix_fd_add_local(
        fd,
        IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
        move |_, condition| {
            if condition.intersects(IOCondition::HUP | IOCondition::ERR) {
                eprintln!("error: lost the system bus connection");
                gtk::main_quit();
                return ControlFlow::Break;
            }
            loop {
                match bus.process(Duration::ZERO) {
                    Ok(true) => {}
                    Ok(false) => return ControlFlow::Continue,
                    Err(err) => {
                        eprintln!("error: system bus: {err}");
                        gtk::main_quit();
                        return ControlFlow::Break;
                    }
                }
            }
        },
    );
    for signal in [libc::SIGINT, libc::SIGTERM] {
        glib::source::unix_signal_add_local(signal, || {
            gtk::main_quit();
            ControlFlow::Break
        });
    }

    gtk::main();
    polkit::unregister(&conn, &subject);
    drop(lock);
    Ok(())
}

/// A call from polkitd to our agent object.
fn handle(msg: Message, conn: &Bus, dialogs: &Rc<Dialogs>) {
    let member = msg.member();
    match member.as_deref() {
        Some("BeginAuthentication") => match Request::parse(&msg) {
            Ok(request) => dialogs.begin(request, msg),
            Err(err) => {
                eprintln!("warning: malformed BeginAuthentication: {err:#}");
                let name = ErrorName::new("org.freedesktop.DBus.Error.InvalidArgs")
                    .expect("valid error name");
                let text = CString::new(format!("{err:#}")).unwrap_or_default();
                send(conn, msg.error(&name, &text));
            }
        },
        Some("CancelAuthentication") => {
            match msg.read1::<String>() {
                Ok(cookie) => dialogs.cancel(&cookie),
                Err(err) => eprintln!("warning: malformed CancelAuthentication: {err}"),
            }
            send(conn, msg.method_return());
        }
        _ => {
            if let Some(reply) = dbus::channel::default_reply(&msg) {
                send(conn, reply);
            }
        }
    }
}

fn send(conn: &Bus, msg: Message) {
    if conn.send(msg).is_err() {
        eprintln!("warning: failed to send a reply on the system bus");
    }
    conn.channel().flush();
}
