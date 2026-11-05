//! The day/night schedule (what the sun alone asks for), and a short history of what was
//! applied, for the graph.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::Setting;
use crate::config::{AppConfig, APP_NAME};
use crate::protocol::Period;
use crate::solar::{self, Location};

/// The scheduled setting at `unix`, its period and the sun elevation.
pub fn scheduled(
    config: &AppConfig,
    location: Option<Location>,
    unix: f64,
) -> (Setting, Period, Option<f64>) {
    let day = Setting {
        temperature: f64::from(config.temperature.day),
        brightness: config.brightness.day,
    };
    let night = Setting {
        temperature: f64::from(config.temperature.night),
        brightness: config.brightness.night,
    };
    let Some(location) = location else {
        return (day, Period::Unknown, None);
    };
    let elevation = solar::elevation(unix, location);
    let progress = solar::day_progress(elevation, &config.elevation);
    let period = match progress {
        p if p >= 1.0 => Period::Day,
        p if p <= 0.0 => Period::Night,
        _ => Period::Transition,
    };
    (night.lerp(day, progress), period, Some(elevation))
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
        let (setting, period, elevation) = scheduled(&config, None, 0.0);
        assert_eq!(setting.temperature, 6500.0);
        assert_eq!(period, Period::Unknown);
        assert!(elevation.is_none());
    }
}
