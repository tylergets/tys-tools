//! The lock screen's state: what has been typed, what the auth threads said, the clock,
//! and the animations between them; turned into a [`View`] for every frame.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use ty_common::screenlock::{Canvas, Control, Damage, Handler, Key, KeyEvent, Output};

use crate::auth::{Event, Finger};
use crate::config::AppConfig;
use crate::pam::Secret;
use crate::render::{Auth, Renderer, View};

/// How long a wrong password or finger stays on screen.
const PASSWORD_ERROR_FOR: Duration = Duration::from_secs(4);
const FINGER_ERROR_FOR: Duration = Duration::from_millis(2500);
/// Longer passwords than this are cut off (typing keeps nothing past it).
const MAX_PASSWORD_CHARS: usize = 512;

/// Animation lengths.
const TRANSITION: Duration = Duration::from_millis(260);
const DOT_POP: Duration = Duration::from_millis(160);
const SHAKE: Duration = Duration::from_millis(450);
const CHECKING_PULSE: Duration = Duration::from_millis(1100);
/// From an accepted finger or password to the actual unlock, for the animation.
const FINGER_UNLOCK_AFTER: Duration = Duration::from_millis(550);
const PASSWORD_UNLOCK_AFTER: Duration = Duration::from_millis(220);
/// About 60 frames a second while something moves.
const FRAME: Duration = Duration::from_millis(16);

pub struct App {
    renderer: Renderer,
    state: State,
    quote_command: Option<Vec<String>>,
    quote: Option<Receiver<String>>,
}

/// Which auth block is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Fingerprint,
    Password,
}

/// Everything but the renderer, so a frame can borrow both.
struct State {
    time_format: String,
    date_format: String,
    show_seconds: bool,
    password: Secret,
    /// When each typed character was typed, for its dot's pop-in.
    typed_at: Vec<Instant>,
    checking_since: Option<Instant>,
    password_error: Option<(String, Instant)>,
    finger: FingerState,
    finger_error: Option<(String, Instant)>,
    caps_lock: bool,
    events: Receiver<Event>,
    submit: Sender<Secret>,
    /// Authenticated: unlock once the animation has played.
    unlock_at: Option<Instant>,
    accepted_at: Option<(Mode, Instant)>,
    mode: Mode,
    /// The mode before the last change, and when it changed.
    changed: Option<(Mode, Instant)>,
    shake_since: Option<Instant>,
    /// Whether a frame was shown yet (changes before it don't animate).
    drawn: bool,
    dirty: bool,
    /// The clock text last drawn, to redraw when it changes.
    shown_clock: (String, String),
    on_locked: Option<Box<dyn FnOnce()>>,
}

#[derive(PartialEq, Eq)]
enum FingerState {
    Off,
    Available,
}

impl App {
    pub fn new(
        config: &AppConfig,
        renderer: Renderer,
        events: Receiver<Event>,
        submit: Sender<Secret>,
        finger_available: bool,
        on_locked: Box<dyn FnOnce()>,
    ) -> Self {
        let show_seconds = ["%S", "%T", "%X", "%r", "%s"]
            .iter()
            .any(|spec| config.time_format.contains(spec));
        let state = State {
            time_format: config.time_format.clone(),
            date_format: config.date_format.clone(),
            show_seconds,
            password: Secret::default(),
            typed_at: Vec::new(),
            checking_since: None,
            password_error: None,
            finger: if finger_available {
                FingerState::Available
            } else {
                FingerState::Off
            },
            finger_error: None,
            caps_lock: false,
            events,
            submit,
            unlock_at: None,
            accepted_at: None,
            mode: if finger_available {
                Mode::Fingerprint
            } else {
                Mode::Password
            },
            changed: None,
            drawn: false,
            shake_since: None,
            dirty: true,
            shown_clock: (String::new(), String::new()),
            on_locked: Some(on_locked),
        };
        Self { renderer, state, quote_command: config.quote_command.clone(), quote: None }
    }
}

/// How far an animation that started at `since` and lasts `length` has gone, 0 to 1.
fn progress(since: Instant, length: Duration, now: Instant) -> f64 {
    (now.saturating_duration_since(since).as_secs_f64() / length.as_secs_f64()).min(1.0)
}

impl State {
    fn clock(&self) -> (String, String) {
        let now = Local::now();
        (
            now.format(&self.time_format).to_string(),
            now.format(&self.date_format).to_string(),
        )
    }

