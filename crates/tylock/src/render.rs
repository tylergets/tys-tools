//! Drawing the lock screen: the wallpaper (scaled to cover, optionally blurred, dimmed), a
//! clock and date, and below them either the fingerprint button or the password field.
//!
//! Everything is sized from the monitor's shorter side, with 1080 physical pixels as the
//! reference, so a 4K monitor gets the same layout as a 1080p one at twice the pixels.

use std::f64::consts::TAU;

use anyhow::{Context as _, Result};
use gtk::cairo::{self, Context, Format, ImageSurface};
use gtk::gdk::prelude::GdkContextExt;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::pango::{self, FontDescription, Weight};
use ty_common::screenlock::{Canvas, Damage, Output};
use ty_common::svgpath;

use crate::config::{AppConfig, Palette, Rgba};

const FINGERPRINT_SVG: &str = include_str!("../icons/fingerprint.svg");
/// Backgrounds kept, one per monitor size.
const MAX_BACKGROUNDS: usize = 8;

/// What to show; built by the app for every frame.
pub struct View<'a> {
    pub time: &'a str,
    pub date: &'a str,
    pub caps_lock: bool,
    pub auth: Auth<'a>,
    /// The block `auth` replaces, fading out while `auth` fades in.
    pub previous: Option<Auth<'a>>,
    /// How far the change from `previous` has gone, 0 to 1.
    pub transition: f64,
    /// Sideways offset of `auth`, in layout units (a wrong password or finger shakes).
    pub shake: f64,
}

pub enum Auth<'a> {
    /// The fingerprint button, with the last scan's error under it.
    Fingerprint {
        error: Option<&'a str>,
        /// A finger matched: how far the unlock animation has gone, 0 to 1.
        matched: Option<f64>,
    },
    /// The password field, one dot per typed character.
    Password {
        /// How far each dot has popped in, 0 to 1.
        dots: Vec<f64>,
        /// Being checked: the phase of the dots' pulse, 0 to 1.
        checking: Option<f64>,
        error: Option<&'a str>,
        accepted: bool,
    },
}

pub struct Renderer {
    quote: String,
    palette: Palette,
    dim: f64,
    blur: f64,
    wallpaper: Option<Pixbuf>,
    ui_font: String,
    clock_font: String,
    backgrounds: Vec<((i32, i32), ImageSurface)>,
    fingerprint: Vec<String>,
    /// Per output: the size and clock last drawn, to tell when only the auth block
    /// changed.
    shown: Vec<(u32, (i32, i32), String, String)>,
}

impl Renderer {
    pub fn new(config: &AppConfig) -> Result<Self> {
        let wallpaper = config.wallpaper_path().and_then(|path| {
            Pixbuf::from_file(&path)
                .map_err(|err| eprintln!("tylock: can't load wallpaper {}: {err}", path.display()))
                .ok()
                // Rotated phone photos and the like.
                .map(|pixbuf| pixbuf.apply_embedded_orientation().unwrap_or(pixbuf))
        });
        let ui_font = ty_common::appearance::font_family();
        Ok(Self {
            quote: String::new(),
            palette: config.colors.palette()?,
            dim: config.dim,
            blur: config.blur,
            wallpaper,
            clock_font: config.clock_font.clone().unwrap_or_else(|| ui_font.clone()),
            ui_font,
            backgrounds: Vec::new(),
            fingerprint: svgpath::path_data(FINGERPRINT_SVG),
            shown: Vec::new(),
        })
    }

    pub fn set_quote(&mut self, quote: String) {
        self.quote = quote;
        self.shown.clear();
    }

