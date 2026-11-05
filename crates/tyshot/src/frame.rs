//! Decorative frames drawn around the screenshot: a browser window, a phone, a plain app
//! window. The frame is added outside the selection, so the screenshot itself is never
//! covered or cropped; the exported image grows to include it.
//!
//! All sizes are logical pixels.

use std::f64::consts::{FRAC_PI_2, PI};

use gtk::cairo;

use crate::selection::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Frame {
    #[default]
    None,
    Browser,
    Mobile,
    Window,
}

/// Corner radii: top-left, top-right, bottom-right, bottom-left.
type Radii = [f64; 4];

impl Frame {
    /// The next frame in the cycle the Frame button steps through.
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Browser,
            Self::Browser => Self::Mobile,
            Self::Mobile => Self::Window,
            Self::Window => Self::None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Browser => "Browser",
            Self::Mobile => "Mobile",
            Self::Window => "Window",
        }
    }

    /// Space the frame adds around the content: left, top, right, bottom.
    fn insets(self) -> (f64, f64, f64, f64) {
        match self {
            Self::None => (0.0, 0.0, 0.0, 0.0),
            Self::Browser => (1.0, 44.0, 1.0, 1.0),
            Self::Mobile => (14.0, 46.0, 14.0, 46.0),
            Self::Window => (1.0, 32.0, 1.0, 1.0),
        }
    }

    fn outer_radii(self) -> Radii {
        match self {
            Self::None => [0.0; 4],
            Self::Browser => [10.0; 4],
            Self::Mobile => [44.0; 4],
            Self::Window => [8.0; 4],
        }
    }

    /// Rounding of the content's own corners, so it sits snugly inside the frame.
    fn content_radii(self) -> Radii {
        match self {
            Self::None => [0.0; 4],
            Self::Browser => [0.0, 0.0, 9.0, 9.0],
            Self::Mobile => [30.0; 4],
            Self::Window => [0.0, 0.0, 7.0, 7.0],
        }
    }

    /// The frame's outer bounds around `content`.
    pub fn outer(self, content: &Rect) -> Rect {
        let (left, top, right, bottom) = self.insets();
        Rect {
            x0: content.x0 - left,
            y0: content.y0 - top,
            x1: content.x1 + right,
            y1: content.y1 + bottom,
        }
    }

    /// Clip `cr` to the visible part of `content` (its rounded corners are left to the frame).
    pub fn clip_content(self, cr: &cairo::Context, content: &Rect) {
        rounded_path(cr, content, clamp_radii(self.content_radii(), content));
        cr.clip();
    }

    /// Draw the frame around `content`, leaving the content area untouched.
    pub fn paint(self, cr: &cairo::Context, content: &Rect) {
        if self == Self::None {
            return;
        }
        let outer = self.outer(content);
        cr.save().ok();

        // The frame body: the outer shape minus the content's (rounded) shape.
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        rounded_path(cr, &outer, clamp_radii(self.outer_radii(), &outer));
        rounded_path(cr, content, clamp_radii(self.content_radii(), content));
        let body = match self {
            Self::Browser => (0.925, 0.925, 0.94),
            Self::Mobile => (0.07, 0.07, 0.08),
            _ => (0.17, 0.17, 0.19),
        };
        cr.set_source_rgb(body.0, body.1, body.2);
        let _ = cr.fill();
        cr.set_fill_rule(cairo::FillRule::Winding);

        // A hairline around the outside so light frames stand out on light backgrounds.
        rounded_path(cr, &outer, clamp_radii(self.outer_radii(), &outer));
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        match self {
            Self::None => {}
            Self::Browser => paint_browser_bar(cr, &outer),
            Self::Window => paint_window_bar(cr, &outer),
            Self::Mobile => paint_phone_details(cr, &outer),
        }
        cr.restore().ok();
    }
}

