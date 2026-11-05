//! A full-screen overlay that takes all input (for picking a region or a window), and small
//! always-on-top windows at fixed screen positions.
//!
//! On X11 the overlay is one override-redirect window over the whole screen that grabs the
//! pointer and keyboard. On Wayland, clients can't place or grab, so it is one layer-shell
//! surface per monitor on the overlay layer with exclusive keyboard focus (wlroots
//! compositors and KDE), or a full-screen window per monitor where layer-shell is missing.
//!
//! Either way callers see a single canvas: positions are GTK logical pixels relative to the
//! top-left of [`crate::screen::bounds`], and drawing is done once for the whole canvas.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::screen;
use crate::session;

const GRAB_ATTEMPTS: u32 = 20;
const GRAB_RETRY: Duration = Duration::from_millis(50);

/// A full-screen input-taking overlay. Cheap to clone; all clones are the same overlay.
#[derive(Clone)]
pub struct Overlay {
    inner: Rc<Inner>,
}

struct Inner {
    surfaces: Vec<Surface>,
    size: (f64, f64),
    grabbed: Cell<bool>,
    closed: Cell<bool>,
    /// The overlay itself while it is shown, so it lives as long as its windows even if the
    /// caller drops every handle (event handlers only hold weak references). Cleared by
    /// [`Overlay::close`].
    shown: RefCell<Option<Overlay>>,
}

struct Surface {
    window: gtk::Window,
    /// Where this window's top-left corner is on the canvas.
    origin: (f64, f64),
}

impl Overlay {
    /// Create the overlay (not yet shown). `namespace` names the layer surfaces on Wayland.
    /// Must be called on the GTK thread.
    pub fn new(namespace: &str) -> Result<Self> {
        let (surfaces, size) = if session::is_wayland() {
            wayland_surfaces(namespace)?
        } else {
            x11_surface()?
        };
        for surface in &surfaces {
            surface.window.set_app_paintable(true);
            surface.window.add_events(
                gdk::EventMask::BUTTON_PRESS_MASK
                    | gdk::EventMask::BUTTON_RELEASE_MASK
                    | gdk::EventMask::POINTER_MOTION_MASK
                    | gdk::EventMask::KEY_PRESS_MASK,
            );
        }
        Ok(Self {
            inner: Rc::new(Inner {
                surfaces,
                size,
                grabbed: Cell::new(false),
                closed: Cell::new(false),
                shown: RefCell::new(None),
            }),
        })
    }

    /// Size of the canvas in logical pixels.
    pub fn size(&self) -> (f64, f64) {
        self.inner.size
    }

    pub fn display(&self) -> gdk::Display {
        self.inner.surfaces[0].window.display()
    }

    /// Draw the whole canvas. Called once per window, already translated to canvas
    /// coordinates (each window shows its part).
    pub fn connect_draw(&self, draw: impl Fn(&cairo::Context) + 'static) {
        let draw = Rc::new(draw);
        for surface in &self.inner.surfaces {
            let draw = draw.clone();
            let (dx, dy) = surface.origin;
            surface.window.connect_draw(move |_, cr| {
                cr.translate(-dx, -dy);
                draw(cr);
                glib::Propagation::Stop
            });
        }
    }

    /// Mouse button presses, with the position on the canvas.
    pub fn connect_button_press(
        &self,
        handler: impl Fn(&Overlay, &gdk::EventButton, (f64, f64)) + 'static,
    ) {
        let handler = Rc::new(handler);
        self.each_surface(move |weak, surface| {
            let handler = handler.clone();
            let origin = surface.origin;
            surface.window.connect_button_press_event(move |_, event| {
                if let Some(overlay) = upgrade(&weak) {
                    handler(&overlay, event, to_canvas(origin, event.position()));
                }
                glib::Propagation::Stop
            });
        });
    }

    /// Mouse button releases, with the position on the canvas.
    pub fn connect_button_release(
        &self,
        handler: impl Fn(&Overlay, &gdk::EventButton, (f64, f64)) + 'static,
    ) {
        let handler = Rc::new(handler);
        self.each_surface(move |weak, surface| {
            let handler = handler.clone();
            let origin = surface.origin;
            surface
                .window
                .connect_button_release_event(move |_, event| {
                    if let Some(overlay) = upgrade(&weak) {
                        handler(&overlay, event, to_canvas(origin, event.position()));
                    }
                    glib::Propagation::Stop
                });
        });
    }

