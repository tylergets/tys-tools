//! Ambient light sensors exposed through IIO sysfs. Only read and reported for now.

use std::fs;
use std::path::{Path, PathBuf};

const IIO_DIR: &str = "/sys/bus/iio/devices";

#[derive(Debug, Clone)]
pub struct Sensor {
    dir: PathBuf,
}

impl Sensor {
    /// The first IIO device with an illuminance channel.
    pub fn find() -> Option<Self> {
        let mut dirs: Vec<PathBuf> = fs::read_dir(IIO_DIR)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        dirs.sort();
        dirs.into_iter()
            .find(|dir| {
                dir.join("in_illuminance_input").exists() || dir.join("in_illuminance_raw").exists()
            })
            .map(|dir| Self { dir })
    }

    /// Illuminance in lux.
    pub fn lux(&self) -> Option<f64> {
        let read = |name: &str| read_f64(&self.dir.join(name));
        if let Some(lux) = read("in_illuminance_input") {
            return Some(lux);
        }
        let raw = read("in_illuminance_raw")?;
        let offset = read("in_illuminance_offset").unwrap_or(0.0);
        let scale = read("in_illuminance_scale").unwrap_or(1.0);
        Some((raw + offset) * scale)
    }
}

fn read_f64(path: &Path) -> Option<f64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
