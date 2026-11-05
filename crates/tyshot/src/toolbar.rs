//! The toolbar shown next to the selection: annotation tools, frames, undo/redo, send to an
//! opencode session, copy, save and close. Drawn with cairo (no widgets), so it looks the same in the overlay on any setup.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use gtk::cairo;

use crate::selection::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Select,
    Box,
    Arrow,
    Pen,
    Highlight,
    Text,
    Step,
    Blur,
    Frame,
    Undo,
    Redo,
    Agent,
    Copy,
    Save,
    Close,
}

impl Button {
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Select => "Select / move (V)",
            Self::Box => "Box (R)",
            Self::Arrow => "Arrow (A)",
            Self::Pen => "Pen (P)",
            Self::Highlight => "Highlighter (H)",
            Self::Text => "Text (T)",
            Self::Step => "Numbered step (N)",
            Self::Blur => "Blur (B)",
            Self::Frame => "Frame (F)",
            Self::Undo => "Undo (Ctrl+Z)",
            Self::Redo => "Redo (Ctrl+Shift+Z / Ctrl+Y)",
            Self::Agent => "Send to an opencode session (Ctrl+Enter)",
            Self::Copy => "Copy to clipboard (Enter / Ctrl+C)",
            Self::Save => "Save to file (Ctrl+S)",
            Self::Close => "Close (Esc)",
        }
    }
}

/// Buttons in visual groups, separated by a wider gap.
const GROUPS: [&[Button]; 3] = [
    &[
        Button::Select,
        Button::Box,
        Button::Arrow,
        Button::Pen,
        Button::Highlight,
        Button::Text,
        Button::Step,
        Button::Blur,
    ],
    &[Button::Frame, Button::Undo, Button::Redo],
    &[Button::Agent, Button::Copy, Button::Save, Button::Close],
];
/// The size the proportions below (and the icons) are designed at.
const BASE_BUTTON: f64 = 32.0;
const GAP: f64 = 4.0;
const GROUP_GAP: f64 = 14.0;
const PAD: f64 = 5.0;
/// Space between the selection and the toolbar.
const MARGIN: f64 = 10.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub background: Rect,
    pub buttons: Vec<(Button, Rect)>,
    /// `button_size / BASE_BUTTON`: how much bigger than the base design everything is.
    pub zoom: f64,
}

impl Layout {
    pub fn hit(&self, point: (f64, f64)) -> Option<Button> {
        self.buttons
            .iter()
            .find(|(_, rect)| rect.contains(point))
            .map(|(button, _)| *button)
    }

    pub fn contains(&self, point: (f64, f64)) -> bool {
        self.background.contains(point)
    }
}

/// Place the toolbar centered below `around` (the selection, including any frame); above it
/// if there is no room below; inside its bottom edge if there is room for neither. Kept on
/// the `width` x `height` screen. `button_size` is in logical pixels.
pub fn layout(around: &Rect, width: f64, height: f64, button_size: f64) -> Layout {
    let zoom = button_size / BASE_BUTTON;
    let (gap, group_gap, pad) = (GAP * zoom, GROUP_GAP * zoom, PAD * zoom);
    let buttons: usize = GROUPS.iter().map(|group| group.len()).sum();
    let gaps = (buttons - GROUPS.len()) as f64 * gap + (GROUPS.len() - 1) as f64 * group_gap;
    let bar_w = buttons as f64 * button_size + gaps + pad * 2.0;
    let bar_h = button_size + pad * 2.0;

    let below = around.y1 + MARGIN;
    let above = around.y0 - MARGIN - bar_h;
    let y = if below + bar_h <= height {
        below
    } else if above >= 0.0 {
        above
    } else {
        (around.y1.min(height) - MARGIN - bar_h).max(0.0)
    };
    let center = (around.x0 + around.x1) / 2.0;
    let x = (center - bar_w / 2.0).min(width - bar_w).max(0.0);

    let mut buttons = Vec::new();
    let mut left = x + pad;
    for (index, group) in GROUPS.iter().enumerate() {
        if index > 0 {
            left += group_gap - gap;
        }
        for &button in group.iter() {
            let rect =
                Rect::from_corners((left, y + pad), (left + button_size, y + pad + button_size));
            buttons.push((button, rect));
            left += button_size + gap;
        }
    }
    Layout {
        background: Rect::from_corners((x, y), (x + bar_w, y + bar_h)),
        buttons,
        zoom,
    }
}

