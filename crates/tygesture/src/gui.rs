//! A small window that draws gestures as you make them: the swipe trail against its
//! threshold, the pinch scale and rotation, recent firings and the bindings.

use std::cell::RefCell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use gtk::cairo::{self, Context, FontWeight};
use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;

use crate::state::{Live, Shared, Status};
use crate::APP;

const REDRAW: Duration = Duration::from_millis(16);
/// How long a finished gesture stays on the pad, fading out.
const LINGER: Duration = Duration::from_millis(1500);

type Rgb = (f64, f64, f64);
const BG: Rgb = (0.11, 0.11, 0.12);
const PAD: Rgb = (0.15, 0.15, 0.16);
const FG: Rgb = (0.83, 0.82, 0.82);
const GRID: Rgb = (0.25, 0.25, 0.27);
const TRAIL: Rgb = (0.37, 0.66, 0.96);
const PINCH: Rgb = (0.96, 0.60, 0.26);
const LIT: Rgb = (0.96, 0.77, 0.26);
const PAUSED: Rgb = (0.90, 0.28, 0.30);

thread_local! {
    static WINDOW: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

/// Show the window (or raise it). Must run on the GTK thread.
pub fn open(shared: &Shared) {
    if let Some(window) = WINDOW.with(|w| w.borrow().clone()) {
        window.present();
        return;
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(APP);
    window.set_default_size(780, 400);
    let area = gtk::DrawingArea::new();
    window.add(&area);

    let draw_shared = shared.clone();
    area.connect_draw(move |widget, cr| {
        let (w, h) = (
            f64::from(widget.allocated_width()),
            f64::from(widget.allocated_height()),
        );
        let status = draw_shared.lock().unwrap();
        if let Err(err) = draw(cr, w, h, &status) {
            eprintln!("{APP}: draw: {err}");
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

fn set(cr: &Context, (r, g, b): Rgb, alpha: f64) {
    cr.set_source_rgba(r, g, b, alpha);
}

fn rounded(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, TAU / 2.0);
    cr.arc(x + r, y + r, r, TAU / 2.0, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// `fingers` small dots in a row centered on `(x, y)`.
fn finger_dots(cr: &Context, x: f64, y: f64, fingers: u32) -> Result<(), cairo::Error> {
    let gap = 14.0;
    let start = x - gap * f64::from(fingers.saturating_sub(1)) / 2.0;
    for i in 0..fingers {
        cr.arc(start + gap * f64::from(i), y, 5.0, 0.0, TAU);
        cr.fill()?;
    }
    Ok(())
}

/// Draw `text`, cut with an ellipsis to fit `max` pixels.
fn text_fit(cr: &Context, x: f64, y: f64, text: &str, max: f64) -> Result<(), cairo::Error> {
    let mut shown = text.to_string();
    if cr.text_extents(&shown)?.x_advance() > max {
        while !shown.is_empty() && cr.text_extents(&format!("{shown}…"))?.x_advance() > max {
            shown.pop();
        }
        shown.push('…');
    }
    cr.move_to(x, y);
    cr.show_text(&shown)
}

fn ago(at: Instant) -> String {
    let secs = at.elapsed().as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

fn draw(cr: &Context, w: f64, h: f64, status: &Status) -> Result<(), cairo::Error> {
    set(cr, BG, 1.0);
    cr.paint()?;
    ty_common::appearance::select_font(cr, FontWeight::Normal);

    // Header.
    cr.set_font_size(13.0);
    set(cr, FG, 1.0);
    cr.move_to(16.0, 24.0);
    let touchpad = status.touchpad.as_deref().unwrap_or("no touchpad found");
    cr.show_text(&format!("{APP} · {touchpad}"))?;
    if status.paused {
        set(cr, PAUSED, 1.0);
        cr.show_text("  · paused")?;
    }

    // The touchpad.
    let size = (h - 56.0).min(w * 0.5).max(120.0);
    let (px, py) = (16.0, 40.0);
    let (cx, cy) = (px + size / 2.0, py + size / 2.0);
    set(cr, PAD, 1.0);
    rounded(cr, px, py, size, size, 14.0);
    cr.fill()?;
    cr.set_line_width(if status.is_lit() { 3.0 } else { 1.0 });
    if status.is_lit() {
        set(cr, LIT, 1.0);
    } else {
        set(cr, GRID, 1.0);
    }
    rounded(cr, px, py, size, size, 14.0);
    cr.stroke()?;

    let fade = match status.ended {
        Some((at, _)) => 1.0 - at.elapsed().as_secs_f64() / LINGER.as_secs_f64(),
        None => 1.0,
    };
    let caption = match (&status.live, fade > 0.0) {
        (Some(live), true) => draw_live(cr, status, live, cx, cy, size, fade)?,
        _ => {
            draw_idle(cr, status, cx, cy, size)?;
            "make a gesture".to_string()
        }
    };
    cr.set_font_size(12.0);
    set(cr, FG, 0.85);
    let width = cr.text_extents(&caption)?.x_advance();
    cr.move_to(cx - width / 2.0, py + size - 14.0);
    cr.show_text(&caption)?;

    // Right column: recent firings, then bindings.
    let x = px + size + 28.0;
    let max = w - x - 16.0;
    let mut y = 58.0;
    cr.set_font_size(12.0);
    set(cr, FG, 0.55);
    cr.move_to(x, y);
    cr.show_text(&format!("RECENT ({} fired)", status.fired))?;
    y += 20.0;
    if status.history.is_empty() {
        set(cr, FG, 0.45);
        cr.move_to(x, y);
        cr.show_text("nothing yet")?;
        y += 18.0;
    }
    for fired in status.history.iter().rev().take(6) {
        let fresh = fired.at.elapsed() < Duration::from_millis(1500);
        set(
            cr,
            if fresh { LIT } else { FG },
            if fresh { 1.0 } else { 0.85 },
        );
        cr.move_to(x, y);
        cr.show_text(&fired.gesture)?;
        set(cr, FG, 0.45);
        let when = ago(fired.at);
        let when_w = cr.text_extents(&when)?.x_advance();
        cr.move_to(x + max - when_w, y);
        cr.show_text(&when)?;
        text_fit(
            cr,
            x + 110.0,
            y,
            &fired.command,
            max - 110.0 - when_w - 10.0,
        )?;
        y += 18.0;
    }

    y += 16.0;
    set(cr, FG, 0.55);
    cr.move_to(x, y);
    cr.show_text("BINDINGS")?;
    y += 20.0;
    let latest = status
        .history
        .back()
        .filter(|f| f.at.elapsed() < Duration::from_millis(1500))
        .map(|f| f.gesture.as_str());
    for (gesture, command) in &status.bindings {
        if y > h - 12.0 {
            break;
        }
        let hit = latest == Some(gesture.as_str());
        set(cr, if hit { LIT } else { FG }, if hit { 1.0 } else { 0.85 });
        cr.move_to(x, y);
        cr.show_text(gesture)?;
        set(cr, FG, if hit { 0.9 } else { 0.45 });
        text_fit(cr, x + 110.0, y, command, max - 110.0)?;
        y += 18.0;
    }
    Ok(())
}

/// Pixels per libinput unit, so the swipe threshold ring sits at a third of the pad.
fn swipe_scale(status: &Status, size: f64) -> f64 {
    size * 0.33 / status.thresholds.swipe.max(1.0)
}

fn draw_idle(
    cr: &Context,
    status: &Status,
    cx: f64,
    cy: f64,
    size: f64,
) -> Result<(), cairo::Error> {
    let r = status.thresholds.swipe * swipe_scale(status, size);
    set(cr, GRID, 1.0);
    cr.set_line_width(1.0);
    cr.set_dash(&[4.0, 4.0], 0.0);
    cr.arc(cx, cy, r, 0.0, TAU);
    cr.stroke()?;
    cr.set_dash(&[], 0.0);
    Ok(())
}

fn draw_live(
    cr: &Context,
    status: &Status,
    live: &Live,
    cx: f64,
    cy: f64,
    size: f64,
    fade: f64,
) -> Result<String, cairo::Error> {
    let cancelled = status.ended.is_some_and(|(_, c)| c);
    let suffix = if cancelled { " · cancelled" } else { "" };
    let t = &status.thresholds;
    Ok(match live {
        Live::Swipe { fingers, trail } => {
            let k = swipe_scale(status, size);
            draw_idle(cr, status, cx, cy, size)?;
            // Keep the trail on the pad.
            let limit = size / 2.0 - 12.0;
            let point = |(x, y): (f64, f64)| {
                (
                    cx + (x * k).clamp(-limit, limit),
                    cy + (y * k).clamp(-limit, limit),
                )
            };
            set(cr, TRAIL, 0.9 * fade);
            cr.set_line_width(4.0);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            for (i, &p) in trail.iter().enumerate() {
                let (x, y) = point(p);
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.stroke()?;
            let (dx, dy) = trail.last().copied().unwrap_or_default();
            let (ex, ey) = point((dx, dy));
            set(cr, TRAIL, fade);
            finger_dots(cr, ex, ey, *fingers)?;
            let travel = dx.abs().max(dy.abs());
            format!("swipe {fingers} · {travel:.0} / {:.0}{suffix}", t.swipe)
        }
        Live::Pinch {
            fingers,
            scale,
            angle,
        } => {
            let r0 = size * 0.22;
            cr.set_line_width(1.0);
            set(cr, GRID, 1.0);
            cr.set_dash(&[4.0, 4.0], 0.0);
            for ring in [1.0 - t.pinch, 1.0 + t.pinch] {
                cr.arc(cx, cy, r0 * ring, 0.0, TAU);
                cr.stroke()?;
            }
            cr.set_dash(&[], 0.0);
            set(cr, GRID, 1.0);
            cr.arc(cx, cy, r0, 0.0, TAU);
            cr.stroke()?;

            let r = (r0 * scale).clamp(6.0, size / 2.0 - 10.0);
            set(cr, PINCH, 0.9 * fade);
            cr.set_line_width(3.0);
            cr.arc(cx, cy, r, 0.0, TAU);
            cr.stroke()?;
            // Rotation hand, from 12 o'clock.
            let a = angle.to_radians() - FRAC_PI_2;
            cr.move_to(cx, cy);
            cr.line_to(cx + r * a.cos(), cy + r * a.sin());
            cr.stroke()?;
            finger_dots(cr, cx, cy, *fingers)?;
            format!("pinch {fingers} · ×{scale:.2} · {angle:.0}°{suffix}")
        }
        Live::Hold { fingers, since } => {
            let held = status.ended.map_or(since.elapsed(), |(at, _)| at - *since);
            let grow = (held.as_secs_f64() / 0.6).min(1.0);
            set(cr, TRAIL, 0.25 * fade);
            cr.arc(cx, cy, 20.0 + 40.0 * grow, 0.0, TAU);
            cr.fill()?;
            set(cr, TRAIL, fade);
            finger_dots(cr, cx, cy, *fingers)?;
            format!("hold {fingers} · {} ms{suffix}", held.as_millis())
        }
    })
}
