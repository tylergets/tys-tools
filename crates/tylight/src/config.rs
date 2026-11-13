use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use ty_common::config::Loaded;

use crate::solar::Location;

pub const APP_NAME: &str = "tylight";

/// Lowest and highest supported color temperatures, in kelvin.
pub const MIN_TEMPERATURE: u32 = 1700;
pub const MAX_TEMPERATURE: u32 = 25000;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Settings by sun elevation, from highest to lowest, blended in between.
    pub schedule: Vec<Stop>,
    /// Gamma correction applied to every channel.
    pub gamma: f64,
    /// Fixed location. When unset, GeoClue2 is asked (and the last fix is cached).
    pub location: Option<Location>,
    /// Length of the fade when tylight is toggled or the temperature is set, in seconds.
    /// 0 switches instantly.
    pub fade_seconds: f64,
    pub backend: BackendKind,
    pub tray: bool,
    /// Pause night colors while any of these process names/executable basenames run.
    pub app_whitelist: Vec<String>,
    pub monitor_brightness: MonitorBrightness,
    /// Read the ambient light sensor (if any) and show its value in the tray.
    pub ambient_light: bool,
    /// Set the monitor brightness from the ambient light sensor.
    pub auto_brightness: AutoBrightness,
    /// Named settings, picked from the tray or with `tylight preset <name>`.
    pub presets: BTreeMap<String, Preset>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schedule: default_schedule(),
            gamma: 1.0,
            location: None,
            fade_seconds: 2.0,
            backend: BackendKind::Auto,
            tray: true,
            app_whitelist: Vec::new(),
            monitor_brightness: MonitorBrightness::default(),
            ambient_light: true,
            auto_brightness: AutoBrightness::default(),
            presets: default_presets(),
        }
    }
}

fn default_schedule() -> Vec<Stop> {
    let stop = |name: &str, elevation, temperature| Stop {
        name: Some(name.to_string()),
        elevation,
        temperature,
        brightness: 1.0,
    };
    vec![
        stop("day", 8.0, 6500),
        stop("golden hour", 2.0, 5500),
        stop("sunset", -3.0, 4600),
        stop("dusk", -8.0, 4000),
        stop("night", -14.0, 3600),
    ]
}

fn default_presets() -> BTreeMap<String, Preset> {
    let preset = |temperature, brightness, monitor_brightness| Preset {
        temperature: Some(temperature),
        brightness,
        monitor_brightness,
    };
    BTreeMap::from([
        ("reading".to_string(), preset(5000, None, None)),
        ("evening".to_string(), preset(3400, Some(0.9), None)),
        ("late-night".to_string(), preset(2400, Some(0.75), Some(15))),
    ])
}

/// A named setting. Fields left out follow the schedule.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default)]
    pub temperature: Option<u32>,
    /// Gamma dimming, 0.1-1.0.
    #[serde(default)]
    pub brightness: Option<f64>,
    /// Hardware brightness, in percent (set once when the preset is picked).
    #[serde(default)]
    pub monitor_brightness: Option<u8>,
}

/// Ambient light to monitor brightness, tuned not to jump around: the light level is
/// smoothed (in log scale), small changes are ignored, and brightening reacts faster than
/// dimming.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoBrightness {
    /// Start with auto brightness on (it can be toggled at runtime).
    pub enabled: bool,
    /// `[lux, percent]` points, interpolated in log-lux.
    pub curve: Vec<[f64; 2]>,
    /// Time constant of the light smoothing, in seconds.
    pub smoothing_seconds: f64,
    /// Relative light change (0.25 = 25%) that is ignored.
    pub hysteresis: f64,
    /// How long the light must stay brighter before the screen brightens.
    pub brighten_delay: f64,
    /// How long the light must stay darker before the screen dims.
    pub dim_delay: f64,
    /// Fade length of automatic changes, in seconds.
    pub fade_seconds: f64,
    /// Also adjust external monitors over DDC (slow; off by default).
    pub ddc: bool,
}

