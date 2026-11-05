//! Turns libinput's gesture begin/update/end events into named gestures.
//!
//! Plain bindings fire once when the gesture ends. `repeat` bindings fire during the
//! gesture, once per step of movement, so a swipe or a twist can work like a slider.

use std::fmt;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spin {
    Cw,
    Ccw,
}

/// A recognized gesture; the `u32` is the finger count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Swipe(u32, Dir),
    Pinch(u32, Zoom),
    Rotate(u32, Spin),
    Hold(u32),
}

impl fmt::Display for Gesture {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Gesture::Swipe(n, dir) => write!(f, "swipe {n} {}", format!("{dir:?}").to_lowercase()),
            Gesture::Pinch(n, zoom) => {
                write!(f, "pinch {n} {}", format!("{zoom:?}").to_lowercase())
            }
            Gesture::Rotate(n, spin) => {
                write!(f, "rotate {n} {}", format!("{spin:?}").to_lowercase())
            }
            Gesture::Hold(n) => write!(f, "hold {n}"),
        }
    }
}

/// libinput gesture events, reduced to what the recognizer needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Raw {
    SwipeBegin(u32),
    /// Unaccelerated finger motion since the last update.
    SwipeUpdate {
        dx: f64,
        dy: f64,
    },
    SwipeEnd {
        cancelled: bool,
    },
    PinchBegin(u32),
    /// `scale` is absolute (1.0 at the start); `angle` is degrees since the last update.
    PinchUpdate {
        scale: f64,
        angle: f64,
    },
    PinchEnd {
        cancelled: bool,
    },
    HoldBegin(u32),
    HoldEnd {
        cancelled: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Run this gesture's binding (if it has one).
    Fire(Gesture),
    /// A gesture finished; a one-line summary for `tygesture debug`.
    Ended(String),
}

#[derive(Debug, Default)]
pub struct Recognizer {
    state: State,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Idle,
    Swipe {
        fingers: u32,
        total: (f64, f64),
        /// Motion since the last repeat firing.
        step: (f64, f64),
        repeated: bool,
    },
    Pinch {
        fingers: u32,
        scale: f64,
        angle: f64,
        /// Scale at the last repeat zoom firing.
        base_scale: f64,
        /// Rotation since the last repeat rotate firing.
        angle_step: f64,
        repeated: bool,
    },
    Hold {
        fingers: u32,
    },
}

