//! Where gamma ramps go: X11 RandR or the wlroots gamma-control protocol.

mod wayland;
mod x11;

use anyhow::{bail, Result};

use crate::color::Setting;
use crate::config::BackendKind;

pub trait Backend {
    fn name(&self) -> &'static str;
    /// Set every output's ramps.
    fn apply(&mut self, setting: Setting, gamma: f64) -> Result<()>;
    /// Handle pending display events. Returns true when ramps should be re-applied (new
    /// outputs, or a backend that something else may reset).
    fn poll(&mut self) -> Result<bool>;
    /// Put the original ramps back.
    fn restore(&mut self) -> Result<()>;
}

pub fn open(kind: BackendKind) -> Result<Box<dyn Backend>> {
    let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    match kind {
        BackendKind::Wayland => Ok(Box::new(wayland::WaylandBackend::connect()?)),
        BackendKind::X11 => Ok(Box::new(x11::X11Backend::connect()?)),
        BackendKind::Auto if on_wayland => Ok(Box::new(wayland::WaylandBackend::connect()?)),
        BackendKind::Auto if std::env::var_os("DISPLAY").is_some() => {
            Ok(Box::new(x11::X11Backend::connect()?))
        }
        BackendKind::Auto => bail!("neither WAYLAND_DISPLAY nor DISPLAY is set"),
    }
}