    /// Draw `view`; returns the band that changed when the rest is as last drawn.
    pub fn draw(&mut self, output: &Output, canvas: Canvas<'_>, view: &View) -> Option<Damage> {
        let (w, h) = (canvas.width as i32, canvas.height as i32);
        if let Err(err) = self.try_draw(output, canvas, view) {
            eprintln!("tylock: drawing failed: {err:#}");
            return None;
        }
        let shown = (
            output.id,
            (w, h),
            view.time.to_string(),
            view.date.to_string(),
        );
        let same = self.shown.contains(&shown);
        self.shown.retain(|s| s.0 != output.id);
        self.shown.push(shown);
        same.then(|| {
            let (w, h) = (f64::from(w), f64::from(h));
            let (top, height) = auth_band(w, h);
            let half = AUTH_HALF_WIDTH * unit(w, h);
            let left = (w / 2.0 - half).max(0.0);
            Damage {
                x: left as u32,
                y: top.max(0.0) as u32,
                width: (2.0 * half).ceil() as u32 + 1,
                height: height.ceil() as u32,
            }
        })
    }

    fn try_draw(&mut self, _output: &Output, canvas: Canvas<'_>, view: &View) -> Result<()> {
        let (w, h) = (canvas.width as i32, canvas.height as i32);
        let needed = canvas.stride as usize * canvas.height as usize;
        anyhow::ensure!(canvas.data.len() >= needed, "canvas is too small");
        let background = self.background(w, h)?;
        // SAFETY: `data` holds `stride * height` bytes and outlives the surface, which is
        // finished (and lets go of the memory) before this function returns.
        let surface = unsafe {
            ImageSurface::create_for_data_unsafe(
                canvas.data.as_mut_ptr(),
                Format::Rgb24,
                w,
                h,
                canvas.stride as i32,
            )?
        };
        let result = (|| -> Result<()> {
            let cr = Context::new(&surface)?;
            cr.set_operator(cairo::Operator::Source);
            cr.set_source_surface(&background, 0.0, 0.0)?;
            cr.paint()?;
            cr.set_operator(cairo::Operator::Over);
            self.paint_ui(&cr, f64::from(w), f64::from(h), view)
        })();
        surface.finish();
        result
    }

    /// The dimmed wallpaper at this size, made once per size.
    fn background(&mut self, w: i32, h: i32) -> Result<ImageSurface> {
        if let Some((_, surface)) = self.backgrounds.iter().find(|(size, _)| *size == (w, h)) {
            return Ok(surface.clone());
        }
        let surface = self.build_background(w, h)?;
        if self.backgrounds.len() >= MAX_BACKGROUNDS {
            self.backgrounds.remove(0);
        }
        self.backgrounds.push(((w, h), surface.clone()));
        Ok(surface)
    }

