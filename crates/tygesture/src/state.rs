//! State shared between the libinput loop, the tray and the window.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{Config, Thresholds};
use crate::gesture::Raw;

/// How long the tray icon and window border stay lit after a gesture fires.
pub const FLASH: Duration = Duration::from_millis(350);
const HISTORY: usize = 12;
const MAX_TRAIL: usize = 4000;

pub type Shared = Arc<Mutex<Status>>;

#[derive(Debug, Default)]
pub struct Status {
    pub paused: bool,
    pub touchpad: Option<String>,
    pub fired: usize,
    pub flash_until: Option<Instant>,
    /// The gesture in progress, or the last one (see `ended`).
    pub live: Option<Live>,
    /// When the live gesture ended, and whether it was cancelled.
    pub ended: Option<(Instant, bool)>,
    /// Most recent last.
    pub history: VecDeque<Fired>,
    /// `(gesture, command)` for every binding, for the window.
    pub bindings: Vec<(String, String)>,
    pub thresholds: Thresholds,
    /// Set by `tygesture show`; the tray thread opens the window.
    pub show_window: bool,
}

#[derive(Debug, Clone)]
pub enum Live {
    /// Cumulative finger position from the start of the swipe.
    Swipe {
        fingers: u32,
        trail: Vec<(f64, f64)>,
    },
    Pinch {
        fingers: u32,
        scale: f64,
        angle: f64,
    },
    Hold {
        fingers: u32,
        since: Instant,
    },
}

#[derive(Debug, Clone)]
pub struct Fired {
    pub at: Instant,
    pub gesture: String,
    pub command: String,
}

impl Status {
    pub fn new(cfg: &Config) -> Self {
        Self {
            bindings: cfg
                .bindings()
                .into_iter()
                .map(|(g, b)| (g.to_string(), b.command.clone()))
                .collect(),
            thresholds: cfg.thresholds.clone(),
            ..Self::default()
        }
    }

    pub fn is_lit(&self) -> bool {
        self.flash_until.is_some_and(|t| Instant::now() < t)
    }

    /// Record a fired gesture and light the icon.
    pub fn fired(&mut self, gesture: String, command: String) {
        self.fired += 1;
        self.flash_until = Some(Instant::now() + FLASH);
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(Fired {
            at: Instant::now(),
            gesture,
            command,
        });
    }

    /// Follow raw libinput events so the window can draw the gesture in progress.
    pub fn track(&mut self, raw: Raw) {
        match (raw, &mut self.live) {
            (Raw::SwipeBegin(fingers), _) => {
                self.begin(Live::Swipe {
                    fingers,
                    trail: vec![(0.0, 0.0)],
                });
            }
            (Raw::SwipeUpdate { dx, dy }, Some(Live::Swipe { trail, .. })) => {
                let (x, y) = trail.last().copied().unwrap_or_default();
                if trail.len() < MAX_TRAIL {
                    trail.push((x + dx, y + dy));
                }
            }
            (Raw::PinchBegin(fingers), _) => self.begin(Live::Pinch {
                fingers,
                scale: 1.0,
                angle: 0.0,
            }),
            (Raw::PinchUpdate { scale: s, angle: a }, Some(Live::Pinch { scale, angle, .. })) => {
                *scale = s;
                *angle += a;
            }
            (Raw::HoldBegin(fingers), _) => self.begin(Live::Hold {
                fingers,
                since: Instant::now(),
            }),
            (
                Raw::SwipeEnd { cancelled }
                | Raw::PinchEnd { cancelled }
                | Raw::HoldEnd { cancelled },
                Some(_),
            ) => self.ended = Some((Instant::now(), cancelled)),
            _ => {}
        }
    }

    fn begin(&mut self, live: Live) {
        self.live = Some(live);
        self.ended = None;
    }
}
