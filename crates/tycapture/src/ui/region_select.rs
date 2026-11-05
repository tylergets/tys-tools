//! Full-screen overlay for dragging out a recording region, or for clicking a window to
//! record (used on Wayland, where a client can't grab the pointer to pick other windows).
//! Draws a frozen snapshot of the screen with everything outside the selection dimmed.
//!
//! A snapshot (rather than a translucent window) behaves the same with or without a
//! compositor, and is taken the same way the recording reads the screen.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use gtk::gdk;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::glib;
use gtk::prelude::*;
use ty_common::overlay::Overlay;
use ty_common::screen;
use ty_common::wm::{self, WindowId};

use crate::capture::Region;

const DIM_ALPHA: f64 = 0.5;
const MIN_SIZE: f64 = 8.0;
/// Lets the compositor remove the overlay before anything starts capturing the screen.
const CLOSE_SETTLE: Duration = Duration::from_millis(150);

/// `(x, y, width, height)` in logical pixels on the overlay.
type Rect = (f64, f64, f64, f64);

#[derive(Default)]
struct Drag {
    start: Option<(f64, f64)>,
    current: (f64, f64),
}

impl Drag {
    fn rect(&self) -> Option<Rect> {
        let (x0, y0) = self.start?;
        let (x1, y1) = self.current;
        Some((x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()))
    }
}

enum Pick {
    Region(Region),
    Window(WindowId),
}

type Done = Box<dyn FnOnce(Option<Pick>)>;

struct State {
    snapshot: Pixbuf,
    /// Snapshot pixels per logical pixel.
    image_scale: f64,
    /// Screen pixels per logical pixel.
    screen_scale: f64,
    drag: Drag,
    /// Windows to pick from, topmost first; empty when dragging out a region.
    windows: Vec<(WindowId, Rect)>,
    hover: Option<usize>,
    done: Option<Done>,
}

impl State {
    fn picking_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    /// The highlighted rectangle: the drag, or the window under the pointer.
    fn selection(&self) -> Option<Rect> {
        if self.picking_windows() {
            self.hover.map(|index| self.windows[index].1)
        } else {
            self.drag.rect()
        }
    }
}

/// Show the overlay and call `on_done` with the selected region (screen pixels), or `None` if
/// cancelled (Escape or right-click).
pub fn select_region(on_done: impl FnOnce(Option<Region>) + 'static) {
    open(
        Vec::new(),
        Box::new(move |pick| {
            on_done(match pick {
                Some(Pick::Region(region)) => Some(region),
                _ => None,
            })
        }),
    );
}

/// Show the overlay with the visible windows highlighted under the pointer, and call
/// `on_done` with the clicked window, or `None` if cancelled.
pub fn select_window(on_done: impl FnOnce(Result<Option<WindowId>>) + 'static) {
    let windows = match wm::visible_windows() {
        Ok(windows) if !windows.is_empty() => windows,
        Ok(_) => return on_done(Ok(None)),
        Err(err) => return on_done(Err(err)),
    };
    let scale = screen::logical_scale();
    let windows = windows
        .into_iter()
        .map(|window| {
            let r = window.rect;
            let rect = (
                f64::from(r.x) / scale,
                f64::from(r.y) / scale,
                f64::from(r.width) / scale,
                f64::from(r.height) / scale,
            );
            (window.id, rect)
        })
        .collect();
    open(
        windows,
        Box::new(move |pick| {
            on_done(Ok(match pick {
                Some(Pick::Window(id)) => Some(id),
                _ => None,
            }))
        }),
    );
}

