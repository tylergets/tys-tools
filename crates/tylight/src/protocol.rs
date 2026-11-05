//! Commands and status exchanged with the daemon over its socket (one line each way).

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use ty_common::status::Report;

use crate::monitors::Change;
use crate::solar::Location;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Status,
    Enable,
    /// Disable, optionally only for this many seconds.
    Disable(Option<u64>),
    /// Disable until the day setting starts again.
    DisableUntilMorning,
    Toggle,
    /// Hold a fixed temperature until `Auto`.
    Set(u32),
    Auto,
    /// Apply a named preset until `Auto`.
    Preset(String),
    Brightness(Change),
    /// Turn auto brightness on/off (`None` toggles).
    AutoBrightness(Option<bool>),
    /// Open the graph window.
    ShowGraph,
}

impl Command {
    pub fn to_line(&self) -> String {
        match self {
            Command::Status => "status".into(),
            Command::Enable => "enable".into(),
            Command::Disable(None) => "disable".into(),
            Command::Disable(Some(secs)) => format!("disable {secs}"),
            Command::DisableUntilMorning => "disable-until-morning".into(),
            Command::Toggle => "toggle".into(),
            Command::Set(kelvin) => format!("set {kelvin}"),
            Command::Auto => "auto".into(),
            Command::Preset(name) => format!("preset {name}"),
            Command::Brightness(change) => format!("brightness {}", change.to_arg()),
            Command::AutoBrightness(None) => "auto-brightness toggle".into(),
            Command::AutoBrightness(Some(true)) => "auto-brightness on".into(),
            Command::AutoBrightness(Some(false)) => "auto-brightness off".into(),
            Command::ShowGraph => "graph".into(),
        }
    }

    pub fn parse(line: &str) -> Result<Self> {
        let mut words = line.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let arg = words.next();
        let number = |what: &str| -> Result<u64> {
            arg.with_context(|| format!("{verb} needs {what}"))?
                .parse()
                .with_context(|| format!("invalid {what}"))
        };
        Ok(match verb {
            "status" => Command::Status,
            "enable" => Command::Enable,
            "disable" if arg.is_none() => Command::Disable(None),
            "disable" => Command::Disable(Some(number("seconds")?)),
            "disable-until-morning" => Command::DisableUntilMorning,
            "toggle" => Command::Toggle,
            "set" => Command::Set(number("a temperature")? as u32),
            "auto" => Command::Auto,
            "brightness" => {
                Command::Brightness(Change::parse(arg.context("brightness needs a value")?)?)
            }
            "preset" => Command::Preset(arg.context("preset needs a name")?.to_string()),
            "auto-brightness" => Command::AutoBrightness(parse_switch(arg)?),
            "graph" => Command::ShowGraph,
            other => bail!("unknown command '{other}'"),
        })
    }
}

/// `on`, `off`, `toggle` (or nothing): `Some(true)`, `Some(false)`, `None`.
pub fn parse_switch(arg: Option<&str>) -> Result<Option<bool>> {
    match arg.unwrap_or("toggle") {
        "on" | "true" | "1" => Ok(Some(true)),
        "off" | "false" | "0" => Ok(Some(false)),
        "toggle" => Ok(None),
        other => bail!("expected on, off or toggle (got '{other}')"),
    }
}

/// `90`, `90s`, `30m`, `1h`, `1h30m`.
pub fn parse_duration(text: &str) -> Result<u64> {
    let text = text.trim();
    if let Ok(secs) = text.parse::<u64>() {
        return Ok(secs);
    }
    let mut total = 0;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => bail!("invalid duration '{text}' (use e.g. 30m, 1h, 1h30m)"),
        };
        let value: u64 = digits
            .parse()
            .with_context(|| format!("invalid duration '{text}'"))?;
        total += value * unit;
        digits.clear();
    }
    if !digits.is_empty() || total == 0 {
        bail!("invalid duration '{text}' (use e.g. 30m, 1h, 1h30m)");
    }
    Ok(total)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    Day,
    Night,
    Transition,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub enabled: bool,
    /// Unix seconds when tylight turns itself back on.
    pub disabled_until: Option<u64>,
    /// Temperature held by `set`.
    pub manual: Option<u32>,
    /// Active preset.
    pub preset: Option<String>,
    pub period: Period,
    /// Name of the schedule stop closest to the sun now ("sunset", "dusk", ...).
    #[serde(default)]
    pub stage: Option<String>,
    /// Applied color temperature (K).
    pub temperature: u32,
    /// Applied gamma dimming.
    pub brightness: f64,
    pub elevation: Option<f64>,
    pub location: Option<Location>,
    /// `config`, `geoclue` or `cache`.
    pub location_source: Option<String>,
    pub backend: String,
    /// Hardware brightness of the monitors, in percent.
    pub monitor_brightness: Option<u8>,
    pub ambient_lux: Option<f64>,
    /// `None` without a light sensor.
    pub auto_brightness: Option<bool>,
}

