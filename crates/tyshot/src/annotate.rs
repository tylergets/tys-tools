//! Annotations drawn on top of the screenshot (boxes, arrows, pen and highlighter strokes,
//! text, numbered step markers, blurred areas), and rendering
//! the final image. The overlay preview and the exported image share [`paint_scene`], so what
//! is shown is what gets copied or saved.
//!
//! Annotation coordinates are logical (overlay) pixels, absolute on the screen.

use std::cell::RefCell;

use anyhow::{Context, Result};
use gtk::cairo;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::glib;
use gtk::prelude::*;

use crate::selection::{PixelRect, Rect};
use ty_common::frame::Frame;

#[derive(Debug, Clone, PartialEq)]
pub enum Annotation {
    Box(Rect),
    Arrow {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// A freehand stroke through these points.
    Pen(Vec<(f64, f64)>),
    /// A wide translucent freehand stroke, multiplied into the image like a marker.
    Highlight(Vec<(f64, f64)>),
    /// Text (possibly several lines) whose first line's top-left corner is `at`.
    Text {
        at: (f64, f64),
        text: String,
    },
    /// A numbered circle centered on `at`.
    Step {
        at: (f64, f64),
        number: u32,
    },
    Blur(Rect),
}

impl Annotation {
    /// Too small to be intentional (a click rather than a drag), or empty.
    pub fn is_degenerate(&self) -> bool {
        const MIN: f64 = 3.0;
        match self {
            Self::Box(rect) | Self::Blur(rect) => rect.width() < MIN || rect.height() < MIN,
            Self::Arrow { from, to } => (to.0 - from.0).hypot(to.1 - from.1) < MIN * 2.0,
            // A click with the pen leaves a dot, which is fine; only an empty stroke isn't.
            Self::Pen(points) | Self::Highlight(points) => points.is_empty(),
            Self::Text { text, .. } => text.trim().is_empty(),
            Self::Step { .. } => false,
        }
    }
}

/// How annotations are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// Boxes, arrows, pen, text and step markers.
    pub color: (f64, f64, f64),
    /// Box, arrow and pen thickness, logical pixels.
    pub line_width: f64,
    /// Highlighter color.
    pub highlight: (f64, f64, f64),
    /// Text size, logical pixels. The highlighter is this wide and step markers scale with it.
    pub font_size: f64,
}

/// Opacity of the highlighter stroke.
const HIGHLIGHT_ALPHA: f64 = 0.55;
/// Line height as a multiple of the font size.
const LINE_SPACING: f64 = 1.25;

/// The screen snapshot, cached as a cairo surface for fast repaints, plus a blurred copy of
/// it (made on first use) that blur annotations reveal.
pub struct Canvas {
    pub snapshot: Pixbuf,
    surface: cairo::ImageSurface,
    blurred: Option<cairo::ImageSurface>,
    /// Copies of `surface` and `blurred` kept in the X server, made the first time the
    /// overlay paints them. Painting an image onto an X11 window uploads all of it, every
    /// frame: at 4K that is a 33 MB upload per keystroke, which a GPU-backed X server
    /// can't keep up with, so typing and drawing lag behind. From a server-side copy each
    /// repaint is a copy within the server.
    on_server: RefCell<ServerCopies>,
    /// Device pixels per logical pixel.
    scale: f64,
    /// In device pixels.
    blur_radius: usize,
}

impl Canvas {
    pub fn new(snapshot: Pixbuf, scale: f64, blur_radius: usize) -> Result<Self> {
        let surface =
            cairo::ImageSurface::create(cairo::Format::Rgb24, snapshot.width(), snapshot.height())
                .context("failed to create an image surface")?;
        {
            let cr = cairo::Context::new(&surface).context("failed to create a cairo context")?;
            cr.set_source_pixbuf(&snapshot, 0.0, 0.0);
            cr.paint().context("failed to paint the snapshot")?;
        }
        surface.flush();
        // Device scale makes one surface unit one logical pixel.
        surface.set_device_scale(scale, scale);
        Ok(Self {
            snapshot,
            surface,
            blurred: None,
            on_server: RefCell::default(),
            scale,
            blur_radius,
        })
    }

