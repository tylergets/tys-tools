//! A small window graphing the last and next 12 hours: the scheduled temperature and
//! dimming (dashed), what was applied (solid), monitor brightness and ambient light.

use std::cell::RefCell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Local, Offset, TimeZone};
use gtk::cairo::{self, Context, FontSlant, FontWeight};
use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;

use crate::daemon::Shared;
use crate::schedule::{self, Sample};
use crate::solar;

const HALF_SPAN: f64 = 12.0 * 3600.0;
const REDRAW: Duration = Duration::from_secs(5);
const MIN_K: f64 = 1500.0;
const MAX_K: f64 = 7000.0;
/// Lux mapped to the percent axis on a log scale up to this.
const MAX_LUX: f64 = 100_000.0;

type Rgb = (f64, f64, f64);
const BG: Rgb = (0.11, 0.11, 0.12);
const FG: Rgb = (0.83, 0.82, 0.82);
const GRID: Rgb = (0.25, 0.25, 0.27);
const TEMP: Rgb = (0.96, 0.60, 0.26);
const DIM: Rgb = (0.96, 0.85, 0.40);
const MONITOR: Rgb = (0.37, 0.66, 0.96);
const LUX: Rgb = (0.60, 0.60, 0.62);

thread_local! {
    static WINDOW: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

/// Show the graph window (or raise it). Must run on the GTK thread.
pub fn open(shared: &Shared) {
    if let Some(window) = WINDOW.with(|w| w.borrow().clone()) {
        window.present();
        return;
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("tylight");
    window.set_default_size(760, 340);
    let area = gtk::DrawingArea::new();
    window.add(&area);

    let draw_shared = shared.clone();
    area.connect_draw(move |widget, cr| {
        let (w, h) = (
            f64::from(widget.allocated_width()),
            f64::from(widget.allocated_height()),
        );
        if let Err(err) = draw(cr, w, h, &draw_shared) {
            eprintln!("graph: {err}");
        }
        glib::Propagation::Stop
    });

    let weak = area.downgrade();
    glib::timeout_add_local(REDRAW, move || match weak.upgrade() {
        Some(area) => {
            area.queue_draw();
            ControlFlow::Continue
        }
        None => ControlFlow::Break,
    });

    window.connect_destroy(|_| WINDOW.with(|w| *w.borrow_mut() = None));
    window.show_all();
    WINDOW.with(|w| *w.borrow_mut() = Some(window));
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

fn set(cr: &Context, (r, g, b): Rgb, alpha: f64) {
    cr.set_source_rgba(r, g, b, alpha);
}

struct Plot {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
    start: f64,
    end: f64,
}

impl Plot {
    fn x(&self, unix: f64) -> f64 {
        self.x0 + (unix - self.start) / (self.end - self.start) * (self.x1 - self.x0)
    }
    /// 0..=100 on the left axis.
    fn y_percent(&self, percent: f64) -> f64 {
        self.y1 - percent.clamp(0.0, 100.0) / 100.0 * (self.y1 - self.y0)
    }
    fn y_kelvin(&self, kelvin: f64) -> f64 {
        self.y_percent((kelvin - MIN_K) / (MAX_K - MIN_K) * 100.0)
    }
}

fn lux_percent(lux: f64) -> f64 {
    (lux.max(0.0) + 1.0).log10() / (MAX_LUX + 1.0).log10() * 100.0
}

/// Stroke `points`, breaking the line at `None`.
fn polyline(
    cr: &Context,
    points: impl Iterator<Item = Option<(f64, f64)>>,
) -> Result<(), cairo::Error> {
    let mut drawing = false;
    for point in points {
        match point {
            Some((x, y)) if drawing => cr.line_to(x, y),
            Some((x, y)) => {
                cr.move_to(x, y);
                drawing = true;
            }
            None => drawing = false,
        }
    }
    cr.stroke()
}

fn draw(cr: &Context, w: f64, h: f64, shared: &Shared) -> Result<(), cairo::Error> {
    let status = shared.status();
    let config = &shared.config;
    let now = unix_now();
    let plot = Plot {
        x0: 48.0,
        x1: w - 58.0,
        y0: 48.0,
        y1: h - 30.0,
        start: now - HALF_SPAN,
        end: now + HALF_SPAN,
    };

    set(cr, BG, 1.0);
    cr.paint()?;
    cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(11.0);

    // Night shading.
    if let Some(location) = status.location {
        let mut x = plot.x0;
        while x < plot.x1 {
            let t = plot.start + (x - plot.x0) / (plot.x1 - plot.x0) * (plot.end - plot.start);
            let progress = solar::day_progress(solar::elevation(t, location), &config.elevation);
            set(cr, (0.25, 0.35, 0.75), 0.22 * (1.0 - progress));
            cr.rectangle(x, plot.y0, 2.0, plot.y1 - plot.y0);
            cr.fill()?;
            x += 2.0;
        }
    }

    // Grid and axis labels.
    cr.set_line_width(1.0);
    for percent in [0.0, 25.0, 50.0, 75.0, 100.0] {
        let y = plot.y_percent(percent);
        set(cr, GRID, 1.0);
        cr.move_to(plot.x0, y);
        cr.line_to(plot.x1, y);
        cr.stroke()?;
        set(cr, FG, 0.7);
        cr.move_to(8.0, y + 4.0);
        cr.show_text(&format!("{percent:.0}%"))?;
        let kelvin = MIN_K + percent / 100.0 * (MAX_K - MIN_K);
        cr.move_to(plot.x1 + 6.0, y + 4.0);
        cr.show_text(&format!("{:.0}K", (kelvin / 100.0).round() * 100.0))?;
    }
    let offset = f64::from(Local::now().offset().fix().local_minus_utc());
    let mut hour = ((plot.start + offset) / 3600.0).ceil() * 3600.0 - offset;
    while hour <= plot.end {
        let local_hour = (((hour + offset) / 3600.0).round() as i64).rem_euclid(24);
        if local_hour % 3 == 0 {
            let x = plot.x(hour);
            set(cr, GRID, 1.0);
            cr.move_to(x, plot.y0);
            cr.line_to(x, plot.y1);
            cr.stroke()?;
            set(cr, FG, 0.7);
            let label = Local
                .timestamp_opt(hour as i64, 0)
                .single()
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_default();
            cr.move_to(x - 15.0, plot.y1 + 18.0);
            cr.show_text(&label)?;
        }
        hour += 3600.0;
    }

    // Schedule, dashed, across the whole span.
    let steps = 288;
    let scheduled: Vec<(f64, crate::color::Setting)> = (0..=steps)
        .map(|i| {
            let t = plot.start + f64::from(i) / f64::from(steps) * (plot.end - plot.start);
            (t, schedule::scheduled(config, status.location, t).0)
        })
        .collect();
    cr.set_line_width(1.5);
    cr.set_dash(&[5.0, 4.0], 0.0);
    set(cr, TEMP, 0.8);
    polyline(
        cr,
        scheduled
            .iter()
            .map(|(t, s)| Some((plot.x(*t), plot.y_kelvin(s.temperature)))),
    )?;
    set(cr, DIM, 0.8);
    polyline(
        cr,
        scheduled
            .iter()
            .map(|(t, s)| Some((plot.x(*t), plot.y_percent(s.brightness * 100.0)))),
    )?;
    cr.set_dash(&[], 0.0);

    // History, solid.
    let history: Vec<Sample> = shared
        .history
        .lock()
        .expect("history lock")
        .samples()
        .filter(|s| s.unix >= plot.start)
        .copied()
        .collect();
    // Break lines across gaps (tylight not running).
    let gap = |i: usize| i > 0 && history[i].unix - history[i - 1].unix > 180.0;
    let series = |value: &dyn Fn(&Sample) -> Option<f64>| {
        history
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let point = value(s).map(|y| (plot.x(s.unix), y));
                [gap(i).then_some(None), Some(point)].into_iter().flatten()
            })
            .collect::<Vec<_>>()
    };
    cr.set_line_width(1.0);
    set(cr, LUX, 0.8);
    polyline(
        cr,
        series(&|s| s.lux.map(|l| plot.y_percent(lux_percent(l)))).into_iter(),
    )?;
    cr.set_line_width(2.0);
    set(cr, TEMP, 1.0);
    polyline(
        cr,
        series(&|s| Some(plot.y_kelvin(f64::from(s.temperature)))).into_iter(),
    )?;
    set(cr, DIM, 1.0);
    polyline(
        cr,
        series(&|s| Some(plot.y_percent(s.brightness * 100.0))).into_iter(),
    )?;
    set(cr, MONITOR, 1.0);
    polyline(
        cr,
        series(&|s| s.monitor.map(|m| plot.y_percent(f64::from(m)))).into_iter(),
    )?;

    // Now.
    set(cr, FG, 0.9);
    cr.set_line_width(1.0);
    cr.move_to(plot.x(now), plot.y0 - 4.0);
    cr.line_to(plot.x(now), plot.y1);
    cr.stroke()?;

    // Header: current state and legend.
    set(cr, FG, 1.0);
    cr.set_font_size(13.0);
    cr.move_to(plot.x0, 20.0);
    let mut header = status.summary();
    if let Some(percent) = status.monitor_brightness {
        header.push_str(&format!(" · monitor {percent}%"));
    }
    if let Some(lux) = status.ambient_lux {
        header.push_str(&format!(" · {lux:.0} lux"));
    }
    cr.show_text(&header)?;
    cr.set_font_size(11.0);
    let mut x = plot.x0;
    for (color, label) in [
        (TEMP, "temperature"),
        (DIM, "dimming"),
        (MONITOR, "monitor"),
        (LUX, "light (log)"),
    ] {
        set(cr, color, 1.0);
        cr.rectangle(x, 30.0, 10.0, 3.0);
        cr.fill()?;
        set(cr, FG, 0.8);
        cr.move_to(x + 14.0, 35.0);
        cr.show_text(label)?;
        x += 14.0 + cr.text_extents(label)?.x_advance() + 18.0;
    }
    set(cr, FG, 0.5);
    let note = "dashed: schedule";
    let width = cr.text_extents(note)?.x_advance();
    cr.move_to(plot.x1 - width, 35.0);
    cr.show_text(note)?;
    Ok(())
}
