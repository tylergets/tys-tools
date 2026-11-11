//! Drawing SVG path data (`<path d="...">`) with cairo, so icons can be painted in windows
//! without librsvg. Covers what icon sets use: `M L H V C S Q T A Z`, absolute and relative.

use anyhow::{bail, Context, Result};
use gtk::cairo;

/// Every `d="..."` attribute in `svg`, in document order.
pub fn path_data(svg: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find(" d=\"") {
        let after = &rest[start + 4..];
        let Some(end) = after.find('"') else { break };
        paths.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    paths
}

/// Add the path described by `d` to `cr`'s current path (nothing is stroked or filled).
pub fn trace(cr: &cairo::Context, d: &str) -> Result<()> {
    let mut tokens = Tokens {
        bytes: d.as_bytes(),
        pos: 0,
    };
    let mut command = None;
    // Current point, subpath start, and the last control point for `S`/`T` reflection.
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    let mut last = Control::None;
    loop {
        tokens.skip();
        if tokens.done() {
            return Ok(());
        }
        if let Some(letter) = tokens.command() {
            command = Some(letter);
        }
        let Some(letter) = command else {
            bail!("path data must start with a command: {d:?}");
        };
        let relative = letter.is_ascii_lowercase();
        let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
        let mut next = Control::None;
        match letter.to_ascii_uppercase() {
            b'M' => {
                x = ox + tokens.number()?;
                y = oy + tokens.number()?;
                cr.move_to(x, y);
                (start_x, start_y) = (x, y);
                // Further pairs are line segments.
                command = Some(if relative { b'l' } else { b'L' });
            }
            b'L' => {
                x = ox + tokens.number()?;
                y = oy + tokens.number()?;
                cr.line_to(x, y);
            }
            b'H' => {
                x = ox + tokens.number()?;
                cr.line_to(x, y);
            }
            b'V' => {
                y = oy + tokens.number()?;
                cr.line_to(x, y);
            }
            b'C' => {
                let (x1, y1) = (ox + tokens.number()?, oy + tokens.number()?);
                let (x2, y2) = (ox + tokens.number()?, oy + tokens.number()?);
                x = ox + tokens.number()?;
                y = oy + tokens.number()?;
                cr.curve_to(x1, y1, x2, y2, x, y);
                next = Control::Cubic(x2, y2);
            }
            b'S' => {
                let (x1, y1) = match last {
                    Control::Cubic(cx, cy) => (2.0 * x - cx, 2.0 * y - cy),
                    _ => (x, y),
                };
                let (x2, y2) = (ox + tokens.number()?, oy + tokens.number()?);
                x = ox + tokens.number()?;
                y = oy + tokens.number()?;
                cr.curve_to(x1, y1, x2, y2, x, y);
                next = Control::Cubic(x2, y2);
            }
            b'Q' => {
                let (qx, qy) = (ox + tokens.number()?, oy + tokens.number()?);
                let (nx, ny) = (ox + tokens.number()?, oy + tokens.number()?);
                quad_to(cr, x, y, qx, qy, nx, ny);
                (x, y) = (nx, ny);
                next = Control::Quad(qx, qy);
            }
            b'T' => {
                let (qx, qy) = match last {
                    Control::Quad(cx, cy) => (2.0 * x - cx, 2.0 * y - cy),
                    _ => (x, y),
                };
                let (nx, ny) = (ox + tokens.number()?, oy + tokens.number()?);
                quad_to(cr, x, y, qx, qy, nx, ny);
                (x, y) = (nx, ny);
                next = Control::Quad(qx, qy);
            }
            b'A' => {
                let (rx, ry) = (tokens.number()?, tokens.number()?);
                let rotation = tokens.number()?;
                let large = tokens.flag()?;
                let sweep = tokens.flag()?;
                let (nx, ny) = (ox + tokens.number()?, oy + tokens.number()?);
                arc_to(cr, x, y, rx, ry, rotation, large, sweep, nx, ny);
                (x, y) = (nx, ny);
            }
            b'Z' => {
                cr.close_path();
                (x, y) = (start_x, start_y);
            }
            other => bail!("unsupported path command {:?}", other as char),
        }
        last = next;
    }
}

#[derive(Clone, Copy)]
enum Control {
    None,
    Cubic(f64, f64),
    Quad(f64, f64),
}

struct Tokens<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Tokens<'_> {
    fn skip(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            self.pos += 1;
        }
    }

    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn command(&mut self) -> Option<u8> {
        let letter = *self.bytes.get(self.pos)?;
        if letter.is_ascii_alphabetic() && !matches!(letter, b'e' | b'E') {
            self.pos += 1;
            Some(letter)
        } else {
            None
        }
    }

    /// A number: `-2`, `.5`, `1.02`, `4.5e-3`. In path data `1.02-.1` and `4.5.6` are two
    /// numbers each.
    fn number(&mut self) -> Result<f64> {
        self.skip();
        let start = self.pos;
        let mut seen_dot = false;
        let mut seen_exp = false;
        while let Some(&b) = self.bytes.get(self.pos) {
            let ok = match b {
                b'0'..=b'9' => true,
                b'.' if !seen_dot && !seen_exp => {
                    seen_dot = true;
                    true
                }
                b'-' | b'+' => self.pos == start || matches!(self.bytes[self.pos - 1], b'e' | b'E'),
                b'e' | b'E' if !seen_exp && self.pos > start => {
                    seen_exp = true;
                    true
                }
                _ => false,
            };
            if !ok {
                break;
            }
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("");
        text.parse()
            .with_context(|| format!("expected a number at offset {start} in path data"))
    }

    /// An arc flag: a single `0` or `1`, possibly not separated from what follows.
    fn flag(&mut self) -> Result<bool> {
        self.skip();
        match self.bytes.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => bail!("expected an arc flag at offset {} in path data", self.pos),
        }
    }
}

