//! The daemon: one loop that owns the gamma backend and reacts to commands, location fixes
//! and the clock.

use std::fs::File;
use std::io::Read as _;
use std::os::fd::FromRawFd as _;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use ty_common::lock::InstanceLock;
use ty_common::wake::{self, Waker, Wakeups};

use crate::adaptive::Adaptive;
use crate::ambient::Sensor;
use crate::backend::{self, Backend};
use crate::color::Setting;
use crate::config::{validate_temperature, AppConfig, APP_NAME};
use crate::monitors::{Change, Monitors, MANUAL_FADE};
use crate::protocol::{Command, Status};
use crate::schedule::{self, History, Sample, Scheduled};
use crate::solar::{self, Location};
use crate::{apps, control, geoclue};

/// How often the sun position (and outputs) are re-checked.
const IDLE_TICK: Duration = Duration::from_secs(5);
/// How often the ambient light sensor is read.
const SENSOR_TICK: Duration = Duration::from_secs(1);
/// Frame interval while fading.
const FADE_TICK: Duration = Duration::from_millis(30);

pub enum Msg {
    Command(Command, Option<Sender<Result<Status, String>>>),
    Location(Location, &'static str),
    Quit,
}

/// State shared with the socket, sensor and tray threads.
#[derive(Clone)]
pub struct Shared {
    pub config: Arc<AppConfig>,
    pub status: Arc<Mutex<Status>>,
    pub monitors: Arc<Mutex<Monitors>>,
    pub adaptive: Arc<Mutex<Adaptive>>,
    pub history: Arc<Mutex<History>>,
    /// Set to ask the tray thread to open the graph window.
    pub show_graph: Arc<AtomicBool>,
    /// Whether the GTK (tray) thread is running, so windows can be shown.
    pub gui: Arc<AtomicBool>,
    pub has_sensor: Arc<AtomicBool>,
    /// Tells the tray that `status` (or `show_graph`) changed.
    pub waker: Waker,
}

impl Shared {
    /// The shared state, and the tray's end of [`Shared::waker`].
    pub fn new(config: AppConfig) -> (Self, Wakeups) {
        let (waker, wakeups) = wake::pair();
        let shared = Self {
            adaptive: Arc::new(Mutex::new(Adaptive::new(config.auto_brightness.clone()))),
            config: Arc::new(config),
            status: Arc::default(),
            monitors: Arc::default(),
            history: Arc::new(Mutex::new(History::load())),
            show_graph: Arc::default(),
            gui: Arc::default(),
            has_sensor: Arc::default(),
            waker,
        };
        (shared, wakeups)
    }

    pub fn status(&self) -> Status {
        self.status.lock().expect("status lock").clone()
    }

    fn set_monitor_status(&self, percent: u8) -> Status {
        let mut status = self.status.lock().expect("status lock");
        status.monitor_brightness = Some(percent);
        self.waker.wake();
        status.clone()
    }

    /// A manual brightness change (blocks while DDC is slow). With auto brightness on, it
    /// becomes the new offset on the curve.
    pub fn change_brightness(&self, change: Change) -> Result<Status> {
        let percent =
            self.monitors
                .lock()
                .expect("monitors lock")
                .change(change, MANUAL_FADE, true)?;
        let mut adaptive = self.adaptive.lock().expect("adaptive lock");
        if adaptive.enabled {
            adaptive.on_manual(percent);
        }
        drop(adaptive);
        Ok(self.set_monitor_status(percent))
    }

    /// A change asked for by auto brightness: slower fade, backlight only unless configured.
    fn auto_brightness_to(&self, percent: u8) -> Result<()> {
        let settings = &self.config.auto_brightness;
        let percent = self.monitors.lock().expect("monitors lock").change(
            Change::Set(percent),
            Duration::from_secs_f64(settings.fade_seconds),
            settings.ddc,
        )?;
        self.set_monitor_status(percent);
        Ok(())
    }

