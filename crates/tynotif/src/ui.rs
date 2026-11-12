use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::bus::{self, Shared};
use crate::model::Notification;

pub fn run(state: Shared, replace: bool, force: bool, test: bool) -> Result<()> {
    gtk::init()?;
    ty_common::appearance::apply_gtk();
    let connection = bus::serve(state.clone(), replace, force)?;
    let config = state.lock().unwrap().config.clone();
    let screen = gdk::Screen::default().context("no screen")?;
    let css = gtk::CssProvider::new();
    css.load_from_data(format!(
        "#tynotif, #tynotif viewport {{ background: {}; color: {}; }} .notification {{ background: {}; color: {}; border: 1px solid {}; border-radius: 10px; padding: 16px; }} .notification.critical {{ border-color: {}; }} .notification label, #tynotif .queue {{ color: {}; }} .app {{ opacity: 0.65; }} .summary {{ font-weight: bold; }}",
        config.background, config.foreground, config.background, config.foreground, config.border, config.critical, config.foreground
    ).as_bytes())?;
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = ty_common::overlay::floating_window("tynotif", 0, 0, config.width, 1);
    window.set_widget_name("tynotif");
    // The shared floating window defaults to an app-painted transparent surface.
    // Let GTK paint an opaque background, including the gaps and newly resized areas.
    window.set_app_paintable(false);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    let stack = gtk::Box::new(gtk::Orientation::Vertical, config.gap);
    scroll.add(&stack);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.pack_start(&scroll, true, true, 0);
    let queued = label("", "queue");
    queued.set_xalign(0.5);
    queued.set_margin_top(8);
    queued.set_margin_bottom(8);
    queued.set_no_show_all(true);
    content.pack_end(&queued, false, false, 0);
    window.add(&content);
    let hovered = Rc::new(Cell::new(false));
    window.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);
    let flag = hovered.clone();
    window.connect_enter_notify_event(move |_, _| {
        flag.set(true);
        glib::Propagation::Proceed
    });
    let flag = hovered.clone();
    window.connect_leave_notify_event(move |_, event| {
        if event.detail() != gdk::NotifyType::Inferior {
            flag.set(false);
        }
        glib::Propagation::Proceed
    });
    let mut revision = u64::MAX;
    let mut last = Instant::now();
    let mut last_area = (0, 0, 0, 0);
    let mut cards: HashMap<u32, (Notification, gtk::Box)> = HashMap::new();
    let startup_connection = connection.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let now = Instant::now();
        let elapsed = now.duration_since(last).as_millis().min(u64::MAX as u128) as u64;
        last = now;
        let mut model = state.lock().unwrap();
        if !hovered.get() {
            for notification in model.tick(elapsed) {
                bus::closed(&connection, &notification, 1);
            }
        }
        let monitor =
            gdk::Display::default().and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)));
        let Some(monitor) = monitor else {
            return glib::ControlFlow::Continue;
        };
        let area = monitor.workarea();
        let geometry = (area.x(), area.y(), area.width(), area.height());
        if revision == model.revision && geometry == last_area {
            return glib::ControlFlow::Continue;
        }
        last_area = geometry;
        revision = model.revision;
        let notifications = model.visible();
        let waiting = model.active.len().saturating_sub(notifications.len());
        drop(model);
        cards.retain(|id, (_, widget)| {
            let keep = notifications.iter().any(|n| n.id == *id);
            if !keep {
                stack.remove(widget);
            }
            keep
        });
        if notifications.is_empty() {
            window.hide();
            hovered.set(false);
            return glib::ControlFlow::Continue;
        }
        let width = config
            .width
            .min((area.width() - 2 * config.margin).max(200));
        for (position, notification) in notifications.into_iter().enumerate() {
            let unchanged = cards
                .get(&notification.id)
                .is_some_and(|(previous, _)| same_content(previous, &notification));
            if !unchanged {
                if let Some((_, widget)) = cards.remove(&notification.id) {
                    stack.remove(&widget);
                }
                let widget = card(&notification, &state, &connection);
                stack.pack_start(&widget, false, false, 0);
                cards.insert(notification.id, (notification.clone(), widget));
            }
            stack.reorder_child(&cards[&notification.id].1, position as i32);
        }
        queued.set_text(&format!(
            "+{waiting} more notification{} queued",
            if waiting == 1 { "" } else { "s" }
        ));
        queued.set_visible(waiting > 0);
        content.show_all();
        let (_, natural) = stack.preferred_height_for_width(width);
        let footer_height = if waiting > 0 {
            queued.preferred_height_for_width(width).1
        } else {
            0
        };
        let height = (natural + footer_height)
            .min((area.height() - config.top - config.margin).max(100))
            .max(1);
        let origin = ty_common::screen::bounds().ok();
        // overlay uses canvas-relative coordinates on Wayland; X11 uses root coordinates.
        let ox = origin.map(|r| r.x).unwrap_or(0);
        let oy = origin.map(|r| r.y).unwrap_or(0);
        // Realize while unmapped so the first visible frame uses the final geometry.
        window.realize();
        ty_common::overlay::place_floating(
            &window,
            area.x() - ox + area.width() - width - config.margin,
            area.y() - oy + config.top,
            width,
            height,
        );
        window.show();
        glib::ControlFlow::Continue
    });
    crate::demo::start(&startup_connection, test);
    gtk::main();
    Ok(())
}

