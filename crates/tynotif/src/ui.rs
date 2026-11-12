use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::bus::{self, Shared};
use crate::model::Notification;

pub fn run(state: Shared, replace: bool, force: bool, test: Option<Duration>) -> Result<()> {
    gtk::init()?;
    gdk::set_program_class("Tynotif");
    ty_common::appearance::apply_gtk();
    let connection = bus::serve(state.clone(), replace, force)?;
    let config = state.lock().unwrap().config.clone();
    let screen = gdk::Screen::default().context("no screen")?;
    let css = gtk::CssProvider::new();
    css.load_from_data(format!(
        "#tynotif, #tynotif viewport, #tynotif scrolledwindow {{ background: transparent; box-shadow: none; }} .notification, #tynotif .queue {{ background: {}; color: {}; border: 1px solid {}; border-radius: 10px; padding: 10px 12px; }} .notification.critical {{ border-color: {}; }} .notification label {{ color: {}; }} .app {{ opacity: 0.65; }} .summary {{ font-weight: bold; }} #tynotif button.dismiss {{ background: transparent; background-image: none; border: none; box-shadow: none; min-width: 18px; min-height: 18px; padding: 0; }} #tynotif button.dismiss:hover {{ background: alpha(currentColor, 0.12); }} #tynotif .actions button {{ min-height: 0; padding: 3px 8px; }} #tynotif .actions flowboxchild {{ padding: 0; }}",
        config.background, config.foreground, config.border, config.critical, config.foreground
    ).as_bytes())?;
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = ty_common::overlay::floating_window("tynotif", 0, 0, config.width, 1);
    window.set_widget_name("tynotif");
    window.set_title("tynotif");
    // Compositors must treat this as an overlay, not as a normal app window.
    // In particular, Picom's normal-window transparent clipping cuts through
    // underlying windows to the wallpaper at our gaps and antialiased corners.
    window.set_type_hint(gdk::WindowTypeHint::Notification);
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    if let Some(visual) = screen.rgba_visual() {
        window.set_visual(Some(&visual));
    }
    // Clear damaged pixels to transparent before GTK draws the opaque cards. Merely
    // setting transparent CSS on an app-painted window leaves old pixels behind.
    window.connect_draw(|_, cr| {
        let _ = cr.save();
        cr.set_operator(gtk::cairo::Operator::Clear);
        let _ = cr.paint();
        let _ = cr.restore();
        glib::Propagation::Proceed
    });
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
        window.queue_draw();
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
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    card.style_context().add_class("notification");
    if n.urgency == 2 {
        card.style_context().add_class("critical");
    }
    if !n.icon.is_empty() {
        let image = if n.icon.starts_with('/') {
            gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&n.icon, 24, 24, true)
                .ok()
                .map(|p| gtk::Image::from_pixbuf(Some(&p)))
        } else {
            let image = gtk::Image::from_icon_name(Some(&n.icon), gtk::IconSize::Dialog);
            image.set_pixel_size(24);
            Some(image)
        };
        if let Some(image) = image {
            image.set_valign(gtk::Align::Start);
            card.pack_start(&image, false, false, 0);
        }
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.pack_start(&text, true, true, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let app = label(&n.app, "app");
    app.set_line_wrap(false);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    app.set_max_width_chars(14);
    app.set_tooltip_text(Some(&n.app));
    header.pack_start(&app, false, false, 0);
    let summary = label(&n.summary, "summary");
    summary.set_line_wrap(false);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_tooltip_text(Some(&n.summary));
    header.pack_start(&summary, true, true, 0);
    let dismiss = gtk::Button::with_label("×");
    dismiss.style_context().add_class("dismiss");
    dismiss.set_valign(gtk::Align::Start);
    dismiss.set_tooltip_text(Some("Dismiss notification"));
    let model = state.clone();
    let bus = connection.clone();
    let id = n.id;
    dismiss.connect_clicked(move |_| bus::close(&bus, &mut model.lock().unwrap(), id, 2));
    header.pack_end(&dismiss, false, false, 0);
    text.pack_start(&header, false, false, 0);
    if !n.body.is_empty() {
        let body = label(&n.body, "body");
        body.set_lines(3);
        body.set_ellipsize(gtk::pango::EllipsizeMode::End);
        body.set_tooltip_text(Some(&n.body));
        text.pack_start(&body, false, false, 0);
    }
    if let Some(value) = n.progress {
        let progress = gtk::ProgressBar::new();
        progress.set_fraction(f64::from(value) / 100.0);
        text.pack_start(&progress, false, false, 0);
    }
    // Share a row when possible; wrap long/localized actions onto further rows.
    let actions = gtk::FlowBox::new();
    actions.style_context().add_class("actions");
    actions.set_selection_mode(gtk::SelectionMode::None);
    actions.set_min_children_per_line(1);
    actions.set_max_children_per_line(3);
    actions.set_column_spacing(4);
    actions.set_row_spacing(4);
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
        actions.insert(&button, -1);
    }
    if !n.actions.is_empty() {
        text.pack_start(&actions, false, false, 0);
    }
    card
}
