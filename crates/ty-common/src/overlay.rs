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
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, InputFocus, StackMode};

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

    /// Repaint only the canvas rectangle from (`x0`, `y0`) to (`x1`, `y1`): much less work
    /// than [`Overlay::queue_draw`] on a large screen, for small changes such as typing.
    pub fn queue_draw_area(&self, x0: f64, y0: f64, x1: f64, y1: f64) {
        for surface in &self.inner.surfaces {
            let (dx, dy) = surface.origin;
            let left = (x0 - dx).floor() as i32 - 1;
            let top = (y0 - dy).floor() as i32 - 1;
            let right = (x1 - dx).ceil() as i32 + 1;
            let bottom = (y1 - dy).ceil() as i32 + 1;
            if right > left && bottom > top {
                surface
                    .window
                    .queue_draw_area(left, top, right - left, bottom - top);
            }
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
    floating_window_impl(namespace, x, y, width, height, false)
}

/// A floating surface with GTK toplevel keyboard handling, without focus on map.
/// X11 still bypasses the window manager; callers focus it only on user interaction.
pub fn interactive_floating_window(
    namespace: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> gtk::Window {
    floating_window_impl(namespace, x, y, width, height, true)
}

fn floating_window_impl(
    namespace: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    interactive: bool,
) -> gtk::Window {
    let wayland = floating_is_layer();
    let window = gtk::Window::new(if wayland || interactive {
        gtk::WindowType::Toplevel
    } else {
        gtk::WindowType::Popup
    });
    window.set_app_paintable(true);
    window.set_accept_focus(interactive);
    window.set_focus_on_map(false);
    if interactive && !wayland {
        // GtkWindowType::Popup does not maintain toplevel focus for entry widgets,
        // even if XSetInputFocus succeeds. Use GTK's normal keyboard handling while
        // preserving an unmanaged, precisely positioned X11 surface.
        window.connect_realize(|window| {
            if let Some(surface) = window.window() {
                surface.set_override_redirect(true);
            }
        });
    }
    if wayland {
        window.init_layer_shell();
        window.set_namespace(namespace);
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(if interactive {
            KeyboardMode::OnDemand
        } else {
            KeyboardMode::None
        });
        window.set_exclusive_zone(-1);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Top, true);
    }
    place_floating(&window, x, y, width, height);
    window
}

/// Let a [`floating_window`] take keyboard focus when clicked on Wayland.
/// X11 also needs [`focus_floating_on_click`] because the popup is override-redirect.
pub fn set_floating_focusable(window: &gtk::Window) {
    if floating_is_layer() {
        window.set_accept_focus(true);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    }
}

/// Focus an X11 floating window after a click; Wayland uses on-demand focus.
pub fn focus_floating_on_click(window: &gtk::Window, timestamp: u32) {
    // GDK's surface.focus() sends _NET_ACTIVE_WINDOW to the window manager,
    // which ignores override-redirect popups. Focus these directly instead.
    // Call only in response to user interaction, never when presenting a notification.
    if !floating_is_layer() {
        window.set_accept_focus(true);
        let Some(xid) = x11_xid(window) else {
            return;
        };
        let focus = || -> Result<()> {
            window.display().flush();
            let (conn, _) = screen::x11_connect()?;
            conn.set_input_focus(InputFocus::PARENT, xid, timestamp)?
                .check()?;
            Ok(())
        };
        if let Err(error) = focus() {
            eprintln!("Cannot focus floating window: {error:#}");
        }
    }
}

/// Move and resize a [`floating_window`] (same coordinates as when creating it). It can move
/// to another monitor.
pub fn place_floating(window: &gtk::Window, x: i32, y: i32, width: i32, height: i32) {
    window.set_default_size(width, height);
    window.set_size_request(width, height);
    if !floating_is_layer() {
        window.resize(width, height);
        window.move_(x, y);
        return;
    }
    let Some(display) = gdk::Display::default() else {
        return;
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
        if LayerShell::monitor(window).as_ref() != Some(&monitor) {
            window.set_monitor(&monitor);
        }
        window.set_layer_shell_margin(Edge::Left, gx - area.x());
        window.set_layer_shell_margin(Edge::Top, gy - area.y());
    }
}

/// Floating windows are layer-shell surfaces (on Wayland, where supported), else X11 popups.
fn floating_is_layer() -> bool {
    session::is_wayland() && gtk_layer_shell::is_supported()
}

/// Windows darkening the screen behind a prompt; see [`dim_screen`]. Cheap to clone.
#[derive(Clone)]
pub struct Dim {
    windows: Vec<gtk::Window>,
}

impl Dim {
    /// Keep the dimming under `prompt`, which must be mapped already. Needed on X11, where
    /// window managers stack their windows relative to each other only, so a prompt they
    /// manage would otherwise end up under dimming mapped before it (or after a workspace
    /// switch). Call it from the prompt's map and focus handlers.
    pub fn stay_below(&self, prompt: &gtk::Window) {
        if session::is_wayland() {
            return;
        }
        let Some(frame) = x11_frame(prompt) else {
            return;
        };
        let Ok((conn, _)) = screen::x11_connect() else {
            return;
        };
        for window in &self.windows {
            let Some(xid) = x11_xid(window) else {
                continue;
            };
            let aux = ConfigureWindowAux::new()
                .sibling(frame)
                .stack_mode(StackMode::BELOW);
            let _ = conn.configure_window(xid, &aux);
        }
        let _ = conn.flush();
    }

    pub fn close(&self) {
        for window in &self.windows {
            window.close();
        }
    }
}

/// The X window id of a realized GTK window.
fn x11_xid(window: &gtk::Window) -> Option<u32> {
    extern "C" {
        fn gdk_x11_window_get_xid(window: *mut gdk::ffi::GdkWindow) -> libc::c_ulong;
    }
    let gdk_window = window.window()?;
    // SAFETY: the GDK backend is X11 (checked by the caller) and the window is realized.
    let xid = unsafe { gdk_x11_window_get_xid(gdk_window.to_glib_none().0) };
    u32::try_from(xid).ok()
}

/// The topmost ancestor of `window` below the root: the window manager's frame around
/// it, or the window itself when it isn't reparented.
fn x11_frame(window: &gtk::Window) -> Option<u32> {
    let mut current = x11_xid(window)?;
    let (conn, _) = screen::x11_connect().ok()?;
    loop {
        let tree = conn.query_tree(current).ok()?.reply().ok()?;
        if tree.parent == tree.root || tree.parent == 0 {
            return Some(current);
        }
        current = tree.parent;
    }
}

/// Darken every monitor behind a prompt: always-on-top windows that take no focus,
/// painted translucent black (`alpha`), or solid black without a compositor. They last as
/// long as the returned [`Dim`] does. Show the prompt after this so it stacks above them,
/// and keep it there with [`Dim::stay_below`].
pub fn dim_screen(namespace: &str, alpha: f64) -> Dim {
    let mut windows = Vec::new();
    if floating_is_layer() {
        let Some(display) = gdk::Display::default() else {
            return Dim { windows };
        };
        for index in 0..display.n_monitors() {
            let Some(monitor) = display.monitor(index) else {
                continue;
            };
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            window.init_layer_shell();
            window.set_namespace(namespace);
            window.set_layer(Layer::Overlay);
            window.set_monitor(&monitor);
            for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                window.set_anchor(edge, true);
            }
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::None);
            windows.push(window);
        }
    } else if let Ok((width, height)) = screen::size() {
        let scale = screen::logical_scale();
        let window = gtk::Window::new(gtk::WindowType::Popup);
        window.move_(0, 0);
        let (width, height) = (
            (f64::from(width) / scale).ceil() as i32,
            (f64::from(height) / scale).ceil() as i32,
        );
        window.set_default_size(width, height);
        window.set_size_request(width, height);
        windows.push(window);
    }
    for window in &windows {
        window.set_accept_focus(false);
        window.set_app_paintable(true);
        let translucent =
            WidgetExt::screen(window).is_some_and(|screen| match screen.rgba_visual() {
                Some(visual) if screen.is_composited() => {
                    window.set_visual(Some(&visual));
                    true
                }
                _ => false,
            });
        let alpha = if translucent { alpha } else { 1.0 };
        window.connect_draw(move |_, cr| {
            cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
            cr.set_operator(cairo::Operator::Source);
            let _ = cr.paint();
            glib::Propagation::Stop
        });
        window.show_all();
    }
    Dim { windows }
}

/// A window for a prompt that must be seen and answered right away, such as a password
/// dialog: always on top, centered, and holding the keyboard as soon as it is shown. On
/// Wayland it is a layer-shell surface on the overlay layer with exclusive keyboard focus,
/// on the monitor the compositor considers focused; on X11 (and without layer-shell) a
/// dialog-type toplevel, which window managers float, place by the pointer and focus. Not
/// shown yet.
pub fn dialog_window(namespace: &str, title: &str) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_resizable(false);
    if floating_is_layer() {
        window.init_layer_shell();
        window.set_namespace(namespace);
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        // No anchors: the compositor centers the surface on its monitor.
    } else {
        window.set_type_hint(gdk::WindowTypeHint::Dialog);
        window.set_position(gtk::WindowPosition::Mouse);
        window.set_keep_above(true);
        // Follows workspace switches.
        window.stick();
        window.set_urgency_hint(true);
    }
    window
}