    pub fn set_auto_brightness(&self, on: Option<bool>) -> Result<Status> {
        if !self.has_sensor.load(Ordering::Relaxed) {
            bail!("no ambient light sensor found");
        }
        let enabled = {
            let mut adaptive = self.adaptive.lock().expect("adaptive lock");
            let enabled = on.unwrap_or(!adaptive.enabled);
            adaptive.set_enabled(enabled);
            enabled
        };
        let mut status = self.status.lock().expect("status lock");
        status.auto_brightness = Some(enabled);
        self.waker.wake();
        Ok(status.clone())
    }

    pub fn request_graph(&self) -> Result<Status> {
        if !self.gui.load(Ordering::Relaxed) {
            bail!("the graph needs the tray (set `tray: true`)");
        }
        self.show_graph.store(true, Ordering::Relaxed);
        self.waker.wake();
        Ok(self.status())
    }
}

/// Reads the light sensor, keeps the status current and drives auto brightness.
fn watch_sensor(sensor: Sensor, shared: Shared) {
    thread::spawn(move || {
        let mut last_error = String::new();
        loop {
            let lux = sensor.lux();
            let changed = {
                let mut status = shared.status.lock().expect("status lock");
                std::mem::replace(&mut status.ambient_lux, lux) != lux
            };
            if changed {
                shared.waker.wake();
            }
            if let Some(lux) = lux {
                // Notice brightness changed elsewhere (brightness keys, other tools).
                // Skipped while a change is running (the monitors are locked).
                let current = shared
                    .monitors
                    .try_lock()
                    .ok()
                    .and_then(|monitors| monitors.percent());
                let target = {
                    let mut adaptive = shared.adaptive.lock().expect("adaptive lock");
                    if let (true, Some(current), Some(last)) =
                        (adaptive.enabled, current, adaptive.last_target())
                    {
                        if current.abs_diff(last) > 2 {
                            adaptive.on_manual(current);
                        }
                    }
                    adaptive.update(lux, Instant::now())
                };
                if let Some(percent) = target {
                    if let Err(err) = shared.auto_brightness_to(percent) {
                        let message = format!("{err:#}");
                        if message != last_error {
                            eprintln!("auto brightness: {message}");
                            last_error = message;
                        }
                    }
                }
            }
            thread::sleep(SENSOR_TICK);
        }
    });
}

/// A temperature held by `set`, or a preset.
struct Override {
    preset: Option<String>,
    temperature: Option<u32>,
    brightness: Option<f64>,
}

struct Fade {
    from: Setting,
    /// Target when the fade started; a new jump restarts the fade.
    to: Setting,
    started: Instant,
}

struct Daemon {
    config: AppConfig,
    backend: Box<dyn Backend>,
    location: Option<(Location, &'static str)>,
    enabled: bool,
    suspended_by: Vec<String>,
    last_app_check: Option<Instant>,
    disabled_until: Option<u64>,
    manual: Option<Override>,
    applied: Option<Setting>,
    applied_gamma: Option<f64>,
    fade: Option<Fade>,
    shared: Shared,
}

pub fn run(config: AppConfig) -> Result<()> {
    let lock = InstanceLock::acquire(APP_NAME)?;
    let (tx, rx) = mpsc::channel();
    let (shared, _wakeups) = Shared::new(config.clone());

    let backend = backend::open(config.backend)?;
    println!("Using the {} backend", backend.name());

    control::serve(lock.listener().try_clone()?, tx.clone(), shared.clone());
    watch_signals(tx.clone())?;

    let location = match config.location {
        Some(location) => Some((location, "config")),
        None => {
            let tx = tx.clone();
            geoclue::watch(move |location| {
                let _ = tx.send(Msg::Location(location, "geoclue"));
            });
            geoclue::load_cache().map(|location| (location, "cache"))
        }
    };

    {
        let (monitors, settings) = (shared.monitors.clone(), config.monitor_brightness);
        thread::spawn(move || {
            let found = Monitors::detect(&settings);
            *monitors.lock().expect("monitors lock") = found;
        });
    }

    if let Some(sensor) = config.ambient_light.then(Sensor::find).flatten() {
        println!("Found an ambient light sensor");
        shared.has_sensor.store(true, Ordering::Relaxed);
        watch_sensor(sensor, shared.clone());
    }

    #[cfg(target_os = "linux")]
    let _tray = config
        .tray
        .then(|| crate::tray::start(tx.clone(), shared.clone(), _wakeups))
        .flatten();

    let mut daemon = Daemon {
        config,
        backend,
        location,
        enabled: true,
        suspended_by: Vec::new(),
        last_app_check: None,
        disabled_until: None,
        manual: None,
        applied: None,
        applied_gamma: None,
        fade: None,
        shared,
    };

    let result = daemon.run(rx);
    daemon.shared.history.lock().expect("history lock").save();
    if let Err(err) = daemon.backend.restore() {
        eprintln!("warning: failed to restore gamma: {err:#}");
    }
    result
}

/// Write end of the pipe the signal handler pokes; -1 until `watch_signals` runs.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    // Only async-signal-safe calls here: write one byte for the watcher thread.
    let byte = 1u8;
    unsafe {
        libc::write(
            SIGNAL_PIPE.load(Ordering::Relaxed),
            (&raw const byte).cast(),
            1,
        )
    };
}

/// Quit cleanly (restoring gamma) on SIGTERM, SIGINT or SIGHUP.
fn watch_signals(tx: Sender<Msg>) -> Result<()> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    SIGNAL_PIPE.store(fds[1], Ordering::Relaxed);
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    // SAFETY: fds[0] is the pipe's read end, owned by nothing else.
    let mut pipe = unsafe { File::from_raw_fd(fds[0]) };
    thread::spawn(move || {
        if pipe.read(&mut [0]).is_ok() {
            let _ = tx.send(Msg::Quit);
        }
    });
    Ok(())
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Ease in and out, so fades start and end gently.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Daemon {
    fn run(&mut self, rx: mpsc::Receiver<Msg>) -> Result<()> {
        loop {
            self.update()?;
            let tick = if self.fade.is_some() {
                FADE_TICK
            } else {
                IDLE_TICK
            };
            match rx.recv_timeout(tick) {
                Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Ok(Msg::Command(command, reply)) => {
                    let result = self.handle(command);
                    self.update()?;
                    if let Some(reply) = reply {
                        let _ = reply.send(
                            result
                                .map(|()| self.shared.status())
                                .map_err(|err| format!("{err:#}")),
                        );
                    }
                }
                Ok(Msg::Location(location, source)) => {
                    if self.location.is_none() || source != "cache" {
                        println!(
                            "Location from {source}: {:.2}, {:.2}",
                            location.latitude, location.longitude
                        );
                        self.location = Some((location, source));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn handle(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Status => {}
            Command::Enable => self.enable(),
            Command::Disable(secs) => self.disable(secs.map(|s| unix_now() as u64 + s)),
            Command::DisableUntilMorning => {
                let Some((location, _)) = self.location else {
                    bail!("the location isn't known yet");
                };
                let Some(morning) =
                    solar::next_day_start(unix_now(), location, &self.config.schedule)
                else {
                    bail!("the sun doesn't come up in the next two days here");
                };
                self.disable(Some(morning as u64));
            }
            Command::Toggle if self.enabled => self.disable(None),
            Command::Toggle => self.enable(),
            Command::Set(kelvin) => {
                validate_temperature("temperature", kelvin)?;
                self.manual = Some(Override {
                    preset: None,
                    temperature: Some(kelvin),
                    brightness: None,
                });
                self.enable();
            }
            Command::Preset(name) => {
                let Some(preset) = self.config.presets.get(&name).copied() else {
                    let names: Vec<&str> = self.config.presets.keys().map(String::as_str).collect();
                    bail!("unknown preset '{name}' (have: {})", names.join(", "));
                };
                self.manual = Some(Override {
                    preset: Some(name),
                    temperature: preset.temperature,
                    brightness: preset.brightness,
                });
                self.enable();
                if let Some(percent) = preset.monitor_brightness {
                    let shared = self.shared.clone();
                    thread::spawn(move || {
                        if let Err(err) = shared.change_brightness(Change::Set(percent)) {
                            eprintln!("preset brightness: {err:#}");
                        }
                    });
                }
            }
            Command::Auto => self.manual = None,
            // Handled on the caller's thread (see `Shared`).
            Command::Brightness(_) | Command::AutoBrightness(_) | Command::ShowGraph => {}
        }
        Ok(())
    }

    fn enable(&mut self) {
        self.enabled = true;
        self.disabled_until = None;
    }

    fn disable(&mut self, until: Option<u64>) {
        self.enabled = false;
        self.disabled_until = until;
    }

    /// The setting to aim for now, and what the schedule says.
    fn target(&self, now: f64) -> (Setting, Scheduled) {
        let location = self.location.map(|(location, _)| location);
        let scheduled = schedule::scheduled(&self.config, location, now);
        let base = scheduled.setting;
        let setting = match &self.manual {
            _ if !self.enabled || !self.suspended_by.is_empty() => Setting::NEUTRAL,
            Some(manual) => Setting {
                temperature: manual.temperature.map_or(base.temperature, f64::from),
                brightness: manual.brightness.unwrap_or(base.brightness),
            },
            None => base,
        };
        (setting, scheduled)
    }

    fn update(&mut self) -> Result<()> {
        let now = unix_now();
        if !self.config.app_whitelist.is_empty()
            && self
                .last_app_check
                .is_none_or(|last| last.elapsed() >= IDLE_TICK)
        {
            self.last_app_check = Some(Instant::now());
            match apps::running(&self.config.app_whitelist) {
                Ok(apps) => self.suspended_by = apps,
                Err(err) => eprintln!("app whitelist: {err:#}"),
            }
        }
        if self.disabled_until.is_some_and(|until| now >= until as f64) {
            self.enable();
        }
        let reapply = self.backend.poll()?;
        let (target, scheduled) = self.target(now);
        let current = self.applied.unwrap_or(Setting::NEUTRAL);

        // Fade on jumps (toggles, `set`, the first location fix); the sun moves slowly
        // enough on its own.
        let fade_secs = self.config.fade_seconds;
        let jumped = match &self.fade {
            Some(fade) => fade.to.far_from(target),
            None => current.far_from(target),
        };
        if jumped && fade_secs > 0.0 {
            self.fade = Some(Fade {
                from: current,
                to: target,
                started: Instant::now(),
            });
        }
        let setting = match &self.fade {
            Some(fade) => {
                let t = fade.started.elapsed().as_secs_f64() / fade_secs;
                if t >= 1.0 {
                    self.fade = None;
                    target
                } else {
                    fade.from.lerp(target, smoothstep(t))
                }
            }
            None => target,
        };

        let gamma = if self.suspended_by.is_empty() {
            self.config.gamma
        } else {
            1.0
        };
        // Once neutral, leave games' own gamma adjustments alone while suspended.
        if (reapply && self.suspended_by.is_empty())
            || self.applied_gamma != Some(gamma)
            || self
                .applied
                .is_none_or(|applied| !applied.close_to(setting))
        {
            self.backend.apply(setting, gamma)?;
            self.applied = Some(setting);
            self.applied_gamma = Some(gamma);
        }

        let monitor_brightness = self
            .shared
            .monitors
            .try_lock()
            .ok()
            .map(|monitors| monitors.percent());
        let auto_brightness = self
            .shared
            .has_sensor
            .load(Ordering::Relaxed)
            .then(|| self.shared.adaptive.lock().expect("adaptive lock").enabled);
        let mut status = self.shared.status.lock().expect("status lock");
        let next = Status {
            running: true,
            enabled: self.enabled,
            suspended_by: self.suspended_by.clone(),
            disabled_until: self.disabled_until,
            manual: self
                .manual
                .as_ref()
                .filter(|manual| manual.preset.is_none())
                .and_then(|manual| manual.temperature),
            preset: self
                .manual
                .as_ref()
                .and_then(|manual| manual.preset.clone()),
            period: scheduled.period,
            stage: scheduled.stage,
            temperature: setting.temperature.round() as u32,
            brightness: setting.brightness,
            elevation: scheduled.elevation,
            location: self.location.map(|(location, _)| location),
            location_source: self.location.map(|(_, source)| source.to_string()),
            backend: self.backend.name().to_string(),
            monitor_brightness: monitor_brightness.unwrap_or(status.monitor_brightness),
            // Kept up to date by the sensor thread.
            ambient_lux: status.ambient_lux,
            auto_brightness,
        };
        if *status != next {
            *status = next;
            self.shared.waker.wake();
        }
        let sample = Sample {
            unix: now,
            temperature: status.temperature,
            brightness: status.brightness,
            monitor: status.monitor_brightness,
            lux: status.ambient_lux,
        };
        drop(status);
        self.shared
            .history
            .lock()
            .expect("history lock")
            .record(sample);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingBackend(Arc<Mutex<Vec<(Setting, f64)>>>);

    impl Backend for RecordingBackend {
        fn name(&self) -> &'static str {
            "test"
        }

        fn apply(&mut self, setting: Setting, gamma: f64) -> Result<()> {
            self.0.lock().unwrap().push((setting, gamma));
            Ok(())
        }

        fn poll(&mut self) -> Result<bool> {
            Ok(true)
        }

        fn restore(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn app_pause_restores_preset_and_preserves_explicit_disable() {
        let config = AppConfig {
            fade_seconds: 0.0,
            gamma: 1.2,
            ..AppConfig::default()
        };
        let (mut shared, _wakeups) = Shared::new(config.clone());
        shared.history = Arc::default();
        let backend = RecordingBackend::default();
        let applied = backend.0.clone();
        let mut daemon = Daemon {
            config,
            backend: Box::new(backend),
            location: None,
            enabled: true,
            suspended_by: Vec::new(),
            last_app_check: None,
            disabled_until: None,
            manual: None,
            applied: None,
            applied_gamma: None,
            fade: None,
            shared,
        };
        daemon.handle(Command::Preset("evening".into())).unwrap();
        daemon.update().unwrap();
        let preset = *applied.lock().unwrap().last().unwrap();

        daemon.suspended_by = vec!["steam".into()];
        daemon.update().unwrap();
        assert_eq!(
            applied.lock().unwrap().last(),
            Some(&(Setting::NEUTRAL, 1.0))
        );
        assert_eq!(daemon.shared.status().summary(), "Paused · steam");
        let count = applied.lock().unwrap().len();
        daemon.update().unwrap();
        assert_eq!(applied.lock().unwrap().len(), count);

        daemon.suspended_by.clear();
        daemon.update().unwrap();
        assert_eq!(applied.lock().unwrap().last(), Some(&preset));
        assert_eq!(daemon.shared.status().preset.as_deref(), Some("evening"));

        daemon.suspended_by = vec!["steam".into()];
        daemon.handle(Command::Disable(Some(600))).unwrap();
        let until = daemon.disabled_until;
        daemon.update().unwrap();
        daemon.suspended_by.clear();
        daemon.update().unwrap();
        assert!(!daemon.shared.status().enabled);
        assert_eq!(daemon.disabled_until, until);
        assert_eq!(daemon.target(unix_now()).0, Setting::NEUTRAL);
    }
}
