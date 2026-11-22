//! The fingerprint glyph and breathing animation shared by authentication prompts.

use std::f64::consts::TAU;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Result;
use gtk::cairo;

use crate::svgpath;

pub const FRAME: Duration = Duration::from_millis(33);
const PULSE: Duration = Duration::from_millis(2400);
static PATHS: LazyLock<Vec<String>> =
    LazyLock::new(|| svgpath::path_data(include_str!("../icons/fingerprint.svg")));

/// Draw centered at `center`, within `size` pixels. While waiting, breathe in size
/// and opacity; feedback uses a steady glyph in the caller's color.
pub fn paint(
    cr: &cairo::Context,
    center: (f64, f64),
    size: f64,
    rgba: (f64, f64, f64, f64),
    elapsed: Duration,
    waiting: bool,
) -> Result<()> {
    let pulse = if waiting {
        0.5 - 0.5 * (elapsed.as_secs_f64() / PULSE.as_secs_f64() * TAU).cos()
    } else {
        1.0
    };
    let scale = size / 24.0 * (0.94 + 0.06 * pulse);
    cr.save()?;
    let result = (|| -> Result<()> {
        cr.translate(center.0, center.1);
        cr.scale(scale, scale);
        cr.translate(-12.0, -12.0);
        cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3 * (0.45 + 0.55 * pulse));
        cr.set_line_width(1.75);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.new_path();
        for path in PATHS.iter() {
            svgpath::trace(cr, path)?;
        }
        cr.stroke()?;
        Ok(())
    })();
    let restored = cr.restore();
    result?;
    restored?;
    Ok(())
}
