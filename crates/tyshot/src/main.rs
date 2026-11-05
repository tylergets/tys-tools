mod cli;

use std::cell::RefCell;
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::glib;
use ty_common::clipboard;
use ty_common::screen::ScreenRect;
use tyshot::config::AppConfig;
use tyshot::overlay::{self, Action, Capture};
use tyshot::{opencode, output};

use crate::cli::{Cli, Commands, FullArgs, GuiArgs, PinArgs};

/// Exit status when the user cancels the selection.
const CANCELLED: u8 = 1;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command.unwrap_or(Commands::Gui(GuiArgs::default())) {
        Commands::Gui(args) => gui(cli.config, args),
        Commands::Full(args) => full(cli.config, args),
        Commands::Pin(args) => pin_file(cli.config, args),
        Commands::Tray => {
            let config = load_config(cli.config.clone())?;
            tyshot::tray::run(config, cli.config).map(|()| ExitCode::SUCCESS)
        }
        Commands::Sessions => list_sessions(),
        Commands::ValidateConfig => validate_config(cli.config),
        Commands::ServeClipboard => clipboard::serve().map(|()| ExitCode::SUCCESS),
    }
}

fn load_config(path: Option<std::path::PathBuf>) -> Result<AppConfig> {
    let loaded = AppConfig::load(path)?;
    loaded.config.validate()?;
    Ok(loaded.config)
}