fn same_content(a: &Notification, b: &Notification) -> bool {
    a.app == b.app
        && a.summary == b.summary
        && a.body == b.body
        && a.icon == b.icon
        && a.urgency == b.urgency
        && a.actions == b.actions
        && a.progress == b.progress
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(60);
    label.style_context().add_class(class);
    label
}

fn card(n: &Notification, state: &Shared, connection: &gio::DBusConnection) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.style_context().add_class("notification");
    if n.urgency == 2 {
        card.style_context().add_class("critical");
    }
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    if !n.icon.is_empty() {
        let image = if n.icon.starts_with('/') {
            gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&n.icon, 48, 48, true)
                .ok()
                .map(|p| gtk::Image::from_pixbuf(Some(&p)))
        } else {
            let image = gtk::Image::from_icon_name(Some(&n.icon), gtk::IconSize::Dialog);
            image.set_pixel_size(48);
            Some(image)
        };
        if let Some(image) = image {
            header.pack_start(&image, false, false, 0);
        }
    }
    header.pack_start(&label(&n.app, "app"), true, true, 0);
    let dismiss = gtk::Button::with_label("×");
    dismiss.set_tooltip_text(Some("Dismiss notification"));
    let model = state.clone();
    let bus = connection.clone();
    let id = n.id;
    dismiss.connect_clicked(move |_| bus::close(&bus, &mut model.lock().unwrap(), id, 2));
    header.pack_end(&dismiss, false, false, 0);
    card.pack_start(&header, false, false, 0);
    card.pack_start(&label(&n.summary, "summary"), false, false, 0);
    if !n.body.is_empty() {
        let body = label(&n.body, "body");
        body.set_lines(8);
        body.set_ellipsize(gtk::pango::EllipsizeMode::End);
        card.pack_start(&body, false, false, 0);
    }
    if let Some(value) = n.progress {
        let progress = gtk::ProgressBar::new();
        progress.set_fraction(f64::from(value) / 100.0);
        card.pack_start(&progress, false, false, 0);
    }
    // Vertical buttons keep long/localized action names within the popup width.
    for pair in n.actions.chunks_exact(2) {
        let button = gtk::Button::new();
        button.add(&label(&pair[1], "action"));
        let key = pair[0].clone();
        let model = state.clone();
        let bus = connection.clone();
        button.connect_clicked(move |_| {
            if let Err(error) = bus::action(&bus, &mut model.lock().unwrap(), id, &key) {
                eprintln!("{error}");
            }
        });
        card.pack_start(&button, false, false, 0);
    }
    card
}
