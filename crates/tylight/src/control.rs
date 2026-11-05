//! The daemon's control socket (the single-instance lock socket) and its client.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::APP_NAME;
use crate::daemon::{Msg, Shared};
use crate::protocol::{Command, Status};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Answer requests on `listener` from a background thread.
pub fn serve(listener: UnixListener, tx: Sender<Msg>, shared: Shared) {
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, shared) = (tx.clone(), shared.clone());
            // DDC brightness changes are slow, so each client gets its own thread.
            thread::spawn(move || handle(stream, &tx, &shared));
        }
    });
}

fn handle(mut stream: UnixStream, tx: &Sender<Msg>, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() || line.trim().is_empty() {
        // e.g. the single-instance probe.
        return;
    }
    let reply = Command::parse(line.trim())
        .and_then(|command| run(command, tx, shared))
        .map_err(|err| format!("{err:#}"));
    let text = match reply {
        Ok(status) => serde_json::to_string(&status).unwrap_or_default(),
        Err(err) => format!("error: {err}"),
    };
    let _ = writeln!(stream, "{text}");
}

fn run(command: Command, tx: &Sender<Msg>, shared: &Shared) -> Result<Status> {
    match command {
        Command::Brightness(change) => return shared.change_brightness(change),
        Command::AutoBrightness(on) => return shared.set_auto_brightness(on),
        Command::ShowGraph => return shared.request_graph(),
        _ => {}
    }
    let (reply_tx, reply_rx) = mpsc::channel();
    tx.send(Msg::Command(command, Some(reply_tx)))
        .ok()
        .context("daemon is shutting down")?;
    reply_rx
        .recv_timeout(TIMEOUT)
        .context("daemon did not answer")?
        .map_err(anyhow::Error::msg)
}

/// Send `command` to the running daemon.
pub fn request(command: &Command) -> Result<Status> {
    let path = ty_common::lock::socket_path(APP_NAME);
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "{APP_NAME} is not running (no socket at {})",
            path.display()
        )
    })?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    writeln!(stream, "{}", command.to_line())?;
    stream.shutdown(Shutdown::Write)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    let reply = reply.trim();
    if let Some(err) = reply.strip_prefix("error: ") {
        bail!("{err}");
    }
    serde_json::from_str(reply).with_context(|| format!("unexpected reply: {reply}"))
}