impl Recognizer {
    pub fn feed(&mut self, cfg: &Config, raw: Raw) -> Vec<Output> {
        let t = &cfg.thresholds;
        let mut out = Vec::new();
        match (raw, &mut self.state) {
            (Raw::SwipeBegin(fingers), _) => {
                self.state = State::Swipe {
                    fingers,
                    total: (0.0, 0.0),
                    step: (0.0, 0.0),
                    repeated: false,
                };
            }
            (
                Raw::SwipeUpdate { dx, dy },
                State::Swipe {
                    fingers,
                    total,
                    step,
                    repeated,
                },
            ) => {
                total.0 += dx;
                total.1 += dy;
                step.0 += dx;
                step.1 += dy;
                while let Some((dir, travel)) = dominant(*step) {
                    let gesture = Gesture::Swipe(*fingers, dir);
                    if travel < t.swipe_step || !cfg.repeats(gesture) {
                        break;
                    }
                    out.push(Output::Fire(gesture));
                    *repeated = true;
                    // Keep the overshoot along the swipe axis so fast swipes fire several times.
                    *step = match dir {
                        Dir::Left => (step.0 + t.swipe_step, 0.0),
                        Dir::Right => (step.0 - t.swipe_step, 0.0),
                        Dir::Up => (0.0, step.1 + t.swipe_step),
                        Dir::Down => (0.0, step.1 - t.swipe_step),
                    };
                }
            }
            (
                Raw::SwipeEnd { cancelled },
                State::Swipe {
                    fingers,
                    total,
                    repeated,
                    ..
                },
            ) => {
                let summary = format!("swipe {fingers}: dx={:.0} dy={:.0}", total.0, total.1);
                if !cancelled && !*repeated {
                    if let Some((dir, travel)) = dominant(*total) {
                        if travel >= t.swipe {
                            out.push(Output::Fire(Gesture::Swipe(*fingers, dir)));
                        }
                    }
                }
                out.push(Output::Ended(with_cancel(summary, cancelled)));
                self.state = State::Idle;
            }
            (Raw::PinchBegin(fingers), _) => {
                self.state = State::Pinch {
                    fingers,
                    scale: 1.0,
                    angle: 0.0,
                    base_scale: 1.0,
                    angle_step: 0.0,
                    repeated: false,
                };
            }
            (
                Raw::PinchUpdate { scale: s, angle: a },
                State::Pinch {
                    fingers,
                    scale,
                    angle,
                    base_scale,
                    angle_step,
                    repeated,
                },
            ) => {
                *scale = s;
                *angle += a;
                *angle_step += a;
                if let Some(zoom) = zoom(s / *base_scale, t.pinch) {
                    let gesture = Gesture::Pinch(*fingers, zoom);
                    if cfg.repeats(gesture) {
                        out.push(Output::Fire(gesture));
                        *base_scale = s;
                        *repeated = true;
                    }
                }
                while angle_step.abs() >= t.rotate {
                    let gesture = Gesture::Rotate(*fingers, spin(*angle_step));
                    if !cfg.repeats(gesture) {
                        break;
                    }
                    out.push(Output::Fire(gesture));
                    *angle_step -= t.rotate.copysign(*angle_step);
                    *repeated = true;
                }
            }
            (
                Raw::PinchEnd { cancelled },
                State::Pinch {
                    fingers,
                    scale,
                    angle,
                    repeated,
                    ..
                },
            ) => {
                let summary = format!("pinch {fingers}: scale={scale:.2} angle={angle:.0}°");
                if !cancelled && !*repeated {
                    // Whichever of zoom and rotation went further past its threshold wins.
                    let zoom_strength = (*scale - 1.0).abs() / t.pinch;
                    let spin_strength = angle.abs() / t.rotate;
                    let gesture = if zoom_strength >= spin_strength {
                        zoom(*scale, t.pinch).map(|z| Gesture::Pinch(*fingers, z))
                    } else {
                        Some(Gesture::Rotate(*fingers, spin(*angle)))
                    };
                    if let Some(gesture) =
                        gesture.filter(|_| zoom_strength.max(spin_strength) >= 1.0)
                    {
                        out.push(Output::Fire(gesture));
                    }
                }
                out.push(Output::Ended(with_cancel(summary, cancelled)));
                self.state = State::Idle;
            }
            (Raw::HoldBegin(fingers), _) => self.state = State::Hold { fingers },
            (Raw::HoldEnd { cancelled }, State::Hold { fingers }) => {
                if !cancelled {
                    out.push(Output::Fire(Gesture::Hold(*fingers)));
                }
                out.push(Output::Ended(with_cancel(
                    format!("hold {fingers}"),
                    cancelled,
                )));
                self.state = State::Idle;
            }
            // An update or end without its begin (e.g. we started mid-gesture).
            _ => {}
        }
        out
    }
}

/// The main direction of `(dx, dy)` and the travel along it.
fn dominant((dx, dy): (f64, f64)) -> Option<(Dir, f64)> {
    if dx == 0.0 && dy == 0.0 {
        None
    } else if dx.abs() >= dy.abs() {
        Some((if dx < 0.0 { Dir::Left } else { Dir::Right }, dx.abs()))
    } else {
        Some((if dy < 0.0 { Dir::Up } else { Dir::Down }, dy.abs()))
    }
}

fn zoom(ratio: f64, threshold: f64) -> Option<Zoom> {
    if ratio <= 1.0 - threshold {
        Some(Zoom::In)
    } else if ratio >= 1.0 + threshold {
        Some(Zoom::Out)
    } else {
        None
    }
}

fn spin(angle: f64) -> Spin {
    if angle >= 0.0 {
        Spin::Cw
    } else {
        Spin::Ccw
    }
}