    /// Make the blurred copy now (it takes a moment on large screens), e.g. when the blur
    /// tool is picked, so the first blur drag is smooth.
    pub fn prepare_blur(&mut self) {
        if self.blurred.is_some() {
            return;
        }
        let (width, height, stride) = (
            self.surface.width(),
            self.surface.height(),
            self.surface.stride(),
        );
        let mut blurred = Vec::new();
        let read = self.surface.with_data(|data| {
            // Rgb24 is 4 bytes per pixel; processing the unused byte too is harmless.
            blurred = crate::blur::obscure(
                data,
                width as usize,
                height as usize,
                stride as usize,
                4,
                self.blur_radius,
            );
        });
        if read.is_err() {
            eprintln!("warning: could not read the snapshot to blur it");
            return;
        }
        match cairo::ImageSurface::create_for_data(
            blurred,
            cairo::Format::Rgb24,
            width,
            height,
            stride,
        ) {
            Ok(surface) => {
                surface.set_device_scale(self.scale, self.scale);
                self.blurred = Some(surface);
            }
            Err(err) => eprintln!("warning: failed to create the blurred image: {err}"),
        }
    }

    /// Paint the plain snapshot onto a logical-pixel `cr`.
    pub fn paint_snapshot(&self, cr: &cairo::Context) {
        self.paint_cached(cr, &self.surface, |copies| &mut copies.plain);
    }

    /// Paint the blurred snapshot (if prepared) onto a logical-pixel `cr`.
    fn paint_blurred(&self, cr: &cairo::Context) {
        if let Some(blurred) = &self.blurred {
            self.paint_cached(cr, blurred, |copies| &mut copies.blurred);
        }
    }

    /// Paint `image`, from its server-side copy when `cr` draws to an X11 window.
    fn paint_cached(
        &self,
        cr: &cairo::Context,
        image: &cairo::ImageSurface,
        slot: impl FnOnce(&mut ServerCopies) -> &mut Option<cairo::Surface>,
    ) {
        let target = cr.target();
        if target.type_() != cairo::SurfaceType::Xlib {
            paint_surface(cr, image);
            return;
        }
        let mut copies = self.on_server.borrow_mut();
        let slot = slot(&mut copies);
        if slot.is_none() {
            *slot = server_copy(&target, image, self.scale);
        }
        match slot {
            Some(copy) => paint_surface(cr, copy),
            None => paint_surface(cr, image),
        }
    }
}

#[derive(Default)]
struct ServerCopies {
    plain: Option<cairo::Surface>,
    blurred: Option<cairo::Surface>,
}

/// A surface like `target` (in the X server) holding `image`, whose device scale is
/// `scale`; `None` if the server can't make one.
fn server_copy(
    target: &cairo::Surface,
    image: &cairo::ImageSurface,
    scale: f64,
) -> Option<cairo::Surface> {
    // In logical pixels, as `image` is painted: the copy is what each frame showed.
    let width = (f64::from(image.width()) / scale).ceil() as i32;
    let height = (f64::from(image.height()) / scale).ceil() as i32;
    let copy = target
        .create_similar(cairo::Content::Color, width, height)
        .ok()?;
    {
        let cr = cairo::Context::new(&copy).ok()?;
        cr.set_operator(cairo::Operator::Source);
        paint_surface(&cr, image);
    }
    copy.flush();
    Some(copy)
}

/// Paint the snapshot and `annotations` (in order) onto `cr`, whose user space is logical
/// pixels.
pub fn paint_scene(
    cr: &cairo::Context,
    canvas: &mut Canvas,
    annotations: &[Annotation],
    style: &Style,
) {
    canvas.paint_snapshot(cr);
    paint_annotations(cr, canvas, annotations, style);
}

/// Paint `annotations` in order, preparing the blur first if any needs it.
pub fn paint_annotations(
    cr: &cairo::Context,
    canvas: &mut Canvas,
    annotations: &[Annotation],
    style: &Style,
) {
    if annotations
        .iter()
        .any(|annotation| matches!(annotation, Annotation::Blur(_)))
    {
        canvas.prepare_blur();
    }
    for annotation in annotations {
        paint_annotation(cr, canvas, annotation, style);
    }
}