    /// Pointer motion, as a position on the canvas.
    pub fn connect_motion(&self, handler: impl Fn(&Overlay, (f64, f64)) + 'static) {
        let handler = Rc::new(handler);
        self.each_surface(move |weak, surface| {
            let handler = handler.clone();
            let origin = surface.origin;
            surface.window.connect_motion_notify_event(move |_, event| {
                if let Some(overlay) = upgrade(&weak) {
                    handler(&overlay, to_canvas(origin, event.position()));
                }
                glib::Propagation::Stop
            });
        });
    }

    pub fn connect_key_press(&self, handler: impl Fn(&Overlay, &gdk::EventKey) + 'static) {
        let handler = Rc::new(handler);
        self.each_surface(move |weak, surface| {
            let handler = handler.clone();
            surface.window.connect_key_press_event(move |_, event| {
                if let Some(overlay) = upgrade(&weak) {
                    handler(&overlay, event);
                }
                glib::Propagation::Stop
            });
        });
    }

    fn each_surface(&self, mut connect: impl FnMut(Weak<Inner>, &Surface)) {
        for surface in &self.inner.surfaces {
            connect(Rc::downgrade(&self.inner), surface);
        }
    }

    /// Show the overlay with `cursor` and take the input. On X11 the grab is retried for a
    /// moment, since a hotkey daemon or menu may still hold its own; `on_grab_failed` is
    /// called (from the main loop) if it never succeeds.
    pub fn show(&self, cursor: gdk::CursorType, on_grab_failed: impl FnOnce() + 'static) {
        *self.inner.shown.borrow_mut() = Some(self.clone());
        for surface in &self.inner.surfaces {
            surface.window.show_all();
        }
        self.set_cursor(cursor);
        if session::is_wayland() {
            // Layer surfaces get exclusive keyboard focus; the pointer needs no grab.
            if let Some(surface) = self.inner.surfaces.first() {
                surface.window.present();
            }
            return;
        }
        let weak = Rc::downgrade(&self.inner);
        glib::idle_add_local_once(move || grab(weak, GRAB_ATTEMPTS, Box::new(on_grab_failed)));
    }

    pub fn queue_draw(&self) {
        for surface in &self.inner.surfaces {
            surface.window.queue_draw();
        }
    }

    pub fn set_cursor(&self, cursor: gdk::CursorType) {
        for surface in &self.inner.surfaces {
            if let Some(gdk_window) = surface.window.window() {
                let cursor = gdk::Cursor::for_display(&surface.window.display(), cursor);
                gdk_window.set_cursor(cursor.as_ref());
            }
        }
    }

    /// Center of the monitor the pointer is on (X11), or of the first monitor, on the
    /// canvas.
    pub fn pointer_monitor_center(&self) -> (f64, f64) {
        let display = self.display();
        let pointer_monitor = (!session::is_wayland())
            .then(|| {
                let pointer = display.default_seat()?.pointer()?;
                let (_, x, y) = pointer.position();
                display.monitor_at_point(x, y)
            })
            .flatten();
        let monitor = pointer_monitor.or_else(|| display.monitor(0));
        match monitor {
            Some(monitor) => {
                let area = monitor.geometry();
                let (x0, y0) = self.inner.layout_origin(&display);
                (
                    f64::from(area.x() - x0) + f64::from(area.width()) / 2.0,
                    f64::from(area.y() - y0) + f64::from(area.height()) / 2.0,
                )
            }
            None => (self.inner.size.0 / 2.0, self.inner.size.1 / 2.0),
        }
    }

    /// Release the input and close every window. Safe to call more than once.
    pub fn close(&self) {
        if self.inner.closed.replace(true) {
            return;
        }
        if self.inner.grabbed.get() {
            if let Some(seat) = self.display().default_seat() {
                seat.ungrab();
            }
        }
        for surface in &self.inner.surfaces {
            surface.window.close();
        }
        self.inner.shown.borrow_mut().take();
    }
}

impl Inner {
    /// GDK coordinates of the canvas's top-left corner.
    fn layout_origin(&self, display: &gdk::Display) -> (i32, i32) {
        if !session::is_wayland() {
            return (0, 0);
        }
        (0..display.n_monitors())
            .filter_map(|index| display.monitor(index))
            .map(|monitor| monitor.geometry())
            .fold(None, |acc: Option<(i32, i32)>, g| {
                Some(acc.map_or((g.x(), g.y()), |(x, y)| (x.min(g.x()), y.min(g.y()))))
            })
            .unwrap_or((0, 0))
    }
}

fn upgrade(weak: &Weak<Inner>) -> Option<Overlay> {
    weak.upgrade().map(|inner| Overlay { inner })
}

fn to_canvas(origin: (f64, f64), (x, y): (f64, f64)) -> (f64, f64) {
    (x + origin.0, y + origin.1)
}