    /// When the clock text can next change.
    fn next_tick(&self) -> Instant {
        let now = Local::now();
        let into_second = Duration::from_nanos(u64::from(now.nanosecond() % 1_000_000_000));
        let to_next_second = Duration::from_secs(1).saturating_sub(into_second);
        let wait = if self.show_seconds {
            to_next_second
        } else {
            Duration::from_secs(u64::from(59 - now.second().min(59))) + to_next_second
        };
        // A little late rather than early, so the new minute has begun.
        Instant::now() + wait + Duration::from_millis(5)
    }

    fn checking(&self) -> bool {
        self.checking_since.is_some()
    }

    fn unlocking(&self) -> bool {
        self.unlock_at.is_some()
    }

    /// The block that should be on screen now.
    fn wanted_mode(&self) -> Mode {
        if let Some((mode, _)) = self.accepted_at {
            return mode;
        }
        let typing = !self.password.is_empty() || self.checking() || self.password_error.is_some();
        match self.finger {
            FingerState::Available if !typing => Mode::Fingerprint,
            _ => Mode::Password,
        }
    }

    fn clear_password(&mut self) {
        self.password.clear();
        self.typed_at.clear();
    }

    fn submit_password(&mut self) {
        if self.password.is_empty() || self.checking() {
            return;
        }
        if self.submit.send(self.password.take()).is_ok() {
            self.checking_since = Some(Instant::now());
            self.password_error = None;
        }
    }