impl Status {
    pub fn offline() -> Self {
        Self::default()
    }

    /// "42 min left" for a timed disable.
    pub fn remaining(&self) -> Option<String> {
        let until = self.disabled_until?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let mins = until.saturating_sub(now).div_ceil(60);
        Some(if mins >= 60 {
            format!("{}h {:02}m left", mins / 60, mins % 60)
        } else {
            format!("{mins} min left")
        })
    }

    pub fn summary(&self) -> String {
        if !self.enabled {
            return match self.remaining() {
                Some(left) => format!("Off · {left}"),
                None => "Off".into(),
            };
        }
        let what = match (&self.preset, self.manual, self.period) {
            (Some(preset), _, _) => preset.as_str(),
            (None, Some(_), _) => "manual",
            (None, None, Period::Unknown) => "no location yet",
            (None, None, _) if self.stage.is_some() => self.stage.as_deref().unwrap_or_default(),
            (None, None, Period::Day) => "day",
            (None, None, Period::Night) => "night",
            (None, None, Period::Transition) => "transition",
        };
        format!("{}K · {what}", self.temperature)
    }
}

impl Report for Status {
    fn format_plain(&self) -> String {
        if !self.running {
            return "tylight is not running".into();
        }
        let mut line = self.summary();
        if let Some(percent) = self.monitor_brightness {
            let auto = if self.auto_brightness == Some(true) {
                " (auto)"
            } else {
                ""
            };
            line.push_str(&format!(" · monitors {percent}%{auto}"));
        }
        if let Some(lux) = self.ambient_lux {
            line.push_str(&format!(" · {lux:.0} lux"));
        }
        line
    }

    fn format_polybar(&self) -> String {
        match (self.running, self.enabled) {
            (false, _) => String::new(),
            (true, false) => "off".into(),
            (true, true) => format!("{}K", self.temperature),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        for command in [
            Command::Status,
            Command::Enable,
            Command::Disable(None),
            Command::Disable(Some(3600)),
            Command::DisableUntilMorning,
            Command::Toggle,
            Command::Set(3400),
            Command::Auto,
            Command::Brightness(Change::By(-10)),
            Command::Brightness(Change::Set(70)),
            Command::Preset("late-night".into()),
            Command::AutoBrightness(None),
            Command::AutoBrightness(Some(true)),
            Command::AutoBrightness(Some(false)),
            Command::ShowGraph,
        ] {
            assert_eq!(Command::parse(&command.to_line()).unwrap(), command);
        }
        assert!(Command::parse("dance").is_err());
        assert!(Command::parse("set").is_err());
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90").unwrap(), 90);
        assert_eq!(parse_duration("30m").unwrap(), 1800);
        assert_eq!(parse_duration("1h30m").unwrap(), 5400);
        assert!(parse_duration("1x").is_err());
        assert!(parse_duration("1h30").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn formats_status() {
        let status = Status {
            running: true,
            enabled: true,
            period: Period::Night,
            temperature: 4000,
            monitor_brightness: Some(60),
            ..Status::default()
        };
        assert_eq!(status.format_plain(), "4000K · night · monitors 60%");
        let dusk = Status {
            period: Period::Transition,
            stage: Some("dusk".into()),
            ..status.clone()
        };
        assert_eq!(dusk.format_plain(), "4000K · dusk · monitors 60%");
        assert_eq!(status.format_polybar(), "4000K");
        assert_eq!(Status::offline().format_polybar(), "");
    }
}
