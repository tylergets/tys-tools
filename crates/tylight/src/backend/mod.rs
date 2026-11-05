//! Where gamma ramps go: X11 RandR or the wlroots gamma-control protocol.

mod wayland;
mod x11;

use anyhow::Result;
use ty_common::session::{self, Session};

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
    let wayland = match kind {
        BackendKind::Wayland => true,
        BackendKind::X11 => false,
        BackendKind::Auto => session::require("tylight")? == Session::Wayland,
    };
    if wayland {
        Ok(Box::new(wayland::WaylandBackend::connect()?))
    } else {
        Ok(Box::new(x11::X11Backend::connect()?))
    }
}