fn open(windows: Vec<(WindowId, Rect)>, on_done: Done) {
    let snapshot = match screen::screenshot() {
        Ok(shot) => Pixbuf::from_bytes(
            &glib::Bytes::from_owned(shot.rgb),
            gtk::gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            shot.width as i32,
            shot.height as i32,
            shot.width as i32 * 3,
        ),
        Err(err) => {
            eprintln!("error: failed to snapshot the screen: {err:#}");
            on_done(None);
            return;
        }
    };
    let overlay = match Overlay::new("tycapture") {
        Ok(overlay) => overlay,
        Err(err) => {
            eprintln!("error: failed to show the selection overlay: {err:#}");
            on_done(None);
            return;
        }
    };
    let (width, _) = overlay.size();
    let state = Rc::new(RefCell::new(State {
        image_scale: (f64::from(snapshot.width()) / width).max(1.0),
        screen_scale: screen::logical_scale(),
        snapshot,
        drag: Drag::default(),
        windows,
        hover: None,
        done: Some(on_done),
    }));

    {
        let state = state.clone();
        let size = overlay.size();
        overlay.connect_draw(move |cr| draw(cr, size, &state.borrow()));
    }
    {
        let state = state.clone();
        overlay.connect_button_press(move |overlay, event, position| {
            if event.button() != 1 {
                return finish(overlay, &state, None);
            }
            let mut s = state.borrow_mut();
            if !s.picking_windows() {
                s.drag = Drag {
                    start: Some(position),
                    current: position,
                };
                overlay.queue_draw();
            }
        });
    }
    {
        let state = state.clone();
        overlay.connect_motion(move |overlay, position| {
            let mut s = state.borrow_mut();
            if s.picking_windows() {
                let hover = s
                    .windows
                    .iter()
                    .position(|(_, rect)| contains(rect, position));
                if hover != s.hover {
                    s.hover = hover;
                    overlay.queue_draw();
                }
            } else if s.drag.start.is_some() {
                s.drag.current = position;
                overlay.queue_draw();
            }
        });
    }
    {
        let state = state.clone();
        overlay.connect_button_release(move |overlay, event, position| {
            if event.button() != 1 {
                return;
            }
            let pick = release(&mut state.borrow_mut(), position);
            match pick {
                Some(pick) => finish(overlay, &state, Some(pick)),
                None => overlay.queue_draw(),
            }
        });
    }
    {
        let state = state.clone();
        overlay.connect_key_press(move |overlay, event| {
            if event.keyval() == gdk::keys::constants::Escape {
                finish(overlay, &state, None);
            }
        });
    }

    let on_grab_failed = {
        let (overlay, state) = (overlay.clone(), state.clone());
        move || finish(&overlay, &state, None)
    };
    overlay.show(gdk::CursorType::Crosshair, on_grab_failed);
}

/// What a left-button release at `position` picks, if anything.
fn release(s: &mut State, position: (f64, f64)) -> Option<Pick> {
    if s.picking_windows() {
        let index = s
            .windows
            .iter()
            .position(|(_, rect)| contains(rect, position))?;
        return Some(Pick::Window(s.windows[index].0.clone()));
    }
    s.drag.current = position;
    match s.drag.rect() {
        Some((x, y, w, h)) if w >= MIN_SIZE && h >= MIN_SIZE => {
            // Event coordinates are logical pixels; recording needs screen pixels.
            let scale = s.screen_scale;
            Some(Pick::Region(Region::new(
                (x * scale).round() as i32,
                (y * scale).round() as i32,
                (w * scale).round() as u32,
                (h * scale).round() as u32,
            )))
        }
        // A click without a drag: start over.
        _ => {
            s.drag = Drag::default();
            None
        }
    }
}

fn contains(&(x, y, w, h): &Rect, (px, py): (f64, f64)) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn draw(cr: &gtk::cairo::Context, (width, height): (f64, f64), s: &State) {
    cr.save().ok();
    cr.scale(1.0 / s.image_scale, 1.0 / s.image_scale);
    cr.set_source_pixbuf(&s.snapshot, 0.0, 0.0);
    let _ = cr.paint();
    cr.restore().ok();

    // Dim everything except the selection (even-odd leaves the inner rectangle untouched).
    let selection = s.selection();
    cr.set_operator(gtk::cairo::Operator::Over);
    cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
    cr.rectangle(0.0, 0.0, width, height);
    if let Some((x, y, w, h)) = selection {
        cr.rectangle(x, y, w, h);
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, DIM_ALPHA);
    let _ = cr.fill();

    let Some((x, y, w, h)) = selection else {
        return;
    };

    // Outline just outside the selection, so it is never part of the recording.
    cr.set_source_rgba(0.9, 0.28, 0.3, 1.0);
    cr.set_line_width(2.0);
    cr.rectangle(x - 1.0, y - 1.0, w + 2.0, h + 2.0);
    let _ = cr.stroke();

    let scale = s.screen_scale;
    let label = format!("{}x{}", (w * scale).round(), (h * scale).round());
    cr.set_font_size(14.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    let label_y = if y > 24.0 { y - 8.0 } else { y + h + 20.0 };
    cr.move_to(x, label_y);
    let _ = cr.show_text(&label);
}

fn finish(overlay: &Overlay, state: &Rc<RefCell<State>>, pick: Option<Pick>) {
    let Some(done) = state.borrow_mut().done.take() else {
        return;
    };
    overlay.close();
    glib::timeout_add_local_once(CLOSE_SETTLE, move || done(pick));
}