fn grab(weak: Weak<Inner>, attempts_left: u32, on_failed: Box<dyn FnOnce()>) {
    let Some(overlay) = upgrade(&weak) else {
        return;
    };
    if overlay.inner.closed.get() {
        return;
    }
    let window = &overlay.inner.surfaces[0].window;
    let display = window.display();
    let (Some(seat), Some(gdk_window)) = (display.default_seat(), window.window()) else {
        eprintln!("error: no input seat for the overlay");
        on_failed();
        return;
    };
    // No grab cursor, so the window's cursor (which callers may change) is shown.
    let status = seat.grab(
        &gdk_window,
        gdk::SeatCapabilities::ALL,
        true,
        None,
        None,
        None,
    );
    if status == gdk::GrabStatus::Success {
        overlay.inner.grabbed.set(true);
        return;
    }
    if attempts_left == 0 {
        eprintln!("error: could not grab input for the overlay ({status:?})");
        on_failed();
        return;
    }
    glib::timeout_add_local_once(GRAB_RETRY, move || grab(weak, attempts_left - 1, on_failed));
}

fn x11_surface() -> Result<(Vec<Surface>, (f64, f64))> {
    let (width, height) = screen::size()?;
    let scale = screen::logical_scale();
    let (width, height) = (
        (f64::from(width) / scale).floor() as i32,
        (f64::from(height) / scale).floor() as i32,
    );
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.move_(0, 0);
    window.set_default_size(width, height);
    window.set_size_request(width, height);
    Ok((
        vec![Surface {
            window,
            origin: (0.0, 0.0),
        }],
        (f64::from(width), f64::from(height)),
    ))
}

fn wayland_surfaces(namespace: &str) -> Result<(Vec<Surface>, (f64, f64))> {
    let display = gdk::Display::default().context("no display (is GTK initialized?)")?;
    let bounds = screen::bounds()?;
    let layer_shell = gtk_layer_shell::is_supported();
    let surfaces = (0..display.n_monitors())
        .filter_map(|index| display.monitor(index).map(|monitor| (index, monitor)))
        .map(|(index, monitor)| {
            let area = monitor.geometry();
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            window.set_default_size(area.width(), area.height());
            window.set_decorated(false);
            if layer_shell {
                window.init_layer_shell();
                window.set_namespace(namespace);
                window.set_layer(Layer::Overlay);
                window.set_monitor(&monitor);
                for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                    window.set_anchor(edge, true);
                }
                // Cover panels too, rather than being pushed aside by them.
                window.set_exclusive_zone(-1);
                window.set_keyboard_mode(KeyboardMode::Exclusive);
            } else if let Some(screen) = gdk::Screen::default() {
                window.fullscreen_on_monitor(&screen, index);
            }
            Surface {
                window,
                origin: (
                    f64::from(area.x() - bounds.x),
                    f64::from(area.y() - bounds.y),
                ),
            }
        })
        .collect::<Vec<_>>();
    if surfaces.is_empty() {
        anyhow::bail!("no monitors found");
    }
    Ok((
        surfaces,
        (f64::from(bounds.width), f64::from(bounds.height)),
    ))
}

/// An always-on-top window that doesn't take focus, covering the given rectangle (logical
/// pixels on the overlay canvas, see the module docs). Not shown yet.
pub fn floating_window(namespace: &str, x: i32, y: i32, width: i32, height: i32) -> gtk::Window {
    let wayland = session::is_wayland() && gtk_layer_shell::is_supported();
    let window = gtk::Window::new(if wayland {
        gtk::WindowType::Toplevel
    } else {
        gtk::WindowType::Popup
    });
    window.set_app_paintable(true);
    window.set_accept_focus(false);
    window.set_default_size(width, height);
    window.set_size_request(width, height);
    if !wayland {
        window.move_(x, y);
        return window;
    }

    window.init_layer_shell();
    window.set_namespace(namespace);
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    let Some(display) = gdk::Display::default() else {
        return window;
    };
    let (x0, y0) = screen::bounds()
        .map(|bounds| (bounds.x, bounds.y))
        .unwrap_or((0, 0));
    let (gx, gy) = (x + x0, y + y0);
    // The monitor holding the window's center, or the nearest one to its corner.
    let monitor = display
        .monitor_at_point(gx + width / 2, gy + height / 2)
        .or_else(|| display.monitor_at_point(gx, gy));
    if let Some(monitor) = monitor {
        let area = monitor.geometry();
        window.set_monitor(&monitor);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Top, true);
        window.set_layer_shell_margin(Edge::Left, gx - area.x());
        window.set_layer_shell_margin(Edge::Top, gy - area.y());
    }
    window
}