/// Draw the toolbar. `active` is filled with `accent`; buttons in `toggled` get an `accent`
/// outline; `hover` gets a lighter background.
pub fn draw(
    cr: &cairo::Context,
    layout: &Layout,
    active: Button,
    toggled: &[Button],
    hover: Option<Button>,
    accent: (f64, f64, f64),
) {
    let zoom = layout.zoom;
    rounded_rect(cr, &layout.background, 9.0 * zoom);
    cr.set_source_rgba(0.08, 0.08, 0.1, 0.92);
    let _ = cr.fill();

    for &(button, rect) in &layout.buttons {
        if button == active {
            rounded_rect(cr, &rect, 6.0 * zoom);
            cr.set_source_rgb(accent.0, accent.1, accent.2);
            let _ = cr.fill();
        } else if hover == Some(button) {
            rounded_rect(cr, &rect, 6.0 * zoom);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
            let _ = cr.fill();
        }
        if toggled.contains(&button) && button != active {
            let inset = 1.0 * zoom;
            let ring = Rect {
                x0: rect.x0 + inset,
                y0: rect.y0 + inset,
                x1: rect.x1 - inset,
                y1: rect.y1 - inset,
            };
            rounded_rect(cr, &ring, 6.0 * zoom);
            cr.set_source_rgb(accent.0, accent.1, accent.2);
            cr.set_line_width(2.0 * zoom);
            let _ = cr.stroke();
        }
        cr.save().ok();
        cr.translate((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
        cr.scale(zoom, zoom);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(1.8);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        draw_icon(cr, button);
        cr.restore().ok();
    }
}

/// Icons are drawn centered on the origin within roughly -8..8 (base size).
fn draw_icon(cr: &cairo::Context, button: Button) {
    match button {
        Button::Select => {
            // Pointer arrow.
            cr.move_to(-5.0, -8.0);
            cr.line_to(-5.0, 6.0);
            cr.line_to(-1.5, 2.5);
            cr.line_to(1.5, 8.0);
            cr.line_to(3.5, 7.0);
            cr.line_to(0.7, 1.5);
            cr.line_to(5.5, 1.5);
            cr.close_path();
            let _ = cr.stroke();
        }
        Button::Box => {
            cr.rectangle(-7.0, -5.5, 14.0, 11.0);
            let _ = cr.stroke();
        }
        Button::Arrow => {
            cr.move_to(-6.5, 6.5);
            cr.line_to(5.5, -5.5);
            let _ = cr.stroke();
            cr.move_to(7.0, -7.0);
            cr.line_to(-0.5, -5.0);
            cr.line_to(5.0, 0.5);
            cr.close_path();
            let _ = cr.fill();
        }
        Button::Pen => {
            // A pencil, tip at the bottom left.
            cr.move_to(-7.0, 7.0);
            cr.line_to(-6.0, 3.0);
            cr.line_to(4.0, -7.0);
            cr.line_to(7.0, -4.0);
            cr.line_to(-3.0, 6.0);
            cr.close_path();
            cr.move_to(1.5, -4.5);
            cr.line_to(4.5, -1.5);
            let _ = cr.stroke();
        }
        Button::Highlight => {
            // A marker over a thick translucent stroke.
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.45);
            cr.rectangle(-8.0, 4.0, 16.0, 4.0);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.move_to(-5.0, 2.5);
            cr.line_to(-2.0, -1.0);
            cr.line_to(4.0, -7.0);
            cr.line_to(7.5, -3.5);
            cr.line_to(1.5, 2.5);
            cr.line_to(-2.0, 5.5);
            cr.close_path();
            let _ = cr.stroke();
        }
        Button::Text => {
            // A serif "T".
            cr.move_to(-6.5, -4.5);
            cr.line_to(-6.5, -6.5);
            cr.line_to(6.5, -6.5);
            cr.line_to(6.5, -4.5);
            cr.move_to(0.0, -6.5);
            cr.line_to(0.0, 6.5);
            cr.move_to(-2.5, 6.5);
            cr.line_to(2.5, 6.5);
            let _ = cr.stroke();
        }
        Button::Step => {
            // A circled "1".
            cr.arc(0.0, 0.0, 7.5, 0.0, TAU);
            let _ = cr.stroke();
            cr.move_to(-2.0, -2.5);
            cr.line_to(0.5, -4.5);
            cr.line_to(0.5, 4.5);
            let _ = cr.stroke();
        }
        Button::Blur => {
            // A mosaic of soft squares.
            let alphas = [0.9, 0.35, 0.7, 0.45, 0.85, 0.3, 0.65, 0.4, 0.95];
            for (index, alpha) in alphas.into_iter().enumerate() {
                let (col, row) = ((index % 3) as f64, (index / 3) as f64);
                cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
                cr.rectangle(-7.5 + col * 5.2, -7.5 + row * 5.2, 4.6, 4.6);
                let _ = cr.fill();
            }
        }
        Button::Frame => {
            // A window with a title bar and dots.
            rounded_rect(cr, &Rect::from_corners((-8.0, -6.5), (8.0, 6.5)), 2.0);
            let _ = cr.stroke();
            cr.move_to(-8.0, -2.5);
            cr.line_to(8.0, -2.5);
            let _ = cr.stroke();
            for index in 0..3 {
                cr.arc(-5.5 + index as f64 * 2.6, -4.5, 0.8, 0.0, TAU);
                let _ = cr.fill();
            }
        }
        Button::Undo => {
            cr.arc(1.0, 1.5, 5.5, PI, TAU + FRAC_PI_2 * 0.9);
            let _ = cr.stroke();
            cr.move_to(-8.0, 1.0);
            cr.line_to(-1.5, 1.0);
            cr.line_to(-4.5, -4.5);
            cr.close_path();
            let _ = cr.fill();
        }
        Button::Redo => {
            // The undo icon, mirrored.
            cr.save().ok();
            cr.scale(-1.0, 1.0);
            draw_icon(cr, Button::Undo);
            cr.restore().ok();
        }
        Button::Agent => {
            // A terminal window with a prompt.
            rounded_rect(cr, &Rect::from_corners((-8.0, -6.5), (8.0, 6.5)), 2.0);
            let _ = cr.stroke();
            cr.move_to(-4.5, -2.5);
            cr.line_to(-1.5, 0.0);
            cr.line_to(-4.5, 2.5);
            cr.move_to(0.5, 3.0);
            cr.line_to(4.5, 3.0);
            let _ = cr.stroke();
        }
        Button::Copy => {
            rounded_rect(cr, &Rect::from_corners((-7.0, -7.0), (3.0, 3.0)), 1.5);
            let _ = cr.stroke();
            rounded_rect(cr, &Rect::from_corners((-3.0, -3.0), (7.0, 7.0)), 1.5);
            cr.set_source_rgb(0.08, 0.08, 0.1);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.stroke();
        }
        Button::Save => {
            cr.move_to(0.0, -7.5);
            cr.line_to(0.0, 3.0);
            cr.move_to(-4.0, -1.0);
            cr.line_to(0.0, 3.0);
            cr.line_to(4.0, -1.0);
            cr.move_to(-7.0, 3.5);
            cr.line_to(-7.0, 7.0);
            cr.line_to(7.0, 7.0);
            cr.line_to(7.0, 3.5);
            let _ = cr.stroke();
        }
        Button::Close => {
            cr.move_to(-5.5, -5.5);
            cr.line_to(5.5, 5.5);
            cr.move_to(5.5, -5.5);
            cr.line_to(-5.5, 5.5);
            let _ = cr.stroke();
        }
    }
}

pub fn rounded_rect(cr: &cairo::Context, rect: &Rect, radius: f64) {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    cr.new_sub_path();
    cr.arc(rect.x1 - radius, rect.y0 + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(rect.x1 - radius, rect.y1 - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(rect.x0 + radius, rect.y1 - radius, radius, FRAC_PI_2, PI);
    cr.arc(
        rect.x0 + radius,
        rect.y0 + radius,
        radius,
        PI,
        PI + FRAC_PI_2,
    );
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn sits_below_then_above_then_inside() {
        let below = layout(&sel(400.0, 100.0, 800.0, 300.0), 1920.0, 1080.0, 32.0);
        assert_eq!(below.background.y0, 310.0);
        assert_eq!((below.background.x0 + below.background.x1) / 2.0, 600.0);

        let above = layout(&sel(400.0, 600.0, 800.0, 1070.0), 1920.0, 1080.0, 32.0);
        assert!(above.background.y1 <= 590.0);

        let inside = layout(&sel(0.0, 0.0, 1920.0, 1080.0), 1920.0, 1080.0, 32.0);
        assert!(inside.background.y1 <= 1080.0 && inside.background.y0 > 0.0);
    }

    #[test]
    fn stays_on_screen_horizontally() {
        let bar = layout(&sel(0.0, 100.0, 10.0, 110.0), 1920.0, 1080.0, 44.0);
        assert_eq!(bar.background.x0, 0.0);
        let bar = layout(&sel(1910.0, 100.0, 1920.0, 110.0), 1920.0, 1080.0, 44.0);
        assert_eq!(bar.background.x1, 1920.0);
    }

    #[test]
    fn scales_with_button_size() {
        let small = layout(&sel(400.0, 100.0, 800.0, 300.0), 1920.0, 1080.0, 32.0);
        let big = layout(&sel(400.0, 100.0, 800.0, 300.0), 1920.0, 1080.0, 64.0);
        assert_eq!(big.buttons[0].1.width(), 64.0);
        assert!((big.background.width() - small.background.width() * 2.0).abs() < 1e-9);
    }

    #[test]
    fn hits_buttons_in_order() {
        let bar = layout(&sel(400.0, 100.0, 800.0, 300.0), 1920.0, 1080.0, 44.0);
        assert_eq!(bar.buttons.len(), 15);
        let center = |rect: &Rect| ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
        for (button, rect) in &bar.buttons {
            assert_eq!(bar.hit(center(rect)), Some(*button));
        }
        assert_eq!(bar.hit((0.0, 0.0)), None);
        // Buttons don't overlap and run left to right.
        for pair in bar.buttons.windows(2) {
            assert!(pair[0].1.x1 < pair[1].1.x0);
        }
    }
}