/// Paint one annotation. `canvas` must have its blur prepared for a blur annotation.
pub fn paint_annotation(
    cr: &cairo::Context,
    canvas: &Canvas,
    annotation: &Annotation,
    style: &Style,
) {
    let (r, g, b) = style.color;
    let width = style.line_width;
    cr.save().ok();
    // Text leaves a current point behind, which would join the next path to it.
    cr.new_path();
    match *annotation {
        Annotation::Box(rect) => {
            cr.set_source_rgb(r, g, b);
            cr.set_line_width(width);
            cr.set_line_join(cairo::LineJoin::Miter);
            cr.rectangle(rect.x0, rect.y0, rect.width(), rect.height());
            let _ = cr.stroke();
        }
        Annotation::Arrow { from, to } => {
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx.hypot(dy);
            if length > 0.0 {
                let (ux, uy) = (dx / length, dy / length);
                // Head scales with the line, but never longer than half the arrow.
                let head = (width * 3.5 + 8.0).min(length * 0.5);
                let half = head * 0.55;
                let base = (to.0 - ux * head, to.1 - uy * head);
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(width);
                cr.set_line_cap(cairo::LineCap::Round);
                cr.move_to(from.0, from.1);
                // Stop inside the head so the round cap doesn't poke past the tip.
                cr.line_to(base.0 + ux * head * 0.3, base.1 + uy * head * 0.3);
                let _ = cr.stroke();
                cr.move_to(to.0, to.1);
                cr.line_to(base.0 - uy * half, base.1 + ux * half);
                cr.line_to(base.0 + uy * half, base.1 - ux * half);
                cr.close_path();
                let _ = cr.fill();
            }
        }
        Annotation::Pen(ref points) => {
            cr.set_source_rgb(r, g, b);
            stroke_freehand(cr, points, width);
        }
        Annotation::Highlight(ref points) => {
            let (r, g, b) = style.highlight;
            cr.set_source_rgba(r, g, b, HIGHLIGHT_ALPHA);
            // Darkens like ink: dark text under it stays readable.
            cr.set_operator(cairo::Operator::Multiply);
            stroke_freehand(cr, points, style.font_size);
        }
        Annotation::Text { at, ref text } => {
            cr.set_source_rgb(r, g, b);
            set_text_font(cr, style.font_size);
            let ascent = cr
                .font_extents()
                .map(|e| e.ascent())
                .unwrap_or(style.font_size);
            for (index, line) in text.split('\n').enumerate() {
                cr.move_to(
                    at.0,
                    at.1 + ascent + index as f64 * style.font_size * LINE_SPACING,
                );
                let _ = cr.show_text(line);
            }
        }
        Annotation::Step { at, number } => {
            let radius = step_radius(style.font_size);
            cr.arc(at.0, at.1, radius, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(r, g, b);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width((radius * 0.12).max(1.5));
            let _ = cr.stroke();

            let label = number.to_string();
            set_text_font(cr, radius * if number < 10 { 1.15 } else { 0.95 });
            if let Ok(extents) = cr.text_extents(&label) {
                // Black on light colors, white on dark ones.
                let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let ink = if luminance > 0.6 { 0.0 } else { 1.0 };
                cr.set_source_rgb(ink, ink, ink);
                cr.move_to(
                    at.0 - extents.x_bearing() - extents.width() / 2.0,
                    at.1 - extents.y_bearing() - extents.height() / 2.0,
                );
                let _ = cr.show_text(&label);
            }
        }
        Annotation::Blur(rect) => {
            if canvas.blurred.is_some() {
                cr.rectangle(rect.x0, rect.y0, rect.width(), rect.height());
                cr.clip();
                canvas.paint_blurred(cr);
            }
        }
    }
    cr.new_path();
    cr.restore().ok();
}

/// Radius of a numbered step marker for `font_size`.
pub fn step_radius(font_size: f64) -> f64 {
    font_size * 0.8
}

fn set_text_font(cr: &cairo::Context, size: f64) {
    ty_common::appearance::select_font(cr, cairo::FontWeight::Bold);
    cr.set_font_size(size);
}

/// Where the text cursor goes after `text` placed at `at`: the top and bottom of the caret
/// line, after the last character of the last line.
pub fn text_caret(
    cr: &cairo::Context,
    at: (f64, f64),
    text: &str,
    font_size: f64,
) -> ((f64, f64), (f64, f64)) {
    cr.save().ok();
    set_text_font(cr, font_size);
    let last = text.rsplit('\n').next().unwrap_or("");
    let advance = cr.text_extents(last).map(|e| e.x_advance()).unwrap_or(0.0);
    cr.restore().ok();
    let lines = text.matches('\n').count() as f64;
    let top = at.1 + lines * font_size * LINE_SPACING;
    (
        (at.0 + advance, top),
        (at.0 + advance, top + font_size * 1.15),
    )
}

/// A rectangle (logical pixels) covering `text` placed at `at` and its caret, with some
/// slack: what needs repainting when the text changes.
pub fn text_bounds(at: (f64, f64), text: &str, font_size: f64) -> Rect {
    let widest = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1)
        .ok()
        .and_then(|surface| cairo::Context::new(&surface).ok())
        .map(|cr| {
            set_text_font(&cr, font_size);
            text.split('\n')
                .filter_map(|line| cr.text_extents(line).ok())
                .map(|e| e.x_advance().max(e.x_bearing() + e.width()))
                .fold(0.0, f64::max)
        })
        // Without a context, assume generously wide glyphs.
        .unwrap_or(text.chars().count() as f64 * font_size);
    let lines = text.matches('\n').count() as f64 + 1.0;
    let slack = font_size * 0.5;
    Rect {
        x0: at.0 - slack,
        y0: at.1 - slack,
        x1: at.0 + widest + 2.0 * slack,
        y1: at.1 + lines * font_size * LINE_SPACING + 2.0 * slack,
    }
}

