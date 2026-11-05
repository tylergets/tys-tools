//! Ambient light to monitor brightness, without jumping around:
//!
//! 1. Light is smoothed in log scale (eyes respond to ratios), with a time constant.
//! 2. Nothing happens while the smoothed light stays within a band (`hysteresis`) around the
//!    level of the last adjustment.
//! 3. Leaving the band must last `brighten_delay` (short) or `dim_delay` (long), so a hand or
//!    a passing shadow over the sensor does nothing.
//! 4. Manual changes become an offset on the curve instead of being undone.

use std::time::Instant;

use crate::config::AutoBrightness;

/// Changes smaller than this (percent points) aren't worth a fade.
const MIN_CHANGE: f64 = 2.0;
const MAX_OFFSET: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Brighter,
    Darker,
}

#[derive(Debug)]
struct Pending {
    direction: Direction,
    since: Instant,
    /// Sum and count of raw log-lux since then, to aim at their average.
    sum: f64,
    count: u32,
}

#[derive(Debug)]
pub struct Adaptive {
    config: AutoBrightness,
    pub enabled: bool,
    /// Smoothed `log10(lux + 1)`.
    smoothed: Option<f64>,
    last_tick: Option<Instant>,
    /// Smoothed level at the last adjustment.
    anchor: Option<f64>,
    /// The light has left the band since then, in this direction.
    pending: Option<Pending>,
    /// Percent points added to the curve, from manual changes.
    offset: f64,
    /// The brightness tylight last asked for.
    last_target: Option<u8>,
}

fn to_log(lux: f64) -> f64 {
    (lux.max(0.0) + 1.0).log10()
}

fn from_log(x: f64) -> f64 {
    10f64.powf(x) - 1.0
}

impl Adaptive {
    pub fn new(config: AutoBrightness) -> Self {
        Self {
            enabled: config.enabled,
            config,
            smoothed: None,
            last_tick: None,
            anchor: None,
            pending: None,
            offset: 0.0,
            last_target: None,
        }
    }

    /// The curve's brightness (percent) for `lux`, linear in log-lux.
    pub fn curve(&self, lux: f64) -> f64 {
        let x = to_log(lux);
        let points = &self.config.curve;
        let first = points[0];
        let last = points[points.len() - 1];
        if x <= to_log(first[0]) {
            return first[1];
        }
        if x >= to_log(last[0]) {
            return last[1];
        }
        points
            .windows(2)
            .find(|pair| x <= to_log(pair[1][0]))
            .map(|pair| {
                let (x0, x1) = (to_log(pair[0][0]), to_log(pair[1][0]));
                pair[0][1] + (pair[1][1] - pair[0][1]) * (x - x0) / (x1 - x0)
            })
            .unwrap_or(last[1])
    }

    #[cfg(test)]
    pub fn smoothed_lux(&self) -> Option<f64> {
        self.smoothed.map(from_log)
    }

    #[cfg(test)]
    pub fn offset(&self) -> f64 {
        self.offset
    }

    /// The brightness tylight last asked for (to notice changes made elsewhere).
    pub fn last_target(&self) -> Option<u8> {
        self.last_target
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        // Adjust right away when turned on.
        self.anchor = None;
        self.pending = None;
        self.last_target = None;
    }

    /// Feed a sensor reading. Returns a brightness to fade to, if it's time to change.
    pub fn update(&mut self, lux: f64, now: Instant) -> Option<u8> {
        let x = to_log(lux);
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);
        self.last_tick = Some(now);
        let tau = self.config.smoothing_seconds;
        let smoothed = match self.smoothed {
            Some(s) if tau > 0.0 => s + (x - s) * (1.0 - (-dt / tau).exp()),
            _ => x,
        };
        self.smoothed = Some(smoothed);
        if !self.enabled {
            return None;
        }