fn gui(config_path: Option<std::path::PathBuf>, args: GuiArgs) -> Result<ExitCode> {
    let config = load_config(config_path)?;
    ty_common::tray::init_gtk()?;
    let snapshot = take_snapshot(args.delay)?;
    let windows = visible_windows();

    let result: Rc<RefCell<Option<Capture>>> = Rc::default();
    {
        let result = result.clone();
        overlay::select(snapshot, windows, &config, move |capture| {
            *result.borrow_mut() = capture;
            gtk::main_quit();
        })?;
    }
    gtk::main();

    let Some(capture) = result.take() else {
        return Ok(ExitCode::from(CANCELLED));
    };
    if args.raw {
        write_raw(&capture.image)?;
    } else if capture.action == Action::Pin {
        pin(config, capture);
    } else {
        deliver(&config, capture)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn pin_file(config_path: Option<std::path::PathBuf>, args: PinArgs) -> Result<ExitCode> {
    let config = load_config(config_path)?;
    ty_common::tray::init_gtk()?;
    let image = Pixbuf::from_file(&args.file)
        .with_context(|| format!("failed to read {}", args.file.display()))?;
    // A file's pixels are taken as device pixels, like a capture's.
    let scale = ty_common::screen::logical_scale();
    let origin = match args.at.as_deref() {
        Some(&[x, y]) => (x, y),
        _ => {
            let bounds = ty_common::screen::bounds()?;
            (
                ((f64::from(bounds.width) - f64::from(image.width())) / 2.0 / scale).max(0.0),
                ((f64::from(bounds.height) - f64::from(image.height())) / 2.0 / scale).max(0.0),
            )
        }
    };
    pin(
        config,
        Capture {
            action: Action::Pin,
            image,
            origin,
            scale,
        },
    );
    Ok(ExitCode::SUCCESS)
}

/// Keep the capture on screen until its pin is closed. Copy and Save from the pin are
/// delivered like the overlay's (with notifications); failures are only reported.
fn pin(config: AppConfig, capture: Capture) {
    let accent = config.overlay.rgb().unwrap_or((0.9, 0.28, 0.3));
    let (origin, scale) = (capture.origin, capture.scale);
    tyshot::pin::show(
        capture,
        accent,
        move |action, image| {
            let capture = Capture {
                action,
                image: image.clone(),
                origin,
                scale,
            };
            if let Err(err) = deliver(&config, capture) {
                eprintln!("error: {err:#}");
            }
        },
        gtk::main_quit,
    );
    gtk::main();
}

fn full(config_path: Option<std::path::PathBuf>, args: FullArgs) -> Result<ExitCode> {
    let config = load_config(config_path)?;
    let image = take_snapshot(args.delay)?;
    if args.raw {
        write_raw(&image)?;
        return Ok(ExitCode::SUCCESS);
    }
    let action = if args.save {
        Action::Save
    } else {
        Action::Copy
    };
    deliver(
        &config,
        Capture {
            action,
            image,
            origin: (0.0, 0.0),
            scale: ty_common::screen::logical_scale(),
        },
    )?;
    Ok(ExitCode::SUCCESS)
}

/// The screen as a pixbuf.
fn take_snapshot(delay: f64) -> Result<Pixbuf> {
    if !(0.0..=3600.0).contains(&delay) {
        bail!("--delay must be between 0 and 3600 seconds");
    }
    std::thread::sleep(Duration::from_secs_f64(delay));
    let shot = ty_common::screen::screenshot().context("failed to snapshot the screen")?;
    let image = Pixbuf::from_bytes(
        &glib::Bytes::from_owned(shot.rgb),
        Colorspace::Rgb,
        false,
        8,
        shot.width as i32,
        shot.height as i32,
        shot.width as i32 * 3,
    );
    Ok(image)
}

/// Visible windows (topmost first), for snapping. A convenience: without it, selection still
/// works by dragging.
fn visible_windows() -> Vec<ScreenRect> {
    match ty_common::wm::visible_windows() {
        Ok(windows) => windows.into_iter().map(|window| window.rect).collect(),
        Err(err) => {
            eprintln!("warning: could not list windows for snapping: {err:#}");
            Vec::new()
        }
    }
}

/// Write the image to stdout as PNG (for `--raw`; no clipboard, file or notification).
fn write_raw(image: &Pixbuf) -> Result<()> {
    let png = image
        .save_to_bufferv("png", &[])
        .context("failed to encode PNG")?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(&png)
        .and_then(|()| stdout.flush())
        .context("failed to write the PNG to stdout")
}

/// What happened to a capture.
enum Delivered {
    Copied,
    Saved(std::path::PathBuf),
    /// Attached to this opencode session; `true` if it was created for it.
    Sent(opencode::Session, bool),
}

/// Copy, save or send the capture, with a notification either way (including on failure).
fn deliver(config: &AppConfig, capture: Capture) -> Result<()> {
    let size = format!("{} × {}", capture.image.width(), capture.image.height());
    let result = encode_and_deliver(config, capture);
    match &result {
        Ok(Delivered::Copied) => {
            output::notify(&config.notifications, "Screenshot copied", &size, None)
        }
        Ok(Delivered::Saved(path)) => {
            println!("{}", path.display());
            output::notify(
                &config.notifications,
                "Screenshot saved",
                &path.display().to_string(),
                Some(path),
            );
        }
        Ok(Delivered::Sent(session, created)) => {
            let next = if config.opencode.submit {
                "The agent is looking at it"
            } else {
                "It goes along with your next message there"
            };
            let summary = if *created {
                "Screenshot sent to a new opencode session".to_string()
            } else {
                format!("Screenshot sent to opencode: {}", session.title)
            };
            output::notify(&config.notifications, &summary, next, None);
        }
        Err(err) => output::notify(
            &config.notifications,
            "Screenshot failed",
            &format!("{err:#}"),
            None,
        ),
    }
    result.map(|_| ())
}

fn encode_and_deliver(config: &AppConfig, capture: Capture) -> Result<Delivered> {
    let png = capture
        .image
        .save_to_bufferv("png", &[])
        .context("failed to encode PNG")?;
    match capture.action {
        Action::Copy => clipboard::copy_png_detached(&png).map(|()| Delivered::Copied),
        Action::Save => output::save_png(&png, &config.save).map(Delivered::Saved),
        Action::Pin => bail!("pinned captures are shown, not delivered"),
        Action::SendToOpencode(target) => {
            let timestamp = glib::DateTime::now_local()
                .and_then(|now| now.format("%Y-%m-%d_%H-%M-%S"))
                .context("failed to format the current time")?;
            let name = format!("{}-{timestamp}.png", config.save.base_name);
            let service = opencode::Service::discover()?;
            // For a new session, the directory to open it in.
            let (session, new_in) = match target {
                opencode::Target::Session(session) => (session, None),
                opencode::Target::NewSession(directory) => (
                    service
                        .create_session(&directory)
                        .context("failed to create an opencode session")?,
                    Some(directory),
                ),
            };
            service
                .send_image(&session.id, &png, &name, config.opencode.submit)
                .with_context(|| format!("failed to send to opencode session {}", session.id))?;
            if let Some(directory) = &new_in {
                opencode::open_in_terminal(&config.opencode.terminal, &session.id, directory)?;
            }
            Ok(Delivered::Sent(session, new_in.is_some()))
        }
    }
}

/// Print `<id>\t<updated>\t<directory>\t<title>` per session, most recently active first.
fn list_sessions() -> Result<ExitCode> {
    for session in opencode::Service::discover()?.sessions(usize::MAX)? {
        let updated = glib::DateTime::from_unix_local(session.updated / 1000)
            .and_then(|time| time.format("%Y-%m-%d %H:%M"))
            .map(|time| time.to_string())
            .unwrap_or_default();
        let directory = session
            .directory
            .as_ref()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        println!("{}\t{updated}\t{directory}\t{}", session.id, session.title);
    }
    Ok(ExitCode::SUCCESS)
}

fn validate_config(config_path: Option<std::path::PathBuf>) -> Result<ExitCode> {
    let loaded = AppConfig::load(config_path)?;
    loaded.config.validate()?;
    match &loaded.path {
        Some(path) => println!("Config is valid: {}", path.display()),
        None => println!("No config file found; defaults are valid"),
    }
    Ok(ExitCode::SUCCESS)
}
