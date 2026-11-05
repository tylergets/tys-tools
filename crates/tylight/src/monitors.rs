//! Hardware brightness: laptop backlights (/sys/class/backlight, written through logind so
//! no root or udev rule is needed) and external monitors over DDC/CI (`ddcutil`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::MonitorBrightness;

const BACKLIGHT_DIR: &str = "/sys/class/backlight";
/// VCP feature code for luminance.
const VCP_BRIGHTNESS: &str = "10";
/// How long a manual backlight brightness change fades.
pub const MANUAL_FADE: Duration = Duration::from_millis(250);
/// Backlight fade frame interval.
const BACKLIGHT_FRAME: Duration = Duration::from_millis(12);
#[cfg(test)]
const BACKLIGHT_STEPS: u32 = 20;
/// DDC writes take ~100ms each, so external monitors get only a few steps.
const DDC_STEPS: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
enum Device {
    Backlight { name: String, max: u32 },
    Ddc { display: u32, max: u32 },
}

/// A brightness request: an absolute percent or a relative step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Set(u8),
    By(i32),
}

impl Change {
    /// `50`, `+10`, `-10`.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim().trim_end_matches('%');
        let value = || -> Result<i32> {
            text.trim_start_matches(['+', '-'])
                .parse::<i32>()
                .with_context(|| format!("invalid brightness '{text}'"))
        };
        if text.starts_with('+') {
            Ok(Change::By(value()?))
        } else if text.starts_with('-') {
            Ok(Change::By(-value()?))
        } else {
            let v = value()?;
            if !(0..=100).contains(&v) {
                bail!("brightness must be between 0 and 100");
            }
            Ok(Change::Set(v as u8))
        }
    }

    pub fn to_arg(self) -> String {
        match self {
            Change::Set(v) => v.to_string(),
            Change::By(d) => format!("{d:+}"),
        }
    }

    fn resolve(self, current: Option<u8>) -> u8 {
        match self {
            Change::Set(v) => v,
            Change::By(d) => (i32::from(current.unwrap_or(50)) + d).clamp(0, 100) as u8,
        }
    }
}

#[derive(Debug, Default)]
pub struct Monitors {
    devices: Vec<Device>,
    /// Last DDC value read or written (DDC reads are slow).
    ddc_percent: Option<u8>,
}

impl Monitors {
    /// Find controllable displays. DDC detection takes a second or two.
    pub fn detect(config: &MonitorBrightness) -> Self {
        let mut devices = Vec::new();
        if config.backlight {
            devices.extend(backlights());
        }
        if config.ddc {
            match ddc_displays() {
                Ok(found) => devices.extend(found),
                Err(err) => eprintln!("ddc: {err:#}"),
            }
        }
        let mut monitors = Self {
            devices,
            ddc_percent: None,
        };
        monitors.ddc_percent = monitors.read_ddc();
        monitors
    }

    /// Current brightness in percent: the backlight if there is one (cheap to read, and
    /// follows the brightness keys), else the last DDC value.
    pub fn percent(&self) -> Option<u8> {
        self.devices
            .iter()
            .find_map(|device| match device {
                Device::Backlight { name, max } => {
                    read_u32(&backlight_path(name, "brightness")).map(|raw| to_percent(raw, *max))
                }
                Device::Ddc { .. } => None,
            })
            .or(self.ddc_percent)
    }

