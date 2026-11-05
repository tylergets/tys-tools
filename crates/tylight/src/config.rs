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
    pub temperature: Temperature,
    /// Gamma dimming (1.0 = none). Separate from the monitors' own brightness.
    pub brightness: Brightness,
    /// Gamma correction applied to every channel.
    pub gamma: f64,
    /// Fixed location. When unset, GeoClue2 is asked (and the last fix is cached).
    pub location: Option<Location>,
    pub elevation: Elevation,
    /// Length of the fade when tylight is toggled or the temperature is set, in seconds.
    /// 0 switches instantly.
    pub fade_seconds: f64,
    pub backend: BackendKind,
    pub tray: bool,
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
            temperature: Temperature::default(),
            brightness: Brightness::default(),
            gamma: 1.0,
            location: None,
            elevation: Elevation::default(),
            fade_seconds: 2.0,
            backend: BackendKind::Auto,
            tray: true,
            monitor_brightness: MonitorBrightness::default(),
            ambient_light: true,
            auto_brightness: AutoBrightness::default(),
            presets: default_presets(),
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Temperature {
    pub day: u32,
    pub night: u32,
}

impl Default for Temperature {
    fn default() -> Self {
        Self {
            day: 6500,
            night: 4000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Brightness {
    pub day: f64,
    pub night: f64,
}

impl Default for Brightness {
    fn default() -> Self {
        Self {
            day: 1.0,
            night: 1.0,
        }
    }
}

/// Solar elevations (degrees) where the day and night settings start. In between, the
/// settings blend.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Elevation {
    pub high: f64,
    pub low: f64,
}

impl Default for Elevation {
    fn default() -> Self {
        Self {
            high: 3.0,
            low: -6.0,
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
        for (name, value) in [
            ("temperature.day", self.temperature.day),
            ("temperature.night", self.temperature.night),
        ] {
            validate_temperature(name, value)?;
        }
        for (name, value) in [
            ("brightness.day", self.brightness.day),
            ("brightness.night", self.brightness.night),
        ] {
            if !(0.1..=1.0).contains(&value) {
                bail!("{name} must be between 0.1 and 1.0 (got {value})");
            }
        }
        if !(0.1..=10.0).contains(&self.gamma) {
            bail!("gamma must be between 0.1 and 10.0 (got {})", self.gamma);
        }
        if let Some(location) = &self.location {
            location.validate()?;
        }
        if self.elevation.high <= self.elevation.low {
            bail!("elevation.high must be above elevation.low");
        }
        if !(0.0..=60.0).contains(&self.fade_seconds) {
            bail!("fade_seconds must be between 0 and 60");
        }
        if !(1..=100).contains(&self.monitor_brightness.step) {
            bail!("monitor_brightness.step must be between 1 and 100");
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
            "temperature: {night: 3000}\nlocation: {latitude: 40.7, longitude: -74.0}\nbackend: x11\n",
        )
        .unwrap();
        assert_eq!(config.temperature.day, 6500);
        assert_eq!(config.temperature.night, 3000);
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
        assert!(bad("temperature: {night: 500}"));
        assert!(bad("brightness: {night: 0}"));
        assert!(bad("location: {latitude: 91, longitude: 0}"));
        assert!(bad("elevation: {high: -6, low: 3}"));
        assert!(ty_common::config::parse::<AppConfig>("nope: 1").is_err());
        assert!(bad("presets: {x: {}}"));
        assert!(bad("presets: {x: {temperature: 100}}"));
        assert!(bad("auto_brightness: {curve: [[10, 50], [5, 60]]}"));
        assert!(bad("auto_brightness: {curve: [[10, 50]]}"));
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
