use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    let mut tray = match crate::tray::Tray::new(state.clone(), connection.clone()) {
        Ok(tray) => Some(tray),
        Err(error) => {
            eprintln!("Cannot create notification tray icon: {error:#}");
            None
        }
    };
    let config = state.lock().unwrap().config.clone();
    let screen = gdk::Screen::default().context("no screen")?;
    let css = gtk::CssProvider::new();
    css.load_from_data(format!(
        "#tynotif, #tynotif viewport, #tynotif scrolledwindow, #tynotif scrolledwindow undershoot, #tynotif scrolledwindow overshoot {{ background: transparent; border: none; outline: none; box-shadow: none; }} #tynotif .notification, #tynotif .queue {{ background: {}; color: {}; border: 1px solid alpha({}, 0.65); border-image: none; box-shadow: none; border-radius: 10px; padding: 12px 14px; }} #tynotif .notification.critical {{ border-color: {}; }} #tynotif .notification label {{ color: {}; }} .app {{ opacity: 0.65; }} .summary {{ font-weight: bold; }} #tynotif button.dismiss {{ background: transparent; background-image: none; border: none; box-shadow: none; min-width: 18px; min-height: 18px; padding: 0; }} #tynotif button.dismiss:hover {{ background: alpha(currentColor, 0.12); }} #tynotif .actions button {{ min-height: 18px; padding: 6px 12px; }} #tynotif .actions flowboxchild {{ padding: 0; }} #tynotif button.dismiss-all {{ background: {}; color: {}; border: 1px solid {}; border-radius: 8px; box-shadow: none; min-height: 20px; padding: 4px 10px; }} #tynotif button.dismiss-all:hover {{ border-color: currentColor; }}",
        config.background, config.foreground, config.border, config.critical, config.foreground, config.background, config.foreground, config.border
    ).as_bytes())?;
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let hover = Rc::new(RefCell::new(Hover::default()));
    // Cards have separate native windows. There is no viewport or shared surface
    // spanning their gaps, and no scroller that can paint a rectangular backdrop.
    let footer = popup(config.width, 0, &hover);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let reveal = gtk::Revealer::new();
    reveal.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    reveal.set_transition_duration(160);
    let dismiss_all = gtk::Button::with_label("Dismiss all");
    dismiss_all.style_context().add_class("dismiss-all");
    dismiss_all.set_tooltip_text(Some("Dismiss all visible and queued notifications"));
    let model = state.clone();
    let bus = connection.clone();
    dismiss_all.connect_clicked(move |_| bus::close_all(&bus, &mut model.lock().unwrap()));
    reveal.add(&dismiss_all);
    toolbar.pack_end(&reveal, false, false, 0);
    let queued = label("", "queue");
    queued.set_line_wrap(false);
    queued.set_ellipsize(gtk::pango::EllipsizeMode::End);
    queued.set_max_width_chars(26);
    queued.set_no_show_all(true);
    toolbar.pack_start(&queued, false, false, 0);
    footer.add(&toolbar);
    toolbar.show_all();
    let mut revision = u64::MAX;
    let mut last = Instant::now();
    let mut last_area = (0, 0, 0, 0);
    let mut cards: HashMap<u32, Card> = HashMap::new();
    let mut presented = Vec::new();
    let mut total = 0;
    let mut waiting = 0;
    let mut paused = false;
    let startup_connection = connection.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let now = Instant::now();
        let elapsed = now.duration_since(last).as_millis().min(u64::MAX as u128) as u64;
        last = now;
        let mut model = state.lock().unwrap();
        model.wake_if_due(std::time::SystemTime::now());
        if !hover.borrow().active() {
            for notification in model.tick_visible(elapsed, &presented) {
                bus::closed(&connection, &notification, 1);
            }
        }
        if let Some(tray) = &mut tray {
            tray.refresh(&model);
        }
        let monitor =
            gdk::Display::default().and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)));
        let Some(monitor) = monitor else {
            for card in cards.values() {
                card.window.hide();
            }
            footer.hide();
            presented.clear();
            *hover.borrow_mut() = Hover::default();
            revision = u64::MAX;
            return glib::ControlFlow::Continue;
        };
        let area = monitor.workarea();
        let geometry = (area.x(), area.y(), area.width(), area.height());
        if revision != model.revision || geometry != last_area {
            last_area = geometry;
            revision = model.revision;
            let notifications = model.visible();
            total = model.active.len();
            paused = model.paused;
            drop(model);
            cards.retain(|id, card| {
                let keep = notifications.iter().any(|n| n.id == *id);
                if !keep {
                    hover.borrow_mut().over.remove(id);
                    card.window.close();
                }
                keep
            });
            let width = config
                .width
                .min((area.width() - 2 * config.margin).max(200));
            let origin = ty_common::screen::bounds().ok();
            let x =
                area.x() - origin.map(|r| r.x).unwrap_or(0) + area.width() - width - config.margin;
            let mut y = area.y() - origin.map(|r| r.y).unwrap_or(0) + config.top;
            let bottom =
                area.y() - origin.map(|r| r.y).unwrap_or(0) + area.height() - config.margin;
            dismiss_all.set_label(&format!("Dismiss all ({total})"));
            // Keep room for a queue badge and hover control at the bottom. Measure
            // their full height, not the revealer's animated height.
            queued.set_text("More notifications queued");
            queued.show();
            let footer_height = dismiss_all
                .preferred_height_for_width(width)
                .1
                .max(queued.preferred_height_for_width(width).1)
                .max(36);
            let budget_bottom = bottom - footer_height - config.gap;
            presented.clear();
            for notification in notifications {
                let id = notification.id;
                let entry = cards.entry(id).or_insert_with(|| {
                    let window = popup(width, id, &hover);
                    let child = card(&notification, &state, &connection);
                    window.add(&child);
                    Card {
                        window,
                        child,
                        notification: notification.clone(),
                    }
                });
                if !same_content(&entry.notification, &notification) {
                    entry.window.remove(&entry.child);
                    entry.child = card(&notification, &state, &connection);
                    entry.window.add(&entry.child);
                    entry.notification = notification;
                }
                entry.child.show_all();
                let height = entry.child.preferred_height_for_width(width).1.max(1);
                if y + height > budget_bottom && !presented.is_empty() {
                    entry.window.hide();
                    hover.borrow_mut().over.remove(&id);
                    continue;
                }
                place(&entry.window, x, y, width, height);
                entry.window.show();
                entry.window.queue_draw();
                presented.push(id);
                y += height + config.gap;
            }
            waiting = total.saturating_sub(presented.len());
            queued.set_text(&format!("+{waiting} more queued"));
            queued.set_visible(waiting > 0);
            // A full-width transparent footer still has a rectangular compositor
            // shadow and input region. Size it to the badge/button, right-aligned.
            // Use the full button width even while its reveal animation is hidden.
            let button_width = if total > 1 {
                dismiss_all.preferred_width().1
            } else {
                0
            };
            let badge_width = if waiting > 0 {
                queued.preferred_width().1
            } else {
                0
            };
            let spacing = if button_width > 0 && badge_width > 0 {
                8
            } else {
                0
            };
            let footer_width = (button_width + badge_width + spacing).max(1).min(width);
            place(
                &footer,
                x + width - footer_width,
                y.min(bottom - footer_height),
                footer_width,
                footer_height,
            );
            if presented.is_empty() {
                *hover.borrow_mut() = Hover::default();
            }
        } else {
            drop(model);
        }
        let show_control = total > 1 && !paused && hover.borrow().active();
        reveal.set_reveal_child(show_control);
        if !paused
            && !presented.is_empty()
            && (waiting > 0 || show_control || reveal.is_child_revealed())
        {
            footer.show();
        } else {
            footer.hide();
            hover.borrow_mut().over.remove(&0);
        }
        glib::ControlFlow::Continue
    });
    crate::demo::start(&startup_connection, test);
    gtk::main();
    Ok(())
}