    /// Apply `change` to every display, fading over `fade` (backlights; DDC takes a few
    /// steps). `ddc: false` leaves external monitors alone. Returns the new percent.
    pub fn change(&mut self, change: Change, fade: Duration, ddc: bool) -> Result<u8> {
        if self.devices.is_empty() {
            bail!("no controllable displays (no backlight, and ddcutil found no monitors)");
        }
        let included = |device: &Device| ddc || matches!(device, Device::Backlight { .. });
        let attempted = self.devices.iter().filter(|d| included(d)).count();
        if attempted == 0 {
            bail!("no backlight to adjust");
        }
        let target = change.resolve(self.percent());
        let ddc_from = self.ddc_percent;
        // Fade every display at once.
        let errors: Vec<String> = thread::scope(|scope| {
            let fades: Vec<_> = self
                .devices
                .iter()
                .filter(|device| included(device))
                .map(|device| {
                    scope.spawn(move || match device {
                        // Never switch a panel fully off.
                        Device::Backlight { name, max } => {
                            fade_backlight(name, from_percent(target.max(1), *max).max(1), fade)
                        }
                        Device::Ddc { display, max } => fade_ddc(*display, *max, ddc_from, target),
                    })
                })
                .collect();
            fades
                .into_iter()
                .filter_map(|fade| match fade.join() {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(format!("{err:#}")),
                    Err(_) => Some("brightness thread panicked".into()),
                })
                .collect()
        });
        if ddc
            && self
                .devices
                .iter()
                .any(|device| matches!(device, Device::Ddc { .. }))
        {
            self.ddc_percent = Some(target);
        }
        if errors.len() == attempted {
            bail!("{}", errors.join("; "));
        }
        for err in errors {
            eprintln!("warning: {err}");
        }
        Ok(target)
    }

    fn read_ddc(&self) -> Option<u8> {
        self.devices.iter().find_map(|device| match device {
            Device::Ddc { display, .. } => get_ddc(*display)
                .ok()
                .map(|(cur, max)| to_percent(cur, max)),
            Device::Backlight { .. } => None,
        })
    }
}

fn to_percent(raw: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    ((f64::from(raw) * 100.0 / f64::from(max)).round() as u32).min(100) as u8
}

fn from_percent(percent: u8, max: u32) -> u32 {
    (f64::from(percent) * f64::from(max) / 100.0).round() as u32
}

fn backlight_path(name: &str, file: &str) -> PathBuf {
    Path::new(BACKLIGHT_DIR).join(name).join(file)
}

fn read_u32(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn backlights() -> Vec<Device> {
    let Ok(entries) = fs::read_dir(BACKLIGHT_DIR) else {
        return Vec::new();
    };
    let mut devices: Vec<Device> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let max = read_u32(&backlight_path(&name, "max_brightness"))?;
            (max > 0).then_some(Device::Backlight { name, max })
        })
        .collect();
    devices.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    devices
}

/// Ease in and out.
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The values to step through from `from` to `to` (ending exactly at `to`).
fn fade_steps(from: u32, to: u32, steps: u32) -> Vec<u32> {
    if from == to {
        return vec![to];
    }
    let mut values: Vec<u32> = (1..=steps)
        .map(|i| {
            let t = smoothstep(f64::from(i) / f64::from(steps));
            (f64::from(from) + (f64::from(to) - f64::from(from)) * t).round() as u32
        })
        .collect();
    values.dedup();
    values
}

fn fade_backlight(name: &str, target: u32, fade: Duration) -> Result<()> {
    let from = read_u32(&backlight_path(name, "brightness")).unwrap_or(target);
    let frames = (fade.as_millis() / BACKLIGHT_FRAME.as_millis()).max(1) as u32;
    let steps = fade_steps(from, target, frames);
    let pause = fade / steps.len() as u32;
    // One logind connection for the whole fade; sysfs if logind isn't available.
    let conn = dbus::blocking::Connection::new_system().ok();
    for (i, raw) in steps.iter().enumerate() {
        if i > 0 {
            thread::sleep(pause);
        }
        set_backlight(conn.as_ref(), name, *raw)?;
    }
    Ok(())
}

fn set_backlight(conn: Option<&dbus::blocking::Connection>, name: &str, raw: u32) -> Result<()> {
    let logind = || -> Result<()> {
        let conn = conn.context("no system bus")?;
        let session = conn.with_proxy(
            "org.freedesktop.login1",
            "/org/freedesktop/login1/session/auto",
            Duration::from_secs(5),
        );
        session.method_call::<(), _, _, _>(
            "org.freedesktop.login1.Session",
            "SetBrightness",
            ("backlight", name, raw),
        )?;
        Ok(())
    };
    match logind() {
        Ok(()) => Ok(()),
        Err(logind_err) => fs::write(backlight_path(name, "brightness"), raw.to_string())
            .with_context(|| format!("failed to set backlight {name} (logind: {logind_err})")),
    }
}

