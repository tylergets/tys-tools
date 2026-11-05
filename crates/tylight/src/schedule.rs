//! The schedule (what the sun alone asks for), and a short history of what was applied,
//! for the graph.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::Setting;
use crate::config::{AppConfig, Stop, APP_NAME};
use crate::protocol::Period;
use crate::solar::{self, Location};

/// What the schedule asks for at some moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheduled {
    pub setting: Setting,
    pub period: Period,
    /// Name of the closest stop.
    pub stage: Option<String>,
    pub elevation: Option<f64>,
}

/// The scheduled setting at `unix`.
pub fn scheduled(config: &AppConfig, location: Option<Location>, unix: f64) -> Scheduled {
    let Some(location) = location else {
        return Scheduled {
            setting: config
                .schedule
                .first()
                .map_or(Setting::NEUTRAL, Stop::setting),
            period: Period::Unknown,
            stage: None,
            elevation: None,
        };
    };
    let elevation = solar::elevation(unix, location);
    let progress = solar::day_progress(elevation, &config.schedule);
    let period = match progress {
        p if p >= 1.0 => Period::Day,
        p if p <= 0.0 => Period::Night,
        _ => Period::Transition,
    };
    let stage = config
        .schedule
        .iter()
        .min_by(|a, b| {
            (a.elevation - elevation)
                .abs()
                .total_cmp(&(b.elevation - elevation).abs())
        })
        .and_then(|stop| stop.name.clone());
    Scheduled {
        setting: at_elevation(&config.schedule, elevation),
        period,
        stage,
        elevation: Some(elevation),
    }
}

/// The setting at `elevation`: the first stop above the highest, the last below the
/// lowest, and blended between the two stops around it.
pub fn at_elevation(stops: &[Stop], elevation: f64) -> Setting {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Setting::NEUTRAL;
    };
    if elevation >= first.elevation {
        return first.setting();
    }
    for pair in stops.windows(2) {
        let (above, below) = (&pair[0], &pair[1]);
        if elevation >= below.elevation {
            let t = (elevation - below.elevation) / (above.elevation - below.elevation);
            return below.setting().lerp(above.setting(), t);
        }
    }
    last.setting()
}

/// One minute of history.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub unix: f64,
    pub temperature: u32,
    pub brightness: f64,
    pub monitor: Option<u8>,
    pub lux: Option<f64>,
}

/// The last day of samples, saved now and then so restarts keep the graph.
#[derive(Debug, Default)]
pub struct History {
    samples: VecDeque<Sample>,
    unsaved: usize,
}

const SAMPLE_INTERVAL: f64 = 60.0;
const KEEP_SECONDS: f64 = 24.0 * 3600.0;
const SAVE_EVERY: usize = 5;

impl History {
    pub fn load() -> Self {
        let samples = path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<Vec<Sample>>(&raw).ok())
            .unwrap_or_default();
        Self {
            samples: samples.into(),
            unsaved: 0,
        }
    }

    pub fn samples(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }

    /// Record `sample` if a minute has passed since the last one.
    pub fn record(&mut self, sample: Sample) {
        if self.push(sample) {
            self.unsaved += 1;
            if self.unsaved >= SAVE_EVERY {
                self.save();
            }
        }
    }

    /// Add `sample` (at most one a minute) and drop what's older than a day.
    fn push(&mut self, sample: Sample) -> bool {
        if self
            .samples
            .back()
            .is_some_and(|last| sample.unix - last.unix < SAMPLE_INTERVAL)
        {
            return false;
        }
        self.samples.push_back(sample);
        while self
            .samples
            .front()
            .is_some_and(|first| sample.unix - first.unix > KEEP_SECONDS)
        {
            self.samples.pop_front();
        }
        true
    }

    pub fn save(&mut self) {
        self.unsaved = 0;
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let samples: Vec<&Sample> = self.samples.iter().collect();
        if let Ok(json) = serde_json::to_string(&samples) {
            let _ = fs::write(path, json);
        }
    }
}

fn path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .map(|dir| dir.join(APP_NAME).join("history.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(unix: f64) -> Sample {
        Sample {
            unix,
            temperature: 6500,
            brightness: 1.0,
            monitor: None,
            lux: None,
        }
    }

    #[test]
    fn records_once_a_minute_and_keeps_a_day() {
        let mut history = History::default();
        // Every 30 s for 26 hours.
        for i in 0..(26 * 120) {
            history.push(sample(f64::from(i) * 30.0));
        }
        let len = history.samples.len();
        assert!((24 * 60..=24 * 60 + 1).contains(&len), "{len}");
    }

    #[test]
    fn schedule_without_location_is_day() {
        let config = AppConfig::default();
        let now = scheduled(&config, None, 0.0);
        assert_eq!(now.setting.temperature, 6500.0);
        assert_eq!(now.period, Period::Unknown);
        assert!(now.elevation.is_none());
    }

    #[test]
    fn blends_between_stops() {
        let stops = AppConfig::default().schedule;
        let kelvin = |elevation| at_elevation(&stops, elevation).temperature;
        assert_eq!(kelvin(40.0), 6500.0);
        assert_eq!(kelvin(8.0), 6500.0);
        assert_eq!(kelvin(5.0), 6000.0);
        assert_eq!(kelvin(2.0), 5500.0);
        assert_eq!(kelvin(-8.0), 4000.0);
        assert_eq!(kelvin(-11.0), 3800.0);
        assert_eq!(kelvin(-40.0), 3600.0);
        // Getting warmer the whole way down.
        let mut last = f64::MAX;
        for tenth in (-200..=100).rev() {
            let k = kelvin(f64::from(tenth) / 10.0);
            assert!(k <= last);
            last = k;
        }
    }

    #[test]
    fn names_the_closest_stage() {
        let config = AppConfig::default();
        let greenwich = Location {
            latitude: 51.48,
            longitude: 0.0,
        };
        // 2024-06-21 12:00 UTC (sun ~62° up) and midnight before it.
        let noon = scheduled(&config, Some(greenwich), 1718971200.0);
        assert_eq!(noon.stage.as_deref(), Some("day"));
        assert_eq!(noon.period, Period::Day);
        let midnight = scheduled(&config, Some(greenwich), 1718971200.0 - 12.0 * 3600.0);
        assert_eq!(midnight.stage.as_deref(), Some("night"));
    }
}