struct Card {
    window: gtk::Window,
    child: gtk::Box,
    notification: Notification,
}

#[derive(Default)]
struct Hover {
    over: HashSet<u32>,
    left_at: Option<Instant>,
}

impl Hover {
    fn active(&self) -> bool {
        !self.over.is_empty()
            || self
                .left_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(250))
    }
}

fn popup(width: i32, id: u32, hover: &Rc<RefCell<Hover>>) -> gtk::Window {
    let window = ty_common::overlay::floating_window("tynotif", 0, 0, width, 1);
    window.set_widget_name("tynotif");
    window.set_title("tynotif");
    window.set_type_hint(gdk::WindowTypeHint::Notification);
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    if let Some(visual) = gdk::Screen::default().and_then(|screen| screen.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.connect_draw(|_, cr| {
        let _ = cr.save();
        cr.set_operator(gtk::cairo::Operator::Clear);
        let _ = cr.paint();
        let _ = cr.restore();
        glib::Propagation::Proceed
    });
    window.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);
    let state = hover.clone();
    window.connect_enter_notify_event(move |_, _| {
        state.borrow_mut().over.insert(id);
        glib::Propagation::Proceed
    });
    let state = hover.clone();
    window.connect_leave_notify_event(move |_, event| {
        if event.detail() != gdk::NotifyType::Inferior {
            let mut state = state.borrow_mut();
            if state.over.remove(&id) {
                state.left_at = Some(Instant::now());
            }
        }
        glib::Propagation::Proceed
    });
    window
}

fn place(window: &gtk::Window, x: i32, y: i32, width: i32, height: i32) {
    window.realize();
    ty_common::overlay::place_floating(window, x, y, width, height);
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
    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.pack_start(&text, true, true, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let app = label(&n.app, "app");
    app.set_line_wrap(false);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    app.set_max_width_chars(40);
    app.set_tooltip_text(Some(&n.app));
    header.pack_start(&app, true, true, 0);
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
    if !n.summary.is_empty() {
        let summary = label(&n.summary, "summary");
        summary.set_lines(2);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.set_tooltip_text(Some(&n.summary));
        text.pack_start(&summary, false, false, 0);
    }
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
        progress.set_margin_top(4);
        progress.set_margin_bottom(2);
        text.pack_start(&progress, false, false, 0);
    }
    // Share a row when possible; wrap long/localized actions onto further rows.
    let actions = gtk::FlowBox::new();
    actions.style_context().add_class("actions");
    actions.set_selection_mode(gtk::SelectionMode::None);
    actions.set_min_children_per_line(1);
    actions.set_max_children_per_line(3);
    actions.set_margin_top(4);
    actions.set_column_spacing(8);
    actions.set_row_spacing(6);
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