/// Stroke a smooth freehand line (round caps and joins) through `points`; one point is a dot.
fn stroke_freehand(cr: &cairo::Context, points: &[(f64, f64)], width: f64) {
    let Some(&(x, y)) = points.first() else {
        return;
    };
    cr.set_line_width(width);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(x, y);
    if points.len() == 1 {
        // A zero-length segment with a round cap draws a dot.
        cr.line_to(x, y);
    }
    for &(x, y) in &points[1..] {
        cr.line_to(x, y);
    }
    let _ = cr.stroke();
}

fn paint_surface(cr: &cairo::Context, surface: &cairo::Surface) {
    if cr.set_source_surface(surface, 0.0, 0.0).is_ok() {
        let _ = cr.paint();
    }
}

/// Render the `crop` (device pixels) of the annotated snapshot as an image.
pub fn render(
    canvas: &mut Canvas,
    annotations: &[Annotation],
    style: &Style,
    crop: PixelRect,
) -> Result<Pixbuf> {
    if annotations.is_empty() {
        // Nothing drawn: the crop is exact, no need to go through cairo.
        return Ok(canvas.snapshot.new_subpixbuf(
            crop.x as i32,
            crop.y as i32,
            crop.width as i32,
            crop.height as i32,
        ));
    }
    let (width, height) = (crop.width as i32, crop.height as i32);
    let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, width, height)
        .context("failed to create an image surface")?;
    {
        let cr = cairo::Context::new(&surface).context("failed to create a cairo context")?;
        cr.translate(-f64::from(crop.x), -f64::from(crop.y));
        cr.scale(canvas.scale, canvas.scale);
        paint_scene(&cr, canvas, annotations, style);
    }
    surface.flush();
    surface_to_pixbuf(&surface)
}

/// Render the annotated `content` (logical px, on screen) inside `frame`. The image covers
/// the whole frame, which may extend past the screen; outside the frame's rounded corners it
/// is transparent.
pub fn render_framed(
    canvas: &mut Canvas,
    annotations: &[Annotation],
    style: &Style,
    content: &Rect,
    frame: Frame,
) -> Result<Pixbuf> {
    let outer = frame.outer(content);
    let scale = canvas.scale;
    let width = (outer.width() * scale).round().max(1.0) as i32;
    let height = (outer.height() * scale).round().max(1.0) as i32;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .context("failed to create an image surface")?;
    {
        let cr = cairo::Context::new(&surface).context("failed to create a cairo context")?;
        cr.scale(scale, scale);
        cr.translate(-outer.x0, -outer.y0);
        cr.save().ok();
        frame.clip_content(&cr, content);
        paint_scene(&cr, canvas, annotations, style);
        cr.restore().ok();
        frame.paint(&cr, content);
    }
    surface.flush();
    surface_to_pixbuf(&surface)
}