    fn build_background(&self, w: i32, h: i32) -> Result<ImageSurface> {
        let surface = ImageSurface::create(Format::Rgb24, w, h)?;
        {
            let cr = Context::new(&surface)?;
            set_color(&cr, self.palette.background, 1.0);
            cr.paint()?;
            if let Some(pixbuf) = &self.wallpaper {
                let radius = self.blur * unit(f64::from(w), f64::from(h));
                if radius >= 1.0 {
                    // Blur at up to a quarter of the size: far less work, and the smooth
                    // upscale blurs a little more on top.
                    let factor = radius.clamp(1.0, 4.0);
                    let (sw, sh) = (
                        (f64::from(w) / factor).ceil() as i32,
                        (f64::from(h) / factor).ceil() as i32,
                    );
                    let mut small = ImageSurface::create(Format::Rgb24, sw, sh)?;
                    {
                        let small_cr = Context::new(&small)?;
                        paint_cover(&small_cr, pixbuf, sw, sh)?;
                    }
                    small.flush();
                    box_blur(&mut small, (radius / factor).round() as usize)?;
                    cr.save()?;
                    cr.scale(factor, factor);
                    cr.set_source_surface(&small, 0.0, 0.0)?;
                    cr.source().set_filter(cairo::Filter::Good);
                    cr.source().set_extend(cairo::Extend::Pad);
                    cr.paint()?;
                    cr.restore()?;
                } else {
                    paint_cover(&cr, pixbuf, w, h)?;
                }
            }
            cr.set_source_rgba(0.0, 0.0, 0.0, self.dim);
            cr.paint()?;
            // A soft vignette keeps the edges quiet and the middle readable.
            let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
            let r = f64::from(w).hypot(f64::from(h)) / 2.0;
            let vignette = cairo::RadialGradient::new(cx, cy, r * 0.3, cx, cy, r);
            vignette.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
            vignette.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.35);
            cr.set_source(&vignette)?;
            cr.paint()?;
        }
        surface.flush();
        Ok(surface)
    }

    fn paint_ui(&self, cr: &Context, w: f64, h: f64, view: &View) -> Result<()> {
        let u = unit(w, h);
        let cx = w / 2.0;
        let p = &self.palette;
        let max_width = (w - 80.0 * u).max(100.0);
        if !self.quote.is_empty() {
            let layout = pangocairo::functions::create_layout(cr);
            layout.set_font_description(Some(&font(&self.ui_font, Weight::Normal, 22.0 * u)));
            layout.set_text(&self.quote);
            let width = max_width.min(1100.0 * u);
            layout.set_width((width * f64::from(pango::SCALE)) as i32);
            layout.set_wrap(pango::WrapMode::WordChar);
            layout.set_height(-4);
            layout.set_ellipsize(pango::EllipsizeMode::End);
            layout.set_alignment(pango::Alignment::Center);
            let (_, height) = layout.pixel_size();
            cr.move_to(cx - width / 2.0, h - 55.0 * u - f64::from(height));
            set_color(cr, p.text, 0.9);
            pangocairo::functions::show_layout(cr, &layout);
        }
        // The auth block stays within the damage rectangle (see `draw`).
        let auth_width = max_width.min(2.0 * (AUTH_HALF_WIDTH - 30.0) * u);

        let time_y = h / 2.0 - 165.0 * u;
        let time_font = font(&self.clock_font, Weight::Medium, 150.0 * u);
        let time = time_markup(view.time, 150.0 * u * 0.36);
        text(
            cr,
            &time_font,
            &time,
            (cx, time_y),
            p.text,
            1.0,
            max_width,
            true,
            u,
        )?;
        let date_font = font(&self.ui_font, Weight::Medium, 28.0 * u);
        let date_y = time_y + 112.0 * u;
        text(
            cr,
            &date_font,
            view.date,
            (cx, date_y),
            p.text,
            0.9,
            max_width,
            false,
            u,
        )?;

        // The auth block: the old one drifts up and fades, the new one rises into place.
        let auth_y = h / 2.0 + 120.0 * u;
        let band = auth_band(w, h);
        let t = if view.previous.is_some() {
            ease_out(view.transition.clamp(0.0, 1.0))
        } else {
            1.0
        };
        if let Some(previous) = view.previous.as_ref().filter(|_| t < 1.0) {
            let at = (cx, auth_y - 10.0 * u * t);
            self.paint_block(cr, previous, at, u, auth_width, 1.0 - t, (w, band))?;
        }
        let at = (cx + view.shake * u, auth_y + 12.0 * u * (1.0 - t));
        let bottom = self.paint_block(cr, &view.auth, at, u, auth_width, t, (w, band))?;
        if view.caps_lock {
            let caps_font = font(&self.ui_font, Weight::Semibold, 16.0 * u);
            let y = bottom + 36.0 * u;
            text(
                cr,
                &caps_font,
                "Caps Lock is on",
                (cx, y),
                p.warning,
                1.0,
                max_width,
                false,
                u,
            )?;
        }
        Ok(())
    }

    /// One auth block at `alpha` (faded as a whole, within the auth band); returns the
    /// y of its last line.
    #[allow(clippy::too_many_arguments)]
    fn paint_block(
        &self,
        cr: &Context,
        auth: &Auth,
        at: (f64, f64),
        u: f64,
        max_width: f64,
        alpha: f64,
        (w, (band_top, band_height)): (f64, (f64, f64)),
    ) -> Result<f64> {
        let faded = alpha < 0.999;
        if faded {
            cr.save()?;
            cr.rectangle(0.0, band_top, w, band_height);
            cr.clip();
            cr.push_group();
        }
        let bottom = match auth {
            Auth::Fingerprint { error, matched } => {
                self.paint_fingerprint(cr, at, u, max_width, *error, *matched)
            }
            Auth::Password {
                dots,
                checking,
                error,
                accepted,
            } => self.paint_password(cr, at, u, max_width, dots, *checking, *error, *accepted),
        };
        if faded {
            cr.pop_group_to_source()?;
            cr.paint_with_alpha(alpha.max(0.0))?;
            cr.restore()?;
        }
        bottom
    }

    /// The round fingerprint button and its text; returns the y of the last line.
    #[allow(clippy::too_many_arguments)]
    fn paint_fingerprint(
        &self,
        cr: &Context,
        (cx, cy): (f64, f64),
        u: f64,
        max_width: f64,
        error: Option<&str>,
        matched: Option<f64>,
    ) -> Result<f64> {
        let p = &self.palette;
        let color = if matched.is_some() {
            p.success
        } else if error.is_some() {
            p.error
        } else {
            p.accent
        };
        let r = 44.0 * u;
        // A match ripples out from the button, which swells for a moment.
        let swell = matched.map_or(1.0, |m| {
            1.0 + 0.1 * (std::f64::consts::PI * m.min(1.0)).sin()
        });
        if let Some(m) = matched {
            for (delay, strength) in [(0.0, 0.8), (0.25, 0.5)] {
                let m = ((m - delay) / (1.0 - delay)).clamp(0.0, 1.0);
                if m > 0.0 && m < 1.0 {
                    cr.new_path();
                    cr.arc(cx, cy, r * (1.0 + 0.8 * ease_out(m)), 0.0, TAU);
                    set_color(cr, p.success, strength * (1.0 - m));
                    cr.set_line_width(2.0 * u * (1.0 - m) + 0.5 * u);
                    cr.stroke()?;
                }
            }
        }
        cr.new_path();
        cr.arc(cx, cy, r * swell, 0.0, TAU);
        set_color(cr, p.background, 0.55);
        cr.fill_preserve()?;
        set_color(cr, color, 0.9);
        cr.set_line_width(1.5 * u);
        cr.stroke()?;

        let size = 50.0 * u * swell;
        cr.save()?;
        cr.translate(cx - size / 2.0, cy - size / 2.0);
        cr.scale(size / 24.0, size / 24.0);
        cr.new_path();
        for path in &self.fingerprint {
            svgpath::trace(cr, path)?;
        }
        set_color(cr, color, 1.0);
        cr.set_line_width(1.6);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.stroke()?;
        cr.restore()?;

        let (line, line_color) = match (matched, error) {
            (Some(_), _) => ("Unlocking…", p.success),
            (None, Some(error)) => (error, p.error),
            (None, None) => ("Touch the fingerprint reader", p.text),
        };
        let main_font = font(&self.ui_font, Weight::Medium, 19.0 * u);
        let y1 = cy + r + 34.0 * u;
        text(
            cr,
            &main_font,
            line,
            (cx, y1),
            line_color,
            1.0,
            max_width,
            false,
            u,
        )?;
        let hint_font = font(&self.ui_font, Weight::Normal, 16.0 * u);
        let y2 = y1 + 29.0 * u;
        let hint = "or type your password";
        text(
            cr,
            &hint_font,
            hint,
            (cx, y2),
            p.muted,
            0.9,
            max_width,
            false,
            u,
        )?;
        Ok(y2)
    }

    /// The password field and the line under it; returns the y of that line (or of where
    /// a line above it would be, when it is empty).
    #[allow(clippy::too_many_arguments)]
    fn paint_password(
        &self,
        cr: &Context,
        (cx, cy): (f64, f64),
        u: f64,
        max_width: f64,
        dots: &[f64],
        checking: Option<f64>,
        error: Option<&str>,
        accepted: bool,
    ) -> Result<f64> {
        let p = &self.palette;
        let gap = 22.0 * u;
        let padding = 52.0 * u;
        // Grows with the password, up to most of the screen.
        let widest = max_width.min(1000.0 * u);
        let fw = (dots.len() as f64 * gap + padding).clamp((360.0 * u).min(widest), widest);
        let fh = 56.0 * u;
        let border = if accepted {
            p.success
        } else if error.is_some() {
            p.error
        } else if checking.is_some() {
            p.muted
        } else {
            p.accent
        };
        rounded_rect(cr, cx - fw / 2.0, cy - fh / 2.0, fw, fh, fh / 2.0);
        set_color(cr, p.background, 0.6);
        cr.fill_preserve()?;
        set_color(cr, border, 0.9);
        cr.set_line_width(1.5 * u);
        cr.stroke()?;

        if !dots.is_empty() {
            let radius = 6.5 * u;
            let fit = ((fw - padding) / gap).floor().max(1.0) as usize;
            // The newest dots, when there are more than fit.
            let shown = &dots[dots.len().saturating_sub(fit)..];
            let n = shown.len();
            let start = cx - (n as f64 - 1.0) * gap / 2.0;
            for (i, &pop) in shown.iter().enumerate() {
                // Checking sends a wave of light along the dots.
                let alpha = match checking {
                    Some(phase) => {
                        let wave = (TAU * (phase - i as f64 * 0.06)).cos() * 0.5 + 0.5;
                        0.35 + 0.55 * wave
                    }
                    None => 1.0,
                };
                set_color(cr, if accepted { p.success } else { p.text }, alpha);
                cr.new_path();
                let size = radius * ease_out_back(pop.clamp(0.0, 1.0)).max(0.0);
                cr.arc(start + i as f64 * gap, cy, size, 0.0, TAU);
                cr.fill()?;
            }
        } else if checking.is_none() {
            let placeholder = font(&self.ui_font, Weight::Normal, 18.0 * u);
            let text_width = fw - 40.0 * u;
            text(
                cr,
                &placeholder,
                "Enter password",
                (cx, cy),
                p.muted,
                0.85,
                text_width,
                false,
                u,
            )?;
        }

        let y = cy + fh / 2.0 + 32.0 * u;
        let status_font = font(&self.ui_font, Weight::Medium, 17.0 * u);
        if checking.is_some() {
            text(
                cr,
                &status_font,
                "Checking…",
                (cx, y),
                p.muted,
                1.0,
                max_width,
                false,
                u,
            )?;
        } else if let Some(error) = error {
            text(
                cr,
                &status_font,
                error,
                (cx, y),
                p.error,
                1.0,
                max_width,
                false,
                u,
            )?;
        } else {
            // Nothing on that line: what comes next (Caps Lock) can take it.
            return Ok(y - 36.0 * u);
        }
        Ok(y)
    }
}

