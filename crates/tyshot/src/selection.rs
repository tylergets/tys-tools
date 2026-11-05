//! Geometry of the selection rectangle in the overlay: creating, moving and resizing it, and
//! converting it to a pixel crop. Coordinates are logical (window) pixels unless noted.

pub use ty_common::geometry::{PixelRect, Rect};

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
}
