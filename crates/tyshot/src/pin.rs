//! Pinned screenshots, like Flameshot's pin: the capture stays on screen in a borderless,
//! always-on-top window where it was taken, to keep a reference in view while you work.
//!
//! - Drag to move it, scroll to zoom (around the pointer), Ctrl+scroll to fade it.
//! - Double-click or middle-click closes it; right-click opens a menu (copy, save, original
//!   size, close).
//! - On Wayland it takes keyboard focus when clicked: Esc closes, Ctrl+C copies, Ctrl+S
//!   saves, `0` resets the size and `+` / `-` zoom. On X11 it is an override-redirect window
//!   (so tiling window managers leave it alone), which can't take focus: use the menu there.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use ty_common::overlay::{floating_window, place_floating, set_floating_focusable};
use ty_common::session;

use crate::overlay::{Action, Capture};

const ZOOM_STEP: f64 = 1.1;
const MIN_ZOOM: f64 = 0.1;
const MAX_ZOOM: f64 = 8.0;
/// Smallest the pin can be zoomed out to, logical px.
const MIN_SIZE: f64 = 24.0;
const OPACITY_STEP: f64 = 0.1;
const MIN_OPACITY: f64 = 0.2;

/// Handles Copy and Save from a pin.
type OnAction = Rc<dyn Fn(Action, &Pixbuf)>;

struct Pin {
    window: gtk::Window,
    image: Pixbuf,
    /// Size at zoom 1, logical px.
    base: (f64, f64),
    zoom: f64,
    opacity: f64,
    /// Top-left corner on the overlay canvas, logical px.
    position: (f64, f64),
    drag: Option<Drag>,
    accent: (f64, f64, f64),
}

/// A move in progress, from where the button went down.
struct Drag {
    /// Pointer position in the window.
    local: (f64, f64),
    /// Pointer position on the screen, and the pin's position then (X11 only; Wayland has no
    /// global pointer position).
    root: (f64, f64),
    start: (f64, f64),
}

impl Pin {
    fn size(&self) -> (f64, f64) {
        (self.base.0 * self.zoom, self.base.1 * self.zoom)
    }

    fn place(&self) {
        let (width, height) = self.size();
        place_floating(
            &self.window,
            self.position.0.round() as i32,
            self.position.1.round() as i32,
            width.round().max(1.0) as i32,
            height.round().max(1.0) as i32,
        );
        self.window.queue_draw();
    }

    /// Zoom by `factor`, keeping the point at `anchor` (window coordinates) still.
    fn zoom_by(&mut self, factor: f64, anchor: (f64, f64)) {
        let min = (MIN_SIZE / self.base.0.min(self.base.1)).max(MIN_ZOOM);
        let zoom = (self.zoom * factor).clamp(min.min(1.0), MAX_ZOOM);
        let k = zoom / self.zoom;
        self.position.0 += anchor.0 * (1.0 - k);
        self.position.1 += anchor.1 * (1.0 - k);
        self.zoom = zoom;
        self.place();
    }

    fn center(&self) -> (f64, f64) {
        let (width, height) = self.size();
        (width / 2.0, height / 2.0)
    }
}