/// Copy an `Rgb24` or `ARgb32` surface into an RGB(A) pixbuf. (gdk's helper for this needs a
/// display.)
fn surface_to_pixbuf(surface: &cairo::ImageSurface) -> Result<Pixbuf> {
    let (width, height, stride) = (
        surface.width() as usize,
        surface.height() as usize,
        surface.stride() as usize,
    );
    let alpha = surface.format() == cairo::Format::ARgb32;
    let channels = if alpha { 4 } else { 3 };
    let mut out = Vec::with_capacity(width * height * channels);
    surface
        .with_data(|data| {
            for row in data.chunks(stride).take(height) {
                // Each pixel is a native-endian 0xAARRGGBB word, premultiplied when `alpha`.
                for pixel in row[..width * 4].chunks_exact(4) {
                    let value = u32::from_ne_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                    let (r, g, b) = ((value >> 16) as u8, (value >> 8) as u8, value as u8);
                    if !alpha {
                        out.extend_from_slice(&[r, g, b]);
                        continue;
                    }
                    let a = (value >> 24) as u8;
                    let unpremultiply = |c: u8| match a {
                        0 => 0,
                        _ => {
                            ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                        }
                    };
                    out.extend_from_slice(&[
                        unpremultiply(r),
                        unpremultiply(g),
                        unpremultiply(b),
                        a,
                    ]);
                }
            }
        })
        .context("failed to read the rendered image")?;
    Ok(Pixbuf::from_bytes(
        &glib::Bytes::from_owned(out),
        Colorspace::Rgb,
        alpha,
        8,
        width as i32,
        height as i32,
        (width * channels) as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vertical black/white stripes, 1 px wide, so blurring turns them gray.
    fn stripes(width: i32, height: i32) -> Pixbuf {
        let mut rgb = Vec::new();
        for _ in 0..height {
            for x in 0..width {
                let value = if x % 2 == 0 { 0 } else { 255 };
                rgb.extend_from_slice(&[value, value, value]);
            }
        }
        Pixbuf::from_bytes(
            &glib::Bytes::from_owned(rgb),
            Colorspace::Rgb,
            false,
            8,
            width,
            height,
            width * 3,
        )
    }

    fn pixel(image: &Pixbuf, x: i32, y: i32) -> [u8; 3] {
        let bytes = image.read_pixel_bytes();
        let at = (y * image.rowstride() + x * image.n_channels()) as usize;
        [bytes[at], bytes[at + 1], bytes[at + 2]]
    }

    #[test]
    fn renders_box_and_blur_into_the_crop() {
        let mut canvas = Canvas::new(stripes(200, 100), 1.0, 3).expect("canvas");
        let style = Style {
            color: (1.0, 0.0, 0.0),
            line_width: 4.0,
            highlight: (1.0, 1.0, 0.0),
            font_size: 20.0,
        };
        let annotations = [
            Annotation::Blur(Rect::from_corners((100.0, 10.0), (190.0, 90.0))),
            Annotation::Box(Rect::from_corners((20.0, 20.0), (60.0, 60.0))),
        ];
        let crop = PixelRect {
            x: 10,
            y: 10,
            width: 180,
            height: 80,
        };
        let image = render(&mut canvas, &annotations, &style, crop).expect("render");
        assert_eq!((image.width(), image.height()), (180, 80));
        // Box outline at x=20 in screen space is x=10 in the crop.
        assert_eq!(pixel(&image, 10, 30), [255, 0, 0]);
        // Untouched stripes stay crisp; blurred stripes turn mid-gray.
        let plain = pixel(&image, 75, 70)[0];
        assert!(plain == 0 || plain == 255);
        let blurred = pixel(&image, 140, 40)[0];
        assert!((100..=155).contains(&blurred), "blurred value {blurred}");
    }

    #[test]
    fn plain_crop_skips_rendering() {
        let mut canvas = Canvas::new(stripes(20, 10), 1.0, 3).expect("canvas");
        let style = Style {
            color: (1.0, 0.0, 0.0),
            line_width: 2.0,
            highlight: (1.0, 1.0, 0.0),
            font_size: 20.0,
        };
        let crop = PixelRect {
            x: 1,
            y: 0,
            width: 5,
            height: 5,
        };
        let image = render(&mut canvas, &[], &style, crop).expect("render");
        assert_eq!(pixel(&image, 0, 0), [255, 255, 255]);
        assert_eq!(pixel(&image, 1, 0), [0, 0, 0]);
    }

    #[test]
    fn renders_pen_highlight_text_and_step() {
        // A plain white snapshot.
        let white = Pixbuf::from_bytes(
            &glib::Bytes::from_owned(vec![255u8; 200 * 100 * 3]),
            Colorspace::Rgb,
            false,
            8,
            200,
            100,
            600,
        );
        let mut canvas = Canvas::new(white, 1.0, 3).expect("canvas");
        let style = Style {
            color: (0.0, 0.0, 1.0),
            line_width: 4.0,
            highlight: (1.0, 1.0, 0.0),
            font_size: 20.0,
        };
        let annotations = [
            Annotation::Pen(vec![(10.0, 10.0), (60.0, 10.0)]),
            Annotation::Highlight(vec![(10.0, 50.0), (60.0, 50.0)]),
            Annotation::Text {
                at: (100.0, 10.0),
                text: "Hi".to_string(),
            },
            Annotation::Step {
                at: (150.0, 70.0),
                number: 3,
            },
        ];
        let crop = PixelRect {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        };
        let image = render(&mut canvas, &annotations, &style, crop).expect("render");
        assert_eq!(pixel(&image, 30, 10), [0, 0, 255]);
        // Yellow multiplied into white is yellow-ish: red and green high, blue low.
        let [r, g, b] = pixel(&image, 30, 50);
        assert!(r > 200 && g > 200 && b < 200, "highlight {r},{g},{b}");
        // The text left some ink in its box.
        let inked = (100..130)
            .flat_map(|x| (10..32).map(move |y| (x, y)))
            .any(|(x, y)| pixel(&image, x, y)[2] == 255 && pixel(&image, x, y)[0] < 128);
        assert!(inked, "text should be drawn");
        // The step marker's rim is filled with the color.
        assert_eq!(pixel(&image, 150 - 14, 70), [0, 0, 255]);
    }

    #[test]
    fn empty_strokes_and_text_are_degenerate() {
        assert!(Annotation::Pen(Vec::new()).is_degenerate());
        assert!(!Annotation::Pen(vec![(1.0, 1.0)]).is_degenerate());
        assert!(Annotation::Text {
            at: (0.0, 0.0),
            text: " \n".to_string()
        }
        .is_degenerate());
    }

    #[test]
    fn tiny_annotations_are_degenerate() {
        assert!(Annotation::Box(Rect::from_corners((0.0, 0.0), (2.0, 50.0))).is_degenerate());
        assert!(Annotation::Arrow {
            from: (0.0, 0.0),
            to: (3.0, 3.0)
        }
        .is_degenerate());
        assert!(!Annotation::Arrow {
            from: (0.0, 0.0),
            to: (30.0, 3.0)
        }
        .is_degenerate());
    }

    /// Writes a preview PNG of the annotations and toolbar; run explicitly with
    /// `TYSHOT_PREVIEW=<in.png>:<out.png> cargo test preview -- --ignored`.
    #[test]
    #[ignore]
    fn preview() {
        let spec = std::env::var("TYSHOT_PREVIEW").expect("TYSHOT_PREVIEW=in.png:out.png");
        let (input, output) = spec.split_once(':').expect("in.png:out.png");
        let snapshot = Pixbuf::from_file(input).expect("input image");
        let mut canvas = Canvas::new(snapshot, 1.0, 10).expect("canvas");
        let style = Style {
            color: (229.0 / 255.0, 72.0 / 255.0, 77.0 / 255.0),
            line_width: 3.0,
            highlight: (1.0, 214.0 / 255.0, 10.0 / 255.0),
            font_size: 20.0,
        };
        let sel = Rect::from_corners((300.0, 200.0), (1100.0, 700.0));
        let annotations = [
            Annotation::Blur(Rect::from_corners((350.0, 250.0), (700.0, 400.0))),
            Annotation::Box(Rect::from_corners((750.0, 300.0), (1050.0, 600.0))),
            Annotation::Arrow {
                from: (500.0, 650.0),
                to: (740.0, 450.0),
            },
            Annotation::Highlight(vec![(320.0, 450.0), (480.0, 452.0), (600.0, 448.0)]),
            Annotation::Pen(
                (0..40)
                    .map(|i| {
                        (
                            800.0 + i as f64 * 5.0,
                            640.0 + (i as f64 / 4.0).sin() * 20.0,
                        )
                    })
                    .collect(),
            ),
            Annotation::Text {
                at: (320.0, 500.0),
                text: "Text annotation\nsecond line".to_string(),
            },
            Annotation::Step {
                at: (760.0, 290.0),
                number: 1,
            },
            Annotation::Step {
                at: (1060.0, 610.0),
                number: 12,
            },
        ];
        let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, 1400, 900).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            paint_scene(&cr, &mut canvas, &annotations, &style);
            let bar = crate::toolbar::layout(&sel, 1400.0, 900.0, 44.0);
            crate::toolbar::draw(
                &cr,
                &bar,
                crate::toolbar::Button::Arrow,
                &[crate::toolbar::Button::Frame],
                Some(crate::toolbar::Button::Save),
                style.color,
            );
        }
        surface.flush();
        surface_to_pixbuf(&surface)
            .unwrap()
            .savev(output, "png", &[])
            .unwrap();
    }
}