fn with_cancel(summary: String, cancelled: bool) -> String {
    if cancelled {
        format!("{summary} (cancelled)")
    } else {
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> Config {
        ty_common::config::parse(yaml).unwrap()
    }

    fn fired(cfg: &Config, events: &[Raw]) -> Vec<Gesture> {
        let mut r = Recognizer::default();
        events
            .iter()
            .flat_map(|&e| r.feed(cfg, e))
            .filter_map(|o| match o {
                Output::Fire(g) => Some(g),
                Output::Ended(_) => None,
            })
            .collect()
    }

    fn swipe(fingers: u32, steps: usize, dx: f64, dy: f64, cancelled: bool) -> Vec<Raw> {
        let mut events = vec![Raw::SwipeBegin(fingers)];
        events.extend(std::iter::repeat_n(Raw::SwipeUpdate { dx, dy }, steps));
        events.push(Raw::SwipeEnd { cancelled });
        events
    }

    #[test]
    fn swipe_fires_once_at_end_past_threshold() {
        let c = cfg("");
        assert_eq!(
            fired(&c, &swipe(3, 10, -30.0, 4.0, false)),
            [Gesture::Swipe(3, Dir::Left)]
        );
        assert_eq!(
            fired(&c, &swipe(4, 10, 1.0, 25.0, false)),
            [Gesture::Swipe(4, Dir::Down)]
        );
    }

    #[test]
    fn short_or_cancelled_swipes_do_nothing() {
        let c = cfg("");
        assert!(fired(&c, &swipe(3, 3, -30.0, 0.0, false)).is_empty());
        assert!(fired(&c, &swipe(3, 10, -30.0, 0.0, true)).is_empty());
    }

    #[test]
    fn repeat_swipe_fires_per_step_and_not_at_end() {
        let c = cfg("swipe: { 3: { up: { command: x, repeat: true } } }");
        // 500 units up at the default 120 step: 4 firings.
        let got = fired(&c, &swipe(3, 10, 0.0, -50.0, false));
        assert_eq!(got, vec![Gesture::Swipe(3, Dir::Up); 4]);
    }

    #[test]
    fn repeat_swipe_can_reverse() {
        let c = cfg("swipe: { 3: { up: { command: u, repeat: true }, down: { command: d, repeat: true } } }");
        let mut events = vec![Raw::SwipeBegin(3)];
        events.extend(
            [Raw::SwipeUpdate {
                dx: 0.0,
                dy: -130.0,
            }; 2],
        );
        events.extend([Raw::SwipeUpdate { dx: 0.0, dy: 130.0 }; 2]);
        events.push(Raw::SwipeEnd { cancelled: false });
        let got = fired(&c, &events);
        assert_eq!(
            got,
            [
                Gesture::Swipe(3, Dir::Up),
                Gesture::Swipe(3, Dir::Up),
                Gesture::Swipe(3, Dir::Down),
                Gesture::Swipe(3, Dir::Down),
            ]
        );
    }

    #[test]
    fn pinch_and_rotate_pick_the_stronger_motion() {
        let c = cfg("");
        let pinch = |scale: f64, angle: f64| {
            vec![
                Raw::PinchBegin(2),
                Raw::PinchUpdate { scale, angle },
                Raw::PinchEnd { cancelled: false },
            ]
        };
        assert_eq!(fired(&c, &pinch(0.5, 5.0)), [Gesture::Pinch(2, Zoom::In)]);
        assert_eq!(fired(&c, &pinch(1.6, 0.0)), [Gesture::Pinch(2, Zoom::Out)]);
        assert_eq!(
            fired(&c, &pinch(1.05, -60.0)),
            [Gesture::Rotate(2, Spin::Ccw)]
        );
        assert!(fired(&c, &pinch(1.05, 3.0)).is_empty());
    }

    #[test]
    fn repeat_rotate_works_like_a_knob() {
        let c = cfg("rotate: { 2: { cw: { command: up, repeat: true } } }");
        let mut events = vec![Raw::PinchBegin(2)];
        events.extend(
            [Raw::PinchUpdate {
                scale: 1.0,
                angle: 10.0,
            }; 8],
        );
        events.push(Raw::PinchEnd { cancelled: false });
        // 80° at the default 25° step: 3 firings.
        assert_eq!(fired(&c, &events), vec![Gesture::Rotate(2, Spin::Cw); 3]);
    }

    #[test]
    fn hold_fires_unless_cancelled() {
        let c = cfg("");
        assert_eq!(
            fired(&c, &[Raw::HoldBegin(4), Raw::HoldEnd { cancelled: false }]),
            [Gesture::Hold(4)]
        );
        assert!(fired(&c, &[Raw::HoldBegin(3), Raw::HoldEnd { cancelled: true }]).is_empty());
    }
}