/// Half the width the auth block may use, in layout units: the widest password field
/// (1000) plus room for the shake.
const AUTH_HALF_WIDTH: f64 = 530.0;

/// The rows the auth block (and the Caps Lock line) can touch, as (top, height): from
/// the top of a match's ripple to below the Caps Lock line.
fn auth_band(w: f64, h: f64) -> (f64, f64) {
    let u = unit(w, h);
    let auth_y = h / 2.0 + 120.0 * u;
    (auth_y - 100.0 * u, 320.0 * u)
}

/// Fast start, gentle stop.
pub fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// Like [`ease_out`] but overshooting a little before settling: a pop.
fn ease_out_back(t: f64) -> f64 {
    let c = 1.7;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}

/// Marks a string given to [`text`] as pango markup.
const MARKUP: &str = "\u{1}markup:";

/// The clock as markup: a trailing AM/PM (or am/pm) drawn at `suffix_px`, so 12-hour
/// times keep the digits as the headline.
fn time_markup(time: &str, suffix_px: f64) -> String {
    let escape = |s: &str| gtk::glib::markup_escape_text(s).to_string();
    match time.rsplit_once(' ') {
        Some((digits, suffix)) if !suffix.is_empty() && suffix.chars().all(char::is_alphabetic) => {
            format!(
                "{MARKUP}{}<span size=\"{}\" weight=\"600\"> {}</span>",
                escape(digits),
                (suffix_px * f64::from(pango::SCALE)) as i32,
                escape(suffix)
            )
        }
        _ => format!("{MARKUP}{}", escape(time)),
    }
}