/// Show `capture`'s image pinned at its origin. `on_action` handles Copy and Save from the
/// pin (it stays open); `on_close` runs once it is closed.
pub fn show(
    capture: Capture,
    accent: (f64, f64, f64),
    on_action: impl Fn(Action, &Pixbuf) + 'static,
    on_close: impl FnOnce() + 'static,
) {
    let Capture {
        image,
        origin,
        scale,
        ..
    } = capture;
    let base = (
        f64::from(image.width()) / scale,
        f64::from(image.height()) / scale,
    );
    let window = floating_window(
        "tyshot-pin",
        origin.0.round() as i32,
        origin.1.round() as i32,
        base.0.round() as i32,
        base.1.round() as i32,
    );
    set_floating_focusable(&window);
    window.set_title("tyshot pin");
    // Transparent where the image is (rounded frame corners) and for fading.
    if let Some(visual) = WidgetExt::screen(&window).and_then(|screen| screen.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::KEY_PRESS_MASK,
    );

    let pin = Rc::new(RefCell::new(Pin {
        window: window.clone(),
        image,
        base,
        zoom: 1.0,
        opacity: 1.0,
        position: origin,
        drag: None,
        accent,
    }));
    let on_action: OnAction = Rc::new(on_action);
    let menu = build_menu(&window, &pin, &on_action);

    {
        let pin = pin.clone();
        window.connect_draw(move |widget, cr| {
            draw(
                cr,
                &pin.borrow(),
                widget.allocated_width(),
                widget.allocated_height(),
            );
            glib::Propagation::Stop
        });
    }
    {
        let pin = pin.clone();
        window.connect_button_press_event(move |window, event| {
            match (event.event_type(), event.button()) {
                (gdk::EventType::DoubleButtonPress, 1) | (_, 2) => window.close(),
                (gdk::EventType::ButtonPress, 1) => {
                    let mut p = pin.borrow_mut();
                    p.drag = Some(Drag {
                        local: event.position(),
                        root: event.root(),
                        start: p.position,
                    });
                }
                (gdk::EventType::ButtonPress, 3) => menu.popup_at_pointer(Some(event)),
                _ => {}
            }
            glib::Propagation::Stop
        });
    }
    {
        let pin = pin.clone();
        window.connect_button_release_event(move |_, event| {
            if event.button() == 1 {
                pin.borrow_mut().drag = None;
            }
            glib::Propagation::Stop
        });
    }
    {
        let pin = pin.clone();
        window.connect_motion_notify_event(move |_, event| {
            let mut p = pin.borrow_mut();
            let Some(drag) = &p.drag else {
                return glib::Propagation::Proceed;
            };
            if !event.state().contains(gdk::ModifierType::BUTTON1_MASK) {
                p.drag = None;
                return glib::Propagation::Proceed;
            }
            p.position = if session::is_wayland() {
                // Keep the grabbed point under the pointer.
                let (x, y) = event.position();
                (
                    p.position.0 + x - drag.local.0,
                    p.position.1 + y - drag.local.1,
                )
            } else {
                // Logical px, like the canvas.
                let (x, y) = event.root();
                (
                    drag.start.0 + x - drag.root.0,
                    drag.start.1 + y - drag.root.1,
                )
            };
            p.place();
            glib::Propagation::Stop
        });
    }
    {
        let pin = pin.clone();
        window.connect_scroll_event(move |_, event| {
            let up = match event.direction() {
                gdk::ScrollDirection::Up => true,
                gdk::ScrollDirection::Down => false,
                gdk::ScrollDirection::Smooth => match event.delta() {
                    (_, dy) if dy < 0.0 => true,
                    (_, dy) if dy > 0.0 => false,
                    _ => return glib::Propagation::Stop,
                },
                _ => return glib::Propagation::Stop,
            };
            let mut p = pin.borrow_mut();
            if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
                let step = if up { OPACITY_STEP } else { -OPACITY_STEP };
                p.opacity = (p.opacity + step).clamp(MIN_OPACITY, 1.0);
                p.window.queue_draw();
            } else {
                let factor = if up { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
                p.zoom_by(factor, event.position());
            }
            glib::Propagation::Stop
        });
    }
    {
        let pin = pin.clone();
        let on_action = on_action.clone();
        window.connect_key_press_event(move |window, event| {
            use gdk::keys::constants as key;
            let ctrl = event.state().contains(gdk::ModifierType::CONTROL_MASK);
            match event.keyval().to_lower() {
                key::Escape => window.close(),
                key::c if ctrl => on_action(Action::Copy, &pin.borrow().image),
                key::s if ctrl => on_action(Action::Save, &pin.borrow().image),
                key::_0 | key::KP_0 => reset_size(&pin),
                key::plus | key::equal | key::KP_Add => {
                    let mut p = pin.borrow_mut();
                    let center = p.center();
                    p.zoom_by(ZOOM_STEP, center);
                }
                key::minus | key::KP_Subtract => {
                    let mut p = pin.borrow_mut();
                    let center = p.center();
                    p.zoom_by(1.0 / ZOOM_STEP, center);
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }

    let on_close = RefCell::new(Some(on_close));
    window.connect_destroy(move |_| {
        if let Some(on_close) = on_close.take() {
            on_close();
        }
    });
    window.show_all();
}

fn reset_size(pin: &Rc<RefCell<Pin>>) {
    let mut p = pin.borrow_mut();
    let center = p.center();
    let factor = 1.0 / p.zoom;
    p.zoom_by(factor, center);
}

fn build_menu(window: &gtk::Window, pin: &Rc<RefCell<Pin>>, on_action: &OnAction) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_attach_widget(Some(window));
    let item = |label: &str, run: Box<dyn Fn()>| {
        let item = gtk::MenuItem::with_label(label);
        item.connect_activate(move |_| run());
        menu.append(&item);
    };
    for (label, action) in [("Copy", Action::Copy), ("Save", Action::Save)] {
        let (pin, on_action) = (pin.clone(), on_action.clone());
        item(
            label,
            Box::new(move || on_action(action.clone(), &pin.borrow().image)),
        );
    }
    {
        let pin = pin.clone();
        item("Original size", Box::new(move || reset_size(&pin)));
    }
    menu.append(&gtk::SeparatorMenuItem::new());
    {
        let window = window.clone();
        item("Close", Box::new(move || window.close()));
    }
    menu.show_all();
    menu
}

fn draw(cr: &cairo::Context, pin: &Pin, width: i32, height: i32) {
    let (width, height) = (f64::from(width), f64::from(height));
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    cr.save().ok();
    cr.scale(
        width / f64::from(pin.image.width()),
        height / f64::from(pin.image.height()),
    );
    cr.set_source_pixbuf(&pin.image, 0.0, 0.0);
    if let Ok(pattern) = cairo::SurfacePattern::try_from(cr.source()) {
        pattern.set_filter(cairo::Filter::Good);
    }
    let _ = cr.paint_with_alpha(pin.opacity);
    cr.restore().ok();

    // A thin outline, so the pin doesn't pass for the real screen under it.
    let (r, g, b) = pin.accent;
    cr.set_source_rgba(r, g, b, 0.85 * pin.opacity);
    cr.set_line_width(1.0);
    cr.rectangle(0.5, 0.5, width - 1.0, height - 1.0);
    let _ = cr.stroke();
}