/// Close/minimize/maximize dots, centered vertically in a bar `bar_height` tall.
fn paint_dots(cr: &cairo::Context, outer: &Rect, bar_height: f64, radius: f64) {
    let colors = [(1.0, 0.37, 0.34), (1.0, 0.74, 0.18), (0.16, 0.78, 0.25)];
    for (index, (r, g, b)) in colors.into_iter().enumerate() {
        let x = outer.x0 + 18.0 + index as f64 * (radius * 2.0 + 8.0);
        cr.arc(x, outer.y0 + bar_height / 2.0, radius, 0.0, 2.0 * PI);
        cr.set_source_rgb(r, g, b);
        let _ = cr.fill();
    }
}

fn paint_browser_bar(cr: &cairo::Context, outer: &Rect) {
    let bar = 44.0;
    paint_dots(cr, outer, bar, 6.0);
    // Address field, when there is room for it.
    let (left, right) = (outer.x0 + 92.0, outer.x1 - 20.0);
    if right - left >= 60.0 {
        let field = Rect {
            x0: left,
            y0: outer.y0 + 9.0,
            x1: right,
            y1: outer.y0 + bar - 9.0,
        };
        rounded_path(cr, &field, [field.height() / 2.0; 4]);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        // A placeholder "URL".
        let line_end = (field.x0 + 14.0 + 180.0).min(field.x1 - 14.0);
        if line_end > field.x0 + 24.0 {
            let y = (field.y0 + field.y1) / 2.0;
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.18);
            cr.set_line_width(5.0);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.move_to(field.x0 + 14.0, y);
            cr.line_to(line_end, y);
            let _ = cr.stroke();
        }
    }
}

fn paint_window_bar(cr: &cairo::Context, outer: &Rect) {
    paint_dots(cr, outer, 32.0, 5.5);
}

fn paint_phone_details(cr: &cairo::Context, outer: &Rect) {
    let cx = (outer.x0 + outer.x1) / 2.0;
    // Speaker slot and camera above the screen.
    let speaker = Rect {
        x0: cx - 30.0,
        y0: outer.y0 + 20.0,
        x1: cx + 30.0,
        y1: outer.y0 + 26.0,
    };
    rounded_path(cr, &speaker, [3.0; 4]);
    cr.set_source_rgb(0.2, 0.2, 0.23);
    let _ = cr.fill();
    cr.arc(cx + 46.0, outer.y0 + 23.0, 4.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
    // Home indicator below it.
    let home = Rect {
        x0: cx - 55.0,
        y0: outer.y1 - 26.0,
        x1: cx + 55.0,
        y1: outer.y1 - 21.0,
    };
    rounded_path(cr, &home, [2.5; 4]);
    cr.set_source_rgb(0.28, 0.28, 0.31);
    let _ = cr.fill();
}

/// Radii no larger than half the rectangle's sides.
fn clamp_radii(radii: Radii, rect: &Rect) -> Radii {
    let max = (rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    radii.map(|radius| radius.min(max))
}

/// Add a rectangle with per-corner radii to the current path.
fn rounded_path(cr: &cairo::Context, rect: &Rect, [tl, tr, br, bl]: Radii) {
    cr.new_sub_path();
    cr.arc(rect.x1 - tr, rect.y0 + tr, tr, -FRAC_PI_2, 0.0);
    cr.arc(rect.x1 - br, rect.y1 - br, br, 0.0, FRAC_PI_2);
    cr.arc(rect.x0 + bl, rect.y1 - bl, bl, FRAC_PI_2, PI);
    cr.arc(rect.x0 + tl, rect.y0 + tl, tl, PI, PI + FRAC_PI_2);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_every_frame() {
        let mut frame = Frame::None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            frame = frame.next();
            seen.push(frame);
        }
        assert_eq!(
            seen,
            [Frame::Browser, Frame::Mobile, Frame::Window, Frame::None]
        );
    }

    #[test]
    fn outer_grows_around_content() {
        let content = Rect::from_corners((100.0, 100.0), (300.0, 200.0));
        assert_eq!(Frame::None.outer(&content), content);
        let browser = Frame::Browser.outer(&content);
        assert_eq!((browser.x0, browser.y0, browser.y1), (99.0, 56.0, 201.0));
    }
}