/// The layout unit: 1 on a 1080p monitor, scaled by the shorter side.
fn unit(w: f64, h: f64) -> f64 {
    (w.min(h) / 1080.0).max(0.4)
}

fn font(family: &str, weight: Weight, px: f64) -> FontDescription {
    let mut font = FontDescription::new();
    font.set_family(family);
    font.set_weight(weight);
    font.set_absolute_size(px * f64::from(pango::SCALE));
    font
}

fn set_color(cr: &Context, color: Rgba, alpha: f64) {
    cr.set_source_rgba(color.r, color.g, color.b, color.a * alpha);
}

/// One line of text centered on `center` (by its ink with `ink`, which centers digits
/// visually, else by its line box), ellipsized to `max_width`, over a soft shadow.
#[allow(clippy::too_many_arguments)]
fn text(
    cr: &Context,
    font: &FontDescription,
    content: &str,
    (cx, cy): (f64, f64),
    color: Rgba,
    alpha: f64,
    max_width: f64,
    ink: bool,
    u: f64,
) -> Result<()> {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(font));
    layout.set_width((max_width * f64::from(pango::SCALE)) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(pango::Alignment::Center);
    match content.strip_prefix(MARKUP) {
        Some(markup) => layout.set_markup(markup),
        None => layout.set_text(content),
    }
    let (ink_rect, logical) = layout.pixel_extents();
    let rect = if ink { ink_rect } else { logical };
    let x = cx - max_width / 2.0;
    let y = cy - f64::from(rect.height()) / 2.0 - f64::from(rect.y());

    let spread = 1.5 * u;
    for (dx, dy) in [(0.0, 0.0), (-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.1 * alpha);
        cr.move_to(x + dx * spread, y + spread + dy * spread);
        pangocairo::functions::show_layout(cr, &layout);
    }
    set_color(cr, color, alpha);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
    Ok(())
}

fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_path();
    cr.arc(x + w - r, y + r, r, -TAU / 4.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, TAU / 4.0);
    cr.arc(x + r, y + h - r, r, TAU / 4.0, TAU / 2.0);
    cr.arc(x + r, y + r, r, TAU / 2.0, TAU * 0.75);
    cr.close_path();
}

/// Draw `pixbuf` scaled to cover `w`×`h`, centered (cropping what doesn't fit).
fn paint_cover(cr: &Context, pixbuf: &Pixbuf, w: i32, h: i32) -> Result<()> {
    let (iw, ih) = (f64::from(pixbuf.width()), f64::from(pixbuf.height()));
    let scale = (f64::from(w) / iw).max(f64::from(h) / ih);
    cr.save()?;
    cr.translate(
        (f64::from(w) - iw * scale) / 2.0,
        (f64::from(h) - ih * scale) / 2.0,
    );
    cr.scale(scale, scale);
    cr.set_source_pixbuf(pixbuf, 0.0, 0.0);
    cr.source().set_filter(cairo::Filter::Good);
    cr.source().set_extend(cairo::Extend::Pad);
    cr.paint()?;
    cr.restore()?;
    Ok(())
}

/// Three box-blur passes (close to a Gaussian) of `radius` pixels, in place.
fn box_blur(surface: &mut ImageSurface, radius: usize) -> Result<()> {
    if radius == 0 {
        return Ok(());
    }
    let (w, h) = (surface.width() as usize, surface.height() as usize);
    let stride = surface.stride() as usize;
    let mut data = surface.data().context("can't access the blur surface")?;
    for _ in 0..3 {
        blur_lines(&mut data, h, w, stride, 4, radius);
        blur_lines(&mut data, w, h, 4, stride, radius);
    }
    Ok(())
}