    fn accept(&mut self, mode: Mode, delay: Duration) {
        let now = Instant::now();
        self.accepted_at = Some((mode, now));
        self.unlock_at = Some(now + delay);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Password(Ok(())) => {
                self.checking_since = None;
                if !self.unlocking() {
                    self.accept(Mode::Password, PASSWORD_UNLOCK_AFTER);
                }
            }
            Event::Password(Err(message)) => {
                self.checking_since = None;
                self.typed_at.clear();
                self.password_error = Some((message, Instant::now() + PASSWORD_ERROR_FOR));
                self.shake_since = Some(Instant::now());
            }
            Event::Finger(Finger::Available) => self.finger = FingerState::Available,
            Event::Finger(Finger::Retry(message)) => {
                self.finger = FingerState::Available;
                self.finger_error = Some((message, Instant::now() + FINGER_ERROR_FOR));
                if self.mode == Mode::Fingerprint {
                    self.shake_since = Some(Instant::now());
                }
            }
            Event::Finger(Finger::Matched) => {
                if !self.unlocking() {
                    self.accept(Mode::Fingerprint, FINGER_UNLOCK_AFTER);
                }
            }
            Event::Finger(Finger::Unavailable) => {
                self.finger = FingerState::Off;
                self.finger_error = None;
            }
        }
        self.dirty = true;
    }

    fn auth(&self, mode: Mode, now: Instant) -> Auth<'_> {
        let accepted = self
            .accepted_at
            .filter(|(m, _)| *m == mode)
            .map(|(_, at)| at);
        match mode {
            Mode::Fingerprint => Auth::Fingerprint {
                error: self.finger_error.as_ref().map(|(m, _)| m.as_str()),
                matched: accepted.map(|at| progress(at, FINGER_UNLOCK_AFTER, now)),
            },
            Mode::Password => Auth::Password {
                dots: self
                    .typed_at
                    .iter()
                    .map(|&at| progress(at, DOT_POP, now))
                    .collect(),
                checking: self.checking_since.map(|since| {
                    let elapsed = now.saturating_duration_since(since).as_secs_f64();
                    (elapsed / CHECKING_PULSE.as_secs_f64()).fract()
                }),
                error: self.password_error.as_ref().map(|(m, _)| m.as_str()),
                accepted: accepted.is_some(),
            },
        }
    }

    fn view<'a>(&'a self, time: &'a str, date: &'a str, now: Instant) -> View<'a> {
        let (previous, transition) = match self.changed {
            Some((previous, at)) if progress(at, TRANSITION, now) < 1.0 => (
                Some(self.auth(previous, now)),
                progress(at, TRANSITION, now),
            ),
            _ => (None, 1.0),
        };
        let shake = self.shake_since.map_or(0.0, |since| {
            let t = progress(since, SHAKE, now);
            (t * std::f64::consts::PI * 6.0).sin() * 12.0 * (1.0 - t)
        });
        View {
            time,
            date,
            caps_lock: self.caps_lock,
            auth: self.auth(self.mode, now),
            previous,
            transition,
            shake,
        }
    }

    /// Whether something is moving at `now`.
    fn animating(&self, now: Instant) -> bool {
        let running = |since: Option<Instant>, length| {
            since.is_some_and(|since| progress(since, length, now) < 1.0)
        };
        running(self.changed.map(|(_, at)| at), TRANSITION)
            || running(self.shake_since, SHAKE)
            || running(self.typed_at.last().copied(), DOT_POP)
            || self.checking()
            || self.unlocking()
    }

    fn key(&mut self, event: KeyEvent) {
        // Typing while a password is checked would be lost or confusing; ignore it.
        if self.unlocking() || self.checking() {
            return;
        }
        match event.key {
            Key::Enter => self.submit_password(),
            Key::Escape => self.clear_password(),
            Key::Backspace if event.ctrl => self.clear_password(),
            Key::Backspace => {
                self.password.pop();
                self.typed_at.pop();
            }
            Key::Text(text) if event.ctrl => {
                if text.eq_ignore_ascii_case("u") {
                    self.clear_password();
                }
            }
            Key::Text(text) => {
                let count = text.chars().count();
                if self.password.chars() + count <= MAX_PASSWORD_CHARS {
                    self.password.push_str(&text);
                    let now = Instant::now();
                    self.typed_at.extend(std::iter::repeat_n(now, count));
                }
                self.password_error = None;
            }
            Key::Other => return,
        }
        self.dirty = true;
    }

    /// Take in the auth threads' news and switch blocks if needed.
    fn update(&mut self, now: Instant) {
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
        let mode = self.wanted_mode();
        if mode != self.mode {
            self.changed = self.drawn.then_some((self.mode, now));
            self.mode = mode;
            self.dirty = true;
        }
    }

    fn control(&mut self) -> Control {
        let now = Instant::now();
        self.update(now);
        let mut wake_at = self.next_tick();
        for slot in [&mut self.password_error, &mut self.finger_error] {
            match slot {
                Some((_, until)) if *until <= now => {
                    *slot = None;
                    self.dirty = true;
                }
                Some((_, until)) => wake_at = wake_at.min(*until),
                None => {}
            }
        }
        if self.clock() != self.shown_clock {
            self.dirty = true;
        }
        let unlock = self.unlock_at.is_some_and(|at| at <= now);
        if self.animating(now) {
            self.dirty = true;
            wake_at = wake_at.min(now + FRAME);
            if let Some(at) = self.unlock_at {
                wake_at = wake_at.min(at);
            }
        }
        Control {
            redraw: std::mem::take(&mut self.dirty),
            unlock,
            wake_at: Some(wake_at),
        }
    }
}

impl Handler for App {
    fn draw(&mut self, output: &Output, canvas: Canvas<'_>) -> Option<Damage> {
        let now = Instant::now();
        if !self.state.drawn {
            // The first frame: anything the auth threads said already belongs in it.
            self.state.update(now);
        }
        let (time, date) = self.state.clock();
        let view = self.state.view(&time, &date, now);
        let damage = self.renderer.draw(output, canvas, &view);
        self.state.drawn = true;
        self.state.shown_clock = (time, date);
        damage
    }

    fn key(&mut self, event: KeyEvent) {
        self.state.key(event);
    }

    fn caps_lock(&mut self, on: bool) {
        if self.state.caps_lock != on {
            self.state.caps_lock = on;
            self.state.dirty = true;
        }
    }

    fn locked(&mut self) {
        if let Some(on_locked) = self.state.on_locked.take() {
            on_locked();
        }
        if let Some(command) = self.quote_command.take() {
            self.quote = Some(crate::quote::spawn(command));
        }
    }

    fn control(&mut self) -> Control {
        if let Some(receiver) = &self.quote {
            match receiver.try_recv() {
                Ok(text) => {
                    self.renderer.set_quote(text);
                    self.state.dirty = true;
                    self.quote = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.quote = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut control = self.state.control();
        if self.quote.is_some() {
            let poll = Instant::now() + Duration::from_millis(100);
            control.wake_at = Some(control.wake_at.map_or(poll, |at| at.min(poll)));
        }
        control
    }
}