        let Some(anchor) = self.anchor else {
            return self.adjust(smoothed);
        };
        let band = (1.0 + self.config.hysteresis).log10();
        let side = |level: f64| {
            if level > anchor + band {
                Some(Direction::Brighter)
            } else if level < anchor - band {
                Some(Direction::Darker)
            } else {
                None
            }
        };
        // Both the smoothed and the raw light must be past the band, so the smoothed
        // value catching up after a shadow doesn't count as staying dark.
        let direction = match (side(smoothed), side(x)) {
            (Some(smoothed), Some(raw)) if smoothed == raw => smoothed,
            _ => {
                self.pending = None;
                return None;
            }
        };
        let delay = match direction {
            Direction::Brighter => self.config.brighten_delay,
            Direction::Darker => self.config.dim_delay,
        };
        match &mut self.pending {
            Some(pending) if pending.direction == direction => {
                pending.sum += x;
                pending.count += 1;
                if now.duration_since(pending.since).as_secs_f64() >= delay {
                    // Aim at where the light settled, in one step, rather than
                    // following the slow average up in a staircase.
                    let level = pending.sum / f64::from(pending.count);
                    self.smoothed = Some(level);
                    return self.adjust(level);
                }
            }
            _ => {
                self.pending = Some(Pending {
                    direction,
                    since: now,
                    sum: x,
                    count: 1,
                })
            }
        }
        None
    }

    fn adjust(&mut self, smoothed: f64) -> Option<u8> {
        self.anchor = Some(smoothed);
        self.pending = None;
        let target = (self.curve(from_log(smoothed)) + self.offset).clamp(1.0, 100.0);
        if self
            .last_target
            .is_some_and(|last| (f64::from(last) - target).abs() < MIN_CHANGE)
        {
            return None;
        }
        let target = target.round() as u8;
        self.last_target = Some(target);
        Some(target)
    }

    /// The user picked `percent` by hand: keep that relation to the light from now on.
    pub fn on_manual(&mut self, percent: u8) {
        self.last_target = Some(percent);
        if let Some(smoothed) = self.smoothed {
            self.offset = (f64::from(percent) - self.curve(from_log(smoothed)))
                .clamp(-MAX_OFFSET, MAX_OFFSET);
            self.anchor = Some(smoothed);
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn adaptive() -> Adaptive {
        Adaptive::new(AutoBrightness {
            enabled: true,
            ..AutoBrightness::default()
        })
    }

    /// Feed `lux` once a second for `secs`; collect the changes.
    fn feed(a: &mut Adaptive, start: &mut Instant, secs: u64, lux: impl Fn(u64) -> f64) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..secs {
            *start += Duration::from_secs(1);
            out.extend(a.update(lux(i), *start));
        }
        out
    }

    #[test]
    fn curve_interpolates_in_log_lux() {
        let a = adaptive();
        assert_eq!(a.curve(0.0), 10.0);
        assert!((a.curve(100.0) - 55.0).abs() < 1e-9);
        assert_eq!(a.curve(50000.0), 100.0);
        let mid = a.curve(31.0);
        assert!(mid > 30.0 && mid < 55.0, "{mid}");
    }

    #[test]
    fn first_reading_sets_brightness() {
        let mut a = adaptive();
        assert_eq!(a.update(100.0, Instant::now()), Some(55));
    }

    #[test]
    fn flicker_is_ignored() {
        let mut a = adaptive();
        let mut now = Instant::now();
        a.update(100.0, now);
        let changes = feed(&mut a, &mut now, 120, |i| {
            if i % 2 == 0 {
                100.0
            } else {
                140.0
            }
        });
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn brightens_quickly() {
        let mut a = adaptive();
        let mut now = Instant::now();
        a.update(100.0, now);
        let changes = feed(&mut a, &mut now, 6, |_| 1000.0);
        assert!(!changes.is_empty(), "brightens within 6 s");
        assert!(changes[0] > 55);
        let later = feed(&mut a, &mut now, 30, |_| 1000.0);
        assert!(later.is_empty(), "one step: {changes:?} then {later:?}");
        assert_eq!(changes, vec![85]);
    }

    #[test]
    fn dims_slowly_and_ignores_short_shadows() {
        let mut a = adaptive();
        let mut now = Instant::now();
        a.update(1000.0, now);
        // A 6 second shadow: nothing.
        assert!(feed(&mut a, &mut now, 6, |_| 5.0).is_empty());
        assert!(feed(&mut a, &mut now, 30, |_| 1000.0).is_empty());
        // Darker for good: dims, but not within the first 10 seconds.
        assert!(feed(&mut a, &mut now, 10, |_| 20.0).is_empty());
        let changes = feed(&mut a, &mut now, 30, |_| 20.0);
        assert!(!changes.is_empty());
        assert!(*changes.last().unwrap() < 85);
    }

    #[test]
    fn manual_changes_become_an_offset() {
        let mut a = adaptive();
        let mut now = Instant::now();
        a.update(100.0, now);
        a.on_manual(75); // curve says 55
        assert!((a.offset() - 20.0).abs() < 1.0);
        let changes = feed(&mut a, &mut now, 20, |_| 1000.0);
        assert_eq!(changes, vec![100]); // 85 + 20, capped
    }

    #[test]
    fn disabled_only_tracks_light() {
        let mut a = Adaptive::new(AutoBrightness::default());
        assert_eq!(a.update(100.0, Instant::now()), None);
        assert!(a.smoothed_lux().is_some());
        a.set_enabled(true);
        assert!(a.update(100.0, Instant::now()).is_some());
    }
}
