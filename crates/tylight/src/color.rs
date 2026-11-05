//! Color temperature to RGB multipliers, and gamma ramps.
//!
//! The white point follows the Planckian locus (Kim et al. cubic approximation in CIE xy),
//! converted to linear sRGB and normalized so 6500K is exactly white.

use serde::{Deserialize, Serialize};

/// Temperature that leaves the screen unchanged.
pub const NEUTRAL_TEMPERATURE: f64 = 6500.0;

/// What is applied to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub temperature: f64,
    /// Gamma dimming, 0..=1.
    pub brightness: f64,
}

impl Setting {
    pub const NEUTRAL: Setting = Setting {
        temperature: NEUTRAL_TEMPERATURE,
        brightness: 1.0,
    };

    /// `self` at `t = 0`, `other` at `t = 1`.
    pub fn lerp(self, other: Setting, t: f64) -> Setting {
        let t = t.clamp(0.0, 1.0);
        Setting {
            temperature: self.temperature + (other.temperature - self.temperature) * t,
            brightness: self.brightness + (other.brightness - self.brightness) * t,
        }
    }

    /// Too small a change to bother re-applying.
    pub fn close_to(self, other: Setting) -> bool {
        (self.temperature - other.temperature).abs() < 1.0
            && (self.brightness - other.brightness).abs() < 0.001
    }

    /// A jump big enough to fade instead of switching at once.
    pub fn far_from(self, other: Setting) -> bool {
        (self.temperature - other.temperature).abs() > 100.0
            || (self.brightness - other.brightness).abs() > 0.05
    }
}

/// RGB multipliers (0..=1, brightest channel 1) for a color temperature.
pub fn whitepoint(kelvin: f64) -> [f64; 3] {
    let raw = locus_rgb(kelvin);
    let white = locus_rgb(NEUTRAL_TEMPERATURE);
    let rgb = [raw[0] / white[0], raw[1] / white[1], raw[2] / white[2]];
    let max = rgb.iter().cloned().fold(f64::MIN, f64::max);
    rgb.map(|c| (c / max).clamp(0.0, 1.0))
}

/// Linear sRGB of the Planckian locus at `kelvin` (Y = 1, negatives clipped).
fn locus_rgb(kelvin: f64) -> [f64; 3] {
    let t = kelvin.clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    let (cx, cy, cz) = (x / y, 1.0, (1.0 - x - y) / y);
    [
        3.2404542 * cx - 1.5371385 * cy - 0.4985314 * cz,
        -0.9692660 * cx + 1.8760108 * cy + 0.0415560 * cz,
        0.0556434 * cx - 0.2040259 * cy + 1.0572252 * cz,
    ]
    .map(|c: f64| c.max(0.0))
}

/// Red, green and blue ramps of `size` entries for `setting`.
pub fn ramps(size: usize, setting: Setting, gamma: f64) -> [Vec<u16>; 3] {
    let white = whitepoint(setting.temperature);
    let last = size.saturating_sub(1).max(1) as f64;
    let ramp = |mult: f64| -> Vec<u16> {
        (0..size)
            .map(|i| {
                let v = (i as f64 / last).powf(1.0 / gamma) * setting.brightness * mult;
                (v.clamp(0.0, 1.0) * 65535.0).round() as u16
            })
            .collect()
    };
    [ramp(white[0]), ramp(white[1]), ramp(white[2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_is_white() {
        for c in whitepoint(6500.0) {
            assert!((c - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn warm_drops_blue_then_green() {
        for k in [5000.0, 4000.0, 3000.0, 2000.0] {
            let [r, g, b] = whitepoint(k);
            assert!((r - 1.0).abs() < 1e-9, "{k}: red stays full");
            assert!(g < 1.0 && b < g, "{k}: {g} {b}");
        }
        assert!(whitepoint(3000.0)[2] < whitepoint(4000.0)[2]);
    }

    #[test]
    fn cool_drops_red() {
        let [r, _, b] = whitepoint(10000.0);
        assert!((b - 1.0).abs() < 1e-9);
        assert!(r < 1.0);
    }

    #[test]
    fn neutral_ramp_is_identity() {
        let [r, g, b] = ramps(256, Setting::NEUTRAL, 1.0);
        assert_eq!(r[0], 0);
        assert_eq!(r[255], 65535);
        assert_eq!(r[128], (128.0 / 255.0 * 65535.0f64).round() as u16);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn lerp_and_closeness() {
        let night = Setting {
            temperature: 3500.0,
            brightness: 0.8,
        };
        let mid = Setting::NEUTRAL.lerp(night, 0.5);
        assert_eq!(mid.temperature, 5000.0);
        assert!((mid.brightness - 0.9).abs() < 1e-9);
        assert!(Setting::NEUTRAL.far_from(night));
        assert!(mid.close_to(mid));
    }
}
