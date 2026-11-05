//! Geometry of the selection rectangle in the overlay: creating, moving and resizing it, and
//! converting it to a pixel crop. Coordinates are logical (window) pixels unless noted.

/// An axis-aligned rectangle with `x0 <= x1` and `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    /// The rectangle spanned by two corners, in any order.
    pub fn from_corners((ax, ay): (f64, f64), (bx, by): (f64, f64)) -> Self {
        Self {
            x0: ax.min(bx),
            y0: ay.min(by),
            x1: ax.max(bx),
            y1: ay.max(by),
        }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }

    /// Clip to `0..width` x `0..height`.
    pub fn clamp_to(self, width: f64, height: f64) -> Self {
        Self {
            x0: self.x0.clamp(0.0, width),
            y0: self.y0.clamp(0.0, height),
            x1: self.x1.clamp(0.0, width),
            y1: self.y1.clamp(0.0, height),
        }
    }

    /// Move by `(dx, dy)`, keeping the whole rectangle inside `0..width` x `0..height`.
    pub fn translate_within(self, dx: f64, dy: f64, width: f64, height: f64) -> Self {
        let dx = dx.clamp(-self.x0, (width - self.x1).max(-self.x0));
        let dy = dy.clamp(-self.y0, (height - self.y1).max(-self.y0));
        Self {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }

    /// The crop in device pixels for a snapshot `scale` times the logical size, clipped to the
    /// snapshot. `None` if nothing is left.
    pub fn to_pixels(self, scale: f64, width: u32, height: u32) -> Option<PixelRect> {
        let edge = |value: f64, max: u32| ((value * scale).round().max(0.0) as u32).min(max);
        let (x0, x1) = (edge(self.x0, width), edge(self.x1, width));
        let (y0, y1) = (edge(self.y0, height), edge(self.y1, height));
        (x1 > x0 && y1 > y0).then_some(PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

/// A crop of the snapshot in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Which edges a resize drags. A corner drags two edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

/// What a press at a point would grab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Resize(Edges),
    Move,
    Outside,
}

/// Find what is under `point`: an edge or corner within `slop` of the outline, the inside, or
/// nothing.
pub fn hit_test(rect: &Rect, (x, y): (f64, f64), slop: f64) -> Hit {
    let near = |value: f64, edge: f64| (value - edge).abs() <= slop;
    let within_x = (rect.x0 - slop..=rect.x1 + slop).contains(&x);
    let within_y = (rect.y0 - slop..=rect.y1 + slop).contains(&y);
    if within_x && within_y {
        let edges = Edges {
            left: near(x, rect.x0),
            right: near(x, rect.x1) && !near(x, rect.x0),
            top: near(y, rect.y0),
            bottom: near(y, rect.y1) && !near(y, rect.y0),
        };
        if edges != Edges::default() {
            return Hit::Resize(edges);
        }
    }
    if rect.contains((x, y)) {
        Hit::Move
    } else {
        Hit::Outside
    }
}

/// `original` with the dragged `edges` moved by `(dx, dy)`. Dragging an edge past the
/// opposite one flips the rectangle instead of collapsing it.
pub fn resize(original: &Rect, edges: Edges, dx: f64, dy: f64) -> Rect {
    let mut x0 = original.x0;
    let mut x1 = original.x1;
    let mut y0 = original.y0;
    let mut y1 = original.y1;
    if edges.left {
        x0 += dx;
    }
    if edges.right {
        x1 += dx;
    }
    if edges.top {
        y0 += dy;
    }
    if edges.bottom {
        y1 += dy;
    }
    Rect::from_corners((x0, y0), (x1, y1))
}

/// The 8 handle positions: corners first, then edge midpoints.
pub fn handles(rect: &Rect) -> [(f64, f64); 8] {
    let (cx, cy) = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    [
        (rect.x0, rect.y0),
        (rect.x1, rect.y0),
        (rect.x0, rect.y1),
        (rect.x1, rect.y1),
        (cx, rect.y0),
        (cx, rect.y1),
        (rect.x0, cy),
        (rect.x1, cy),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn corners_in_any_order() {
        assert_eq!(
            Rect::from_corners((50.0, 10.0), (20.0, 40.0)),
            rect(20.0, 10.0, 50.0, 40.0)
        );
    }

    #[test]
    fn hit_test_finds_corners_edges_and_inside() {
        let r = rect(100.0, 100.0, 200.0, 200.0);
        let top_left = Edges {
            left: true,
            top: true,
            ..Edges::default()
        };
        assert_eq!(hit_test(&r, (98.0, 103.0), 6.0), Hit::Resize(top_left));
        let right = Edges {
            right: true,
            ..Edges::default()
        };
        assert_eq!(hit_test(&r, (204.0, 150.0), 6.0), Hit::Resize(right));
        assert_eq!(hit_test(&r, (150.0, 150.0), 6.0), Hit::Move);
        assert_eq!(hit_test(&r, (300.0, 150.0), 6.0), Hit::Outside);
        // Beside the rectangle but level with an edge is still outside.
        assert_eq!(hit_test(&r, (300.0, 100.0), 6.0), Hit::Outside);
    }

    #[test]
    fn resize_moves_only_dragged_edges_and_flips() {
        let r = rect(100.0, 100.0, 200.0, 200.0);
        let right = Edges {
            right: true,
            ..Edges::default()
        };
        assert_eq!(
            resize(&r, right, 30.0, 99.0),
            rect(100.0, 100.0, 230.0, 200.0)
        );
        assert_eq!(
            resize(&r, right, -150.0, 0.0),
            rect(50.0, 100.0, 100.0, 200.0)
        );
    }

    #[test]
    fn translate_stays_on_screen() {
        let r = rect(10.0, 10.0, 60.0, 60.0);
        assert_eq!(
            r.translate_within(-50.0, 500.0, 100.0, 100.0),
            rect(0.0, 50.0, 50.0, 100.0)
        );
    }

    #[test]
    fn converts_to_device_pixels() {
        let r = rect(10.0, 20.0, 110.0, 70.0);
        assert_eq!(
            r.to_pixels(2.0, 4000, 4000),
            Some(PixelRect {
                x: 20,
                y: 40,
                width: 200,
                height: 100
            })
        );
        // Clipped to the snapshot.
        assert_eq!(
            rect(-5.0, -5.0, 50.0, 50.0).to_pixels(1.0, 30, 30),
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 30,
                height: 30
            })
        );
        assert_eq!(rect(40.0, 40.0, 50.0, 50.0).to_pixels(1.0, 30, 30), None);
    }
}