/// Box-blur `lines` lines of `len` pixels each: pixel `i` of line `l` starts at byte
/// `l * line_step + i * pixel_step`. Edges repeat the outermost pixel.
fn blur_lines(
    data: &mut [u8],
    lines: usize,
    len: usize,
    line_step: usize,
    pixel_step: usize,
    radius: usize,
) {
    if len == 0 {
        return;
    }
    let mut copy = vec![[0u8; 3]; len];
    let window = (2 * radius + 1) as u32;
    let last = len as isize - 1;
    for line in 0..lines {
        let base = line * line_step;
        for (i, px) in copy.iter_mut().enumerate() {
            let at = base + i * pixel_step;
            px.copy_from_slice(&data[at..at + 3]);
        }
        let get = |i: isize, c: usize| u32::from(copy[i.clamp(0, last) as usize][c]);
        for c in 0..3 {
            let r = radius as isize;
            let mut sum: u32 = (-r..=r).map(|i| get(i, c)).sum();
            for i in 0..len {
                data[base + i * pixel_step + c] = (sum / window) as u8;
                let i = i as isize;
                sum += get(i + r + 1, c);
                sum -= get(i - r, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn am_pm_is_smaller() {
        let markup = time_markup("9:41 PM", 10.0);
        assert!(markup.starts_with(MARKUP));
        assert!(markup.contains("9:41<span"));
        assert!(markup.ends_with(" PM</span>"));
        assert_eq!(time_markup("21:41", 10.0), format!("{MARKUP}21:41"));
    }

    #[test]
    fn easing_ends_in_place() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out_back(0.7) > 1.0);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unit_follows_the_shorter_side() {
        assert_eq!(unit(1920.0, 1080.0), 1.0);
        assert_eq!(unit(3840.0, 2160.0), 2.0);
        assert_eq!(unit(1080.0, 1920.0), 1.0);
    }

    #[test]
    fn blur_spreads_and_keeps_flat_areas() {
        // One row, 9 pixels: a single bright pixel in the middle.
        let mut data = vec![0u8; 9 * 4];
        data[4 * 4..4 * 4 + 3].copy_from_slice(&[90, 90, 90]);
        blur_lines(&mut data, 1, 9, 0, 4, 1);
        assert_eq!(&data[3 * 4..3 * 4 + 3], &[30, 30, 30]);
        assert_eq!(&data[4 * 4..4 * 4 + 3], &[30, 30, 30]);
        assert_eq!(&data[0..3], &[0, 0, 0]);

        let mut flat = vec![200u8; 5 * 4];
        blur_lines(&mut flat, 1, 5, 0, 4, 2);
        assert!(flat.chunks(4).all(|px| px[..3] == [200, 200, 200]));
    }

    #[test]
    fn draws_both_views() {
        let mut renderer = Renderer::new(&AppConfig::default()).unwrap();
        renderer.wallpaper = None;
        let (w, h) = (320u32, 180u32);
        let mut data = vec![0u8; (w * h * 4) as usize];
        let output = Output {
            id: 1,
            width: w,
            height: h,
        };
        for auth in [
            Auth::Password {
                dots: vec![1.0, 1.0, 0.5],
                checking: Some(0.3),
                error: Some("Wrong password"),
                accepted: false,
            },
            Auth::Fingerprint {
                error: None,
                matched: Some(0.4),
            },
        ] {
            let view = View {
                time: "12:34 PM",
                date: "Wednesday, November 11",
                auth,
                previous: Some(Auth::Fingerprint {
                    error: None,
                    matched: None,
                }),
                transition: 0.5,
                shake: 3.0,
                caps_lock: true,
            };
            let canvas = Canvas {
                data: &mut data,
                width: w,
                height: h,
                stride: w * 4,
            };
            renderer.try_draw(&output, canvas, &view).unwrap();
            renderer.shown.clear();
            let canvas = Canvas {
                data: &mut data,
                width: w,
                height: h,
                stride: w * 4,
            };
            assert!(renderer.draw(&output, canvas, &view).is_none());
            let canvas = Canvas {
                data: &mut data,
                width: w,
                height: h,
                stride: w * 4,
            };
            // Same clock as just drawn: only the auth band is damaged.
            renderer.draw(&output, canvas, &view).unwrap();
        }
        // Something other than the flat background was drawn.
        let first = &data[0..3];
        assert!(data.chunks(4).any(|px| &px[..3] != first));
    }
}