/// DDC steps pace themselves: each write takes about as long as a frame of the fade.
fn fade_ddc(display: u32, max: u32, from: Option<u8>, target: u8) -> Result<()> {
    let to = from_percent(target, max);
    let steps = match from {
        Some(from) => fade_steps(from_percent(from, max), to, DDC_STEPS),
        None => vec![to],
    };
    for value in steps {
        set_ddc(display, value)?;
    }
    Ok(())
}

fn ddcutil(args: &[&str]) -> Result<String> {
    let output = Command::new("ddcutil")
        .args(args)
        .output()
        .context("ddcutil not found")?;
    if !output.status.success() {
        bail!(
            "ddcutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn ddc_displays() -> Result<Vec<Device>> {
    let detected = ddcutil(&["detect", "--terse"])?;
    let mut devices = Vec::new();
    for display in parse_detect(&detected) {
        match get_ddc(display) {
            Ok((_, max)) if max > 0 => devices.push(Device::Ddc { display, max }),
            Ok(_) => {}
            Err(err) => eprintln!("ddc: display {display}: {err:#}"),
        }
    }
    Ok(devices)
}

/// Display numbers from `ddcutil detect --terse` (skips "Invalid display" entries).
fn parse_detect(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("Display "))
        .filter_map(|rest| rest.trim().parse().ok())
        .collect()
}

/// `(current, max)` from `ddcutil getvcp 10 --terse`: `VCP 10 C 50 100`.
fn parse_getvcp(output: &str) -> Option<(u32, u32)> {
    let fields: Vec<&str> = output.split_whitespace().collect();
    match fields.as_slice() {
        ["VCP", _, "C", current, max, ..] => Some((current.parse().ok()?, max.parse().ok()?)),
        _ => None,
    }
}

fn get_ddc(display: u32) -> Result<(u32, u32)> {
    let display = display.to_string();
    let output = ddcutil(&["--display", &display, "getvcp", VCP_BRIGHTNESS, "--terse"])?;
    parse_getvcp(&output).with_context(|| format!("unexpected ddcutil output: {output}"))
}

fn set_ddc(display: u32, value: u32) -> Result<()> {
    let (display, value) = (display.to_string(), value.to_string());
    ddcutil(&["--display", &display, "setvcp", VCP_BRIGHTNESS, &value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_changes() {
        assert_eq!(Change::parse("50").unwrap(), Change::Set(50));
        assert_eq!(Change::parse("+10").unwrap(), Change::By(10));
        assert_eq!(Change::parse("-5%").unwrap(), Change::By(-5));
        assert!(Change::parse("101").is_err());
        assert!(Change::parse("bright").is_err());
        assert_eq!(Change::By(-5).to_arg(), "-5");
        assert_eq!(
            Change::parse(&Change::By(10).to_arg()).unwrap(),
            Change::By(10)
        );
    }

    #[test]
    fn resolves_relative_changes() {
        assert_eq!(Change::By(10).resolve(Some(95)), 100);
        assert_eq!(Change::By(-30).resolve(Some(20)), 0);
        assert_eq!(Change::Set(40).resolve(Some(90)), 40);
    }

    #[test]
    fn parses_ddcutil_output() {
        let detect = "Display 1\n   I2C bus:  /dev/i2c-5\nInvalid display\n   I2C bus: /dev/i2c-7\nDisplay 2\n";
        assert_eq!(parse_detect(detect), vec![1, 2]);
        assert_eq!(parse_getvcp("VCP 10 C 50 100\n"), Some((50, 100)));
        assert_eq!(parse_getvcp("VCP 10 ERR\n"), None);
    }

    #[test]
    fn fades_end_on_target() {
        let up = fade_steps(10, 200, BACKLIGHT_STEPS);
        assert_eq!(*up.last().unwrap(), 200);
        assert!(up.windows(2).all(|w| w[0] < w[1]));
        let down = fade_steps(90, 30, DDC_STEPS);
        assert_eq!(down.len(), 3);
        assert_eq!(*down.last().unwrap(), 30);
        assert_eq!(fade_steps(5, 5, 20), vec![5]);
        assert_eq!(fade_steps(5, 6, 20), vec![5, 6]);
    }

    #[test]
    fn percent_round_trips() {
        assert_eq!(to_percent(48000, 96000), 50);
        assert_eq!(from_percent(50, 96000), 48000);
        assert_eq!(to_percent(5, 0), 0);
    }
}