impl Default for AutoBrightness {
    fn default() -> Self {
        Self {
            enabled: false,
            curve: vec![
                [0.0, 10.0],
                [10.0, 30.0],
                [100.0, 55.0],
                [1000.0, 85.0],
                [10000.0, 100.0],
            ],
            smoothing_seconds: 4.0,
            hysteresis: 0.25,
            brighten_delay: 3.0,
            dim_delay: 15.0,
            fade_seconds: 1.5,
            ddc: false,
        }
    }
}

/// One step of the schedule: the setting once the sun is at `elevation` degrees.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Stop {
    /// Shown in the status ("4600K · sunset").
    #[serde(default)]
    pub name: Option<String>,
    pub elevation: f64,
    pub temperature: u32,
    /// Gamma dimming, 0.1-1.0 (1.0 = none). Separate from the monitors' own brightness.
    #[serde(default = "full_brightness")]
    pub brightness: f64,
}

fn full_brightness() -> f64 {
    1.0
}

impl Stop {
    pub fn setting(&self) -> crate::color::Setting {
        crate::color::Setting {
            temperature: f64::from(self.temperature),
            brightness: self.brightness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// wlroots gamma control when `$WAYLAND_DISPLAY` is set, otherwise X11 RandR.
    Auto,
    X11,
    Wayland,
}

/// Hardware brightness of the monitors (not the gamma dimming above).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorBrightness {
    /// Laptop panels in /sys/class/backlight (written through logind).
    pub backlight: bool,
    /// External monitors over DDC/CI, through `ddcutil`.
    pub ddc: bool,
    /// Percent used by `brightness up` / `down` and the tray.
    pub step: u8,
}

impl Default for MonitorBrightness {
    fn default() -> Self {
        Self {
            backlight: true,
            ddc: true,
            step: 10,
        }
    }
}

impl AppConfig {
    pub fn load(path: Option<PathBuf>) -> Result<Loaded<Self>> {
        ty_common::config::load(APP_NAME, path)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schedule.is_empty() {
            bail!("schedule needs at least one stop");
        }
        for (i, stop) in self.schedule.iter().enumerate() {
            validate_temperature(&format!("schedule[{i}].temperature"), stop.temperature)?;
            if !(0.1..=1.0).contains(&stop.brightness) {
                bail!(
                    "schedule[{i}].brightness must be between 0.1 and 1.0 (got {})",
                    stop.brightness
                );
            }
            if !(-90.0..=90.0).contains(&stop.elevation) {
                bail!("schedule[{i}].elevation must be between -90 and 90");
            }
        }
        if self
            .schedule
            .windows(2)
            .any(|pair| pair[1].elevation >= pair[0].elevation)
        {
            bail!("schedule elevations must go from highest (day) to lowest (night)");
        }
        if !(0.1..=10.0).contains(&self.gamma) {
            bail!("gamma must be between 0.1 and 10.0 (got {})", self.gamma);
        }
        if let Some(location) = &self.location {
            location.validate()?;
        }
        if !(0.0..=60.0).contains(&self.fade_seconds) {
            bail!("fade_seconds must be between 0 and 60");
        }
        if !(1..=100).contains(&self.monitor_brightness.step) {
            bail!("monitor_brightness.step must be between 1 and 100");
        }
        for app in &self.app_whitelist {
            if app.is_empty()
                || app.trim() != app
                || app.contains('/')
                || app.chars().any(char::is_control)
            {
                bail!("app_whitelist entries must be process names or executable basenames");
            }
        }
        self.auto_brightness.validate()?;
        for (name, preset) in &self.presets {
            if name.is_empty() || name.contains(char::is_whitespace) {
                bail!("preset names can't be empty or contain spaces ('{name}')");
            }
            preset
                .validate()
                .with_context(|| format!("invalid preset '{name}'"))?;
        }
        Ok(())
    }
}

impl Preset {
    pub fn validate(&self) -> Result<()> {
        if self.temperature.is_none()
            && self.brightness.is_none()
            && self.monitor_brightness.is_none()
        {
            bail!("set at least one of temperature, brightness, monitor_brightness");
        }
        if let Some(kelvin) = self.temperature {
            validate_temperature("temperature", kelvin)?;
        }
        if let Some(value) = self.brightness {
            if !(0.1..=1.0).contains(&value) {
                bail!("brightness must be between 0.1 and 1.0");
            }
        }
        if self.monitor_brightness.is_some_and(|p| p > 100) {
            bail!("monitor_brightness must be between 0 and 100");
        }
        Ok(())
    }
}

impl AutoBrightness {
    pub fn validate(&self) -> Result<()> {
        if self.curve.len() < 2 {
            bail!("auto_brightness.curve needs at least two points");
        }
        for pair in self.curve.windows(2) {
            if pair[1][0] <= pair[0][0] {
                bail!("auto_brightness.curve lux values must increase");
            }
        }
        for [lux, percent] in &self.curve {
            if *lux < 0.0 || !(0.0..=100.0).contains(percent) {
                bail!("auto_brightness.curve points are [lux >= 0, percent 0-100]");
            }
        }
        for (name, value) in [
            ("smoothing_seconds", self.smoothing_seconds),
            ("brighten_delay", self.brighten_delay),
            ("dim_delay", self.dim_delay),
            ("fade_seconds", self.fade_seconds),
        ] {
            if !(0.0..=600.0).contains(&value) {
                bail!("auto_brightness.{name} must be between 0 and 600");
            }
        }
        if !(0.0..=10.0).contains(&self.hysteresis) {
            bail!("auto_brightness.hysteresis must be between 0 and 10");
        }
        Ok(())
    }
}

pub fn validate_temperature(name: &str, kelvin: u32) -> Result<()> {
    if !(MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&kelvin) {
        bail!("{name} must be between {MIN_TEMPERATURE}K and {MAX_TEMPERATURE}K (got {kelvin}K)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let config: AppConfig = ty_common::config::parse("").unwrap();
        assert_eq!(config, AppConfig::default());
        config.validate().unwrap();
    }

    #[test]
    fn parses_partial_sections_and_location() {
        let config: AppConfig = ty_common::config::parse(
            "schedule: [{elevation: 3, temperature: 6500}, {elevation: -6, temperature: 3000, brightness: 0.8}]\nlocation: {latitude: 40.7, longitude: -74.0}\nbackend: x11\n",
        )
        .unwrap();
        assert_eq!(config.schedule.len(), 2);
        assert_eq!(config.schedule[0].brightness, 1.0);
        assert_eq!(config.schedule[1].temperature, 3000);
        assert_eq!(config.schedule[1].name, None);
        assert_eq!(config.location.unwrap().latitude, 40.7);
        assert_eq!(config.backend, BackendKind::X11);
        config.validate().unwrap();
    }

    #[test]
    fn rejects_bad_values() {
        let bad = |yaml: &str| {
            ty_common::config::parse::<AppConfig>(yaml)
                .unwrap()
                .validate()
                .is_err()
        };
        assert!(bad("schedule: [{elevation: 0, temperature: 500}]"));
        assert!(bad(
            "schedule: [{elevation: 0, temperature: 4000, brightness: 0}]"
        ));
        assert!(bad("schedule: []"));
        assert!(bad(
            "schedule: [{elevation: -6, temperature: 4000}, {elevation: 3, temperature: 6500}]"
        ));
        assert!(bad("location: {latitude: 91, longitude: 0}"));
        assert!(ty_common::config::parse::<AppConfig>("nope: 1").is_err());
        assert!(bad("presets: {x: {}}"));
        assert!(bad("presets: {x: {temperature: 100}}"));
        assert!(bad("auto_brightness: {curve: [[10, 50], [5, 60]]}"));
        assert!(bad("auto_brightness: {curve: [[10, 50]]}"));
        assert!(bad("app_whitelist: ['']"));
        assert!(bad("app_whitelist: ['/usr/bin/steam']"));
        assert!(bad("app_whitelist: [' steam']"));
    }

    #[test]
    fn presets_replace_the_defaults() {
        let config: AppConfig = ty_common::config::parse(
            "presets: {movie: {temperature: 3200, monitor_brightness: 40}}",
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.presets.len(), 1);
        assert_eq!(config.presets["movie"].brightness, None);
    }
}