/// A quadratic curve as the equivalent cubic.
fn quad_to(cr: &cairo::Context, x0: f64, y0: f64, qx: f64, qy: f64, x: f64, y: f64) {
    let (x1, y1) = (x0 + 2.0 / 3.0 * (qx - x0), y0 + 2.0 / 3.0 * (qy - y0));
    let (x2, y2) = (x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y));
    cr.curve_to(x1, y1, x2, y2, x, y);
}

/// An SVG elliptical arc (endpoint form, SVG spec F.6.5) as cubic curves.
#[allow(clippy::too_many_arguments)]
fn arc_to(
    cr: &cairo::Context,
    x1: f64,
    y1: f64,
    rx: f64,
    ry: f64,
    rotation: f64,
    large: bool,
    sweep: bool,
    x2: f64,
    y2: f64,
) {
    if rx == 0.0 || ry == 0.0 || (x1 == x2 && y1 == y2) {
        cr.line_to(x2, y2);
        return;
    }
    let phi = rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let sign = if large != sweep { 1.0 } else { -1.0 };
    let coefficient = sign * (numerator / denominator).max(0.0).sqrt();
    let cxp = coefficient * rx * y1p / ry;
    let cyp = -coefficient * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0;
    let start = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let end = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let theta = angle((1.0, 0.0), start);
    let mut delta = angle(start, end);
    if !sweep && delta > 0.0 {
        delta -= 2.0 * std::f64::consts::PI;
    } else if sweep && delta < 0.0 {
        delta += 2.0 * std::f64::consts::PI;
    }
    // Cubic segments of at most a quarter turn.
    let segments = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
    let step = delta / segments;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |a: f64| {
        (
            cx + rx * cos_phi * a.cos() - ry * sin_phi * a.sin(),
            cy + rx * sin_phi * a.cos() + ry * cos_phi * a.sin(),
        )
    };
    let tangent = |a: f64| {
        (
            -rx * cos_phi * a.sin() - ry * sin_phi * a.cos(),
            -rx * sin_phi * a.sin() + ry * cos_phi * a.cos(),
        )
    };
    for segment in 0..segments as u32 {
        let a1 = theta + f64::from(segment) * step;
        let a2 = a1 + step;
        let (p1, p2) = (point(a1), point(a2));
        let (t1, t2) = (tangent(a1), tangent(a2));
        cr.curve_to(
            p1.0 + handle * t1.0,
            p1.1 + handle * t1.1,
            p2.0 - handle * t2.0,
            p2.1 - handle * t2.1,
            p2.0,
            p2.1,
        );
    }
}

/// The signed angle from `u` to `v`.
fn angle(u: (f64, f64), v: (f64, f64)) -> f64 {
    let dot = u.0 * v.0 + u.1 * v.1;
    let len = (u.0 * u.0 + u.1 * u.1).sqrt() * (v.0 * v.0 + v.1 * v.1).sqrt();
    let sign = if u.0 * v.1 - u.1 * v.0 < 0.0 {
        -1.0
    } else {
        1.0
    };
    sign * (dot / len).clamp(-1.0, 1.0).acos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_path_data() {
        let svg = r#"<svg><path d="M1 2"/><path d="h3 v4" /></svg>"#;
        assert_eq!(path_data(svg), vec!["M1 2", "h3 v4"]);
    }

    #[test]
    fn tokenizes_packed_numbers() {
        let mut tokens = Tokens {
            bytes: b"1.02-.1 4.5.6 2e-1,3",
            pos: 0,
        };
        let numbers: Vec<f64> = (0..6).map(|_| tokens.number().unwrap()).collect();
        assert_eq!(numbers, vec![1.02, -0.1, 4.5, 0.6, 0.2, 3.0]);
        assert!(tokens.done());
    }

    /// Trace `d` scaled up and return the painted bounding box.
    fn paint(d: &str) -> (f64, f64, f64, f64) {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 240, 240).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(10.0, 10.0);
        trace(&cr, d).unwrap();
        cr.stroke_extents().unwrap()
    }

    #[test]
    fn traces_lines_and_arcs() {
        let (x1, y1, x2, y2) = paint("M2 12a10 10 0 0 1 18-6");
        assert!(x1 < 2.5 && x2 > 19.5, "spans the chord: {x1} {x2}");
        assert!(y1 < 3.0 && y2 > 11.5, "bows upward: {y1} {y2}");
        let (x1, y1, x2, y2) = paint("M1 1h4v4H1Z");
        assert!(x1 < 1.0 && y1 < 1.0 && x2 > 5.0 && y2 > 5.0);
        assert!(trace(
            &cairo::Context::new(cairo::ImageSurface::create(cairo::Format::A8, 1, 1).unwrap())
                .unwrap(),
            "x"
        )
        .is_err());
    }

    #[test]
    fn traces_every_lucide_command() {
        for d in [
            "M12 10a2 2 0 0 0-2 2c0 1.02-.1 2.51-.26 4",
            "M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2",
            "M2 16h.01",
            "M3 3q2 2 4 0t4 0s2 2 4 0",
        ] {
            paint(d);
        }
    }
}
