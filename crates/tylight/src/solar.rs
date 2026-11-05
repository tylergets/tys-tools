//! Solar elevation (NOAA solar calculator equations) and day/night blending.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::config::Elevation;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

impl Location {
    pub fn validate(&self) -> Result<()> {
        if !(-90.0..=90.0).contains(&self.latitude) {
            bail!("latitude must be between -90 and 90");
        }
        if !(-180.0..=180.0).contains(&self.longitude) {
            bail!("longitude must be between -180 and 180");
        }
        Ok(())
    }
}

/// Sun elevation above the horizon in degrees at `unix` seconds.
pub fn elevation(unix: f64, location: Location) -> f64 {
    let jd = unix / 86400.0 + 2440587.5;
    let jc = (jd - 2451545.0) / 36525.0;

    let mean_long = (280.46646 + jc * (36000.76983 + jc * 0.0003032)).rem_euclid(360.0);
    let mean_anom = 357.52911 + jc * (35999.05029 - 0.0001537 * jc);
    let ecc = 0.016708634 - jc * (0.000042037 + 0.0000001267 * jc);
    let m = mean_anom.to_radians();
    let center = m.sin() * (1.914602 - jc * (0.004817 + 0.000014 * jc))
        + (2.0 * m).sin() * (0.019993 - 0.000101 * jc)
        + (3.0 * m).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * jc).to_radians();
    let app_long = (mean_long + center - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let obliq_mean =
        23.0 + (26.0 + (21.448 - jc * (46.815 + jc * (0.00059 - jc * 0.001813))) / 60.0) / 60.0;
    let obliq = (obliq_mean + 0.00256 * omega.cos()).to_radians();
    let decl = (obliq.sin() * app_long.sin()).asin();

    let y = (obliq / 2.0).tan().powi(2);
    let l0 = mean_long.to_radians();
    let eq_time = 4.0
        * (y * (2.0 * l0).sin() - 2.0 * ecc * m.sin() + 4.0 * ecc * y * m.sin() * (2.0 * l0).cos()
            - 0.5 * y * y * (4.0 * l0).sin()
            - 1.25 * ecc * ecc * (2.0 * m).sin())
        .to_degrees();

    let utc_minutes = unix.rem_euclid(86400.0) / 60.0;
    let solar_time = (utc_minutes + eq_time + 4.0 * location.longitude).rem_euclid(1440.0);
    let hour_angle = (solar_time / 4.0 - 180.0).to_radians();

    let lat = location.latitude.to_radians();
    let cos_zenith =
        (lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos()).clamp(-1.0, 1.0);
    90.0 - cos_zenith.acos().to_degrees()
}

/// How far into the day setting `elevation` is: 0 at night, 1 by day.
pub fn day_progress(elevation: f64, limits: &Elevation) -> f64 {
    ((elevation - limits.low) / (limits.high - limits.low)).clamp(0.0, 1.0)
}

/// The next time (unix seconds, minute resolution) after `unix` at which it is fully day,
/// within two days. `None` for polar night.
pub fn next_day_start(unix: f64, location: Location, limits: &Elevation) -> Option<f64> {
    let mut was_day = day_progress(elevation(unix, location), limits) >= 1.0;
    (1..=2 * 24 * 60)
        .map(|m| unix + m as f64 * 60.0)
        .find(|&t| {
            let is_day = day_progress(elevation(t, location), limits) >= 1.0;
            let started = is_day && !was_day;
            was_day = is_day;
            started
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREENWICH: Location = Location {
        latitude: 51.48,
        longitude: 0.0,
    };
    /// 2024-06-21 12:00:00 UTC.
    const SOLSTICE_NOON: f64 = 1718971200.0;

    #[test]
    fn solstice_noon_at_greenwich() {
        // 90 - 51.48 + 23.44.
        let e = elevation(SOLSTICE_NOON, GREENWICH);
        assert!((e - 61.96).abs() < 0.3, "{e}");
    }

    #[test]
    fn midnight_is_below_horizon() {
        assert!(elevation(SOLSTICE_NOON - 12.0 * 3600.0, GREENWICH) < -10.0);
    }

    #[test]
    fn longitude_shifts_noon() {
        // Solar noon in New York is ~16:57 UTC.
        let ny = Location {
            latitude: 40.71,
            longitude: -74.0,
        };
        let noonish = elevation(SOLSTICE_NOON + 5.0 * 3600.0, ny);
        assert!((noonish - 72.7).abs() < 1.0, "{noonish}");
    }

    #[test]
    fn progress_blends_between_limits() {
        let limits = Elevation::default();
        assert_eq!(day_progress(10.0, &limits), 1.0);
        assert_eq!(day_progress(-10.0, &limits), 0.0);
        assert!((day_progress(-1.5, &limits) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn next_day_start_is_the_morning() {
        let midnight = SOLSTICE_NOON - 12.0 * 3600.0;
        let start = next_day_start(midnight, GREENWICH, &Elevation::default()).unwrap();
        let hours = (start - midnight) / 3600.0;
        // Sunrise is ~03:43 UTC; the day setting starts a bit after.
        assert!((3.5..5.0).contains(&hours), "{hours}");
    }
}
