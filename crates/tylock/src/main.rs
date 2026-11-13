mod app;
mod auth;
mod cli;
mod config;
mod pam;
mod quote;
mod render;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use ty_common::lock::InstanceLock;
use ty_common::screenlock;

use crate::app::App;
use crate::cli::{Cli, Commands};
use crate::config::{AppConfig, APP_NAME};
use crate::render::Renderer;

/// xss-lock's `--transfer-sleep-lock` hands over logind's sleep delay lock in this variable;
/// closing it once the screen is locked lets the machine go to sleep.
const SLEEP_LOCK_FD: &str = "XSS_SLEEP_LOCK_FD";
/// How long locking waits to learn whether you have prints enrolled.
const FINGERPRINT_PROBE: Duration = Duration::from_millis(500);

fn main() {
    let cli = Cli::parse();
    let result = match cli.command.clone().unwrap_or(Commands::Lock) {
        Commands::Lock => lock(cli),
        Commands::ValidateConfig => validate(cli.config),
    };
    if let Err(err) = result {
        eprintln!("tylock: {err:#}");
        std::process::exit(1);
    }
}

fn validate(path: Option<PathBuf>) -> Result<()> {
    let loaded = AppConfig::load(path)?;
    loaded.config.validate()?;
    match loaded.path {
        Some(path) => println!("Config is valid: {}", path.display()),
        None => println!("No config file found; defaults are valid"),
    }
    Ok(())
}

fn lock(cli: Cli) -> Result<()> {
    let config = AppConfig::load(cli.config)?.config;
    config.validate()?;
    if !auth::pam_service_exists(&config.pam_service) {
        bail!(
            "there is no PAM service {:?} (/etc/pam.d/{0}), so no password could unlock \
             the screen; not locking. See the tylock README for the PAM setup.",
            config.pam_service
        );
    }
    // Before any thread or connection exists: the parent only waits for "locked".
    let mut parent = if cli.daemonize {
        Some(daemonize()?)
    } else {
        None
    };
    let sleep_lock = take_sleep_lock();

    let _instance = match InstanceLock::acquire(APP_NAME) {
        Ok(instance) => instance,
        Err(_) => {
            // Locking an already locked screen is done already.
            eprintln!("tylock: the screen is already locked");
            notify_parent(&mut parent);
            return Ok(());
        }
    };
    let user = auth::current_user().context("can't find out who you are (no passwd entry)")?;
    let renderer = Renderer::new(&config)?;
    let (waker, wake_source) = screenlock::waker()?;
    let (events, receiver) = mpsc::channel();
    let submit = auth::spawn_password(
        config.pam_service.clone(),
        user.clone(),
        events.clone(),
        waker.clone(),
    );
    // Known before the first frame, so it shows the fingerprint (or not) from the start.
    let finger = config.fingerprint && auth::probe_enrolled(&user, FINGERPRINT_PROBE);
    if config.fingerprint {
        let service = config.fingerprint_pam_service.clone();
        auth::spawn_fingerprint(service, user, finger, events, waker);
    }

    let on_locked = Box::new(move || {
        drop(sleep_lock);
        notify_parent(&mut parent);
    });
    let mut app = App::new(&config, renderer, receiver, submit, finger, on_locked);
    screenlock::run(&mut app, wake_source)?;
    // The auth threads may be inside PAM; there is nothing to wait for.
    std::process::exit(0);
}

/// Fork. The parent waits until the child reports the screen locked and exits 0 (1 if
/// the child fails first); the child continues with the returned pipe to report on.
fn daemonize() -> Result<OwnedFd> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills both descriptors on success.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error()).context("failed to create a pipe");
    }
    // SAFETY: both descriptors are new and ours.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: the process is single-threaded here.
    match unsafe { libc::fork() } {
        -1 => Err(std::io::Error::last_os_error()).context("fork failed"),
        0 => {
            drop(read);
            // SAFETY: no preconditions; detaches from swayidle's session.
            unsafe { libc::setsid() };
            Ok(write)
        }
        _ => {
            drop(write);
            let mut byte = 0u8;
            // SAFETY: reads at most one byte into a valid buffer.
            let n = unsafe { libc::read(read.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) };
            std::process::exit(if n == 1 { 0 } else { 1 });
        }
    }
}

fn notify_parent(parent: &mut Option<OwnedFd>) {
    if let Some(fd) = parent.take() {
        // SAFETY: writes one byte from a valid buffer to our own pipe.
        unsafe { libc::write(fd.as_raw_fd(), b"1".as_ptr().cast(), 1) };
    }
}

/// The sleep delay lock fd from xss-lock, if any (closed when dropped).
fn take_sleep_lock() -> Option<OwnedFd> {
    let fd: i32 = std::env::var(SLEEP_LOCK_FD).ok()?.trim().parse().ok()?;
    std::env::remove_var(SLEEP_LOCK_FD);
    // SAFETY: fcntl only checks that the descriptor is open; if it is, xss-lock passed it
    // to us and nothing else in this process uses it.
    if fd <= 2 || unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        return None;
    }
    // SAFETY: as above. Not inherited by PAM's helpers, so closing it really releases it.
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}
