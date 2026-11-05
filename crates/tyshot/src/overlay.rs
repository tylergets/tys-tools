//! Full-screen overlay for selecting part of a frozen screen snapshot, Flameshot-style: drag
//! out a rectangle (or click a window to select it), move it or resize it by its edges and
//! handles, annotate it with boxes, arrows, pen and highlighter strokes, text, numbered steps
//! and blur from the toolbar beside it, optionally put it in a browser/phone/window frame,
//! then copy or save it.
//!
//! The snapshot is shown in a `ty_common::overlay::Overlay` covering every monitor (one popup
//! on X11, layer-shell surfaces on Wayland), so it behaves the same with or without a
//! compositor.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use gtk::cairo;
use gtk::gdk;
use gtk::gdk_pixbuf::Pixbuf;
use ty_common::overlay::Overlay;
use ty_common::screen::{self, ScreenRect};

use crate::annotate::{self, Annotation, Canvas, Style};
use crate::config::AppConfig;
use crate::opencode::{self, Target};
use crate::selection::{self, Edges, Hit, Rect};
use crate::toolbar::{self, Button};
use ty_common::frame::Frame;

/// How close (logical px) to an edge a press grabs it for resizing.
const HIT_SLOP: f64 = 8.0;
const HANDLE_RADIUS: f64 = 4.5;
/// Below this size (logical px) edge-midpoint handles are hidden to keep corners grabbable.
const MID_HANDLES_MIN: f64 = 40.0;
/// A drag smaller than this is a click: it selects the window under the pointer (or clears
/// the selection) instead.
const MIN_DRAG: f64 = 3.0;
const HINT: &str =
    "Click a window or drag to select   ·   Enter / Ctrl+C copy   ·   Ctrl+S save   ·   Esc cancel";
/// Pen and highlighter points closer than this (logical px) to the previous one are skipped.
const STROKE_STEP: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    /// Attach to an opencode session (possibly a new one).
    SendToOpencode(Target),
}

/// The selected part of the snapshot and what to do with it.
pub struct Capture {
    pub action: Action,
    pub image: Pixbuf,
}

type Done = Box<dyn FnOnce(Option<Capture>)>;

/// What dragging on the screen does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Create, move and resize the selection.
    Select,
    Box,
    Arrow,
    Pen,
    Highlight,
    /// Click to place a text cursor, then type.
    Text,
    /// Click to place the next numbered marker (drag to position it).
    Step,
    Blur,
}

impl Tool {
    fn button(self) -> Button {
        match self {
            Self::Select => Button::Select,
            Self::Box => Button::Box,
            Self::Arrow => Button::Arrow,
            Self::Pen => Button::Pen,
            Self::Highlight => Button::Highlight,
            Self::Text => Button::Text,
            Self::Step => Button::Step,
            Self::Blur => Button::Blur,
        }
    }

    /// The annotation a drag with this tool starts at `at`. `step` is the number the next
    /// step marker gets.
    fn start(self, at: (f64, f64), step: u32) -> Option<Annotation> {
        match self {
            Self::Select | Self::Text => None,
            Self::Box => Some(Annotation::Box(Rect::from_corners(at, at))),
            Self::Arrow => Some(Annotation::Arrow { from: at, to: at }),
            Self::Pen => Some(Annotation::Pen(vec![at])),
            Self::Highlight => Some(Annotation::Highlight(vec![at])),
            Self::Step => Some(Annotation::Step { at, number: step }),
            Self::Blur => Some(Annotation::Blur(Rect::from_corners(at, at))),
        }
    }
}

/// Follow the pointer to `to` while dragging out `annotation`, which started at `start`.
fn extend(annotation: &mut Annotation, start: (f64, f64), to: (f64, f64)) {
    match annotation {
        Annotation::Box(rect) | Annotation::Blur(rect) => *rect = Rect::from_corners(start, to),
        Annotation::Arrow { to: end, .. } => *end = to,
        Annotation::Pen(points) | Annotation::Highlight(points) => {
            let far_enough = points
                .last()
                .is_none_or(|last| (to.0 - last.0).hypot(to.1 - last.1) >= STROKE_STEP);
            if far_enough {
                points.push(to);
            }
        }
        Annotation::Step { at, .. } => *at = to,
        Annotation::Text { .. } => {}
    }
}

/// Text being typed with the text tool.
struct TextEdit {
    at: (f64, f64),
    text: String,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Create {
        anchor: (f64, f64),
    },
    Move {
        start: (f64, f64),
        original: Rect,
    },
    Resize {
        start: (f64, f64),
        original: Rect,
        edges: Edges,
    },
    Draw {
        start: (f64, f64),
    },
}

struct State {
    canvas: Canvas,
    /// Snapshot pixels per logical pixel.
    scale: f64,
    /// Overlay size in logical pixels.
    width: f64,
    height: f64,
    selection: Option<Rect>,
    drag: Option<Drag>,
    tool: Tool,
    annotations: Vec<Annotation>,
    /// Undone annotations, most recently undone last, for redo. Cleared by a new annotation.
    undone: Vec<Annotation>,
    /// The annotation being dragged out.
    pending: Option<Annotation>,
    /// Text being typed; it becomes an annotation when committed.
    editing: Option<TextEdit>,
    /// Windows visible in the snapshot, topmost first (logical px, clipped to the screen).
    windows: Vec<Rect>,
    /// The window under the pointer, shown while there is no selection; a click selects it.
    hover_window: Option<Rect>,
    hover: Option<Button>,
    cursor: gdk::CursorType,
    /// Where the hint is centered (logical px): the monitor under the pointer at start.
    hint_center: (f64, f64),
    color: (f64, f64, f64),
    dim: f64,
    style: Style,
    /// Decorative frame around the selection, included in the copied/saved image.
    frame: Frame,
    /// Toolbar button size, logical px.
    button_size: f64,
    /// The picker's rows: opencode sessions, most recently active first, then new sessions.
    /// Fetched when the picker opens.
    targets: Vec<Target>,
    /// The list for choosing which session to send to, while it is open.
    picker: Option<Picker>,
    /// A message shown until the next click or key press.
    notice: Option<String>,
    done: Option<Done>,
}

/// The session picker: one row per target in `State::targets`.
#[derive(Default)]
struct Picker {
    /// Row rectangles (logical px), set when drawn; used for hit testing.
    rows: Vec<Rect>,
    /// The highlighted row: under the pointer, or moved with the arrow keys.
    selected: usize,
}

impl Picker {
    fn hit(&self, point: (f64, f64)) -> Option<usize> {
        self.rows.iter().position(|row| row.contains(point))
    }
}

impl State {
    /// The toolbar, when it is shown: there is a selection and nothing is being dragged.
    fn toolbar(&self) -> Option<toolbar::Layout> {
        match (self.selection, self.drag) {
            (Some(rect), None) => Some(toolbar::layout(
                &self.frame.outer(&rect),
                self.width,
                self.height,
                self.button_size,
            )),
            _ => None,
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Blur {
            self.canvas.prepare_blur();
        }
        self.tool = tool;
    }

    /// Add a finished annotation (unless it is degenerate). Redo history is dropped.
    fn push_annotation(&mut self, annotation: Annotation) {
        if !annotation.is_degenerate() {
            self.annotations.push(annotation);
            self.undone.clear();
        }
    }

    fn undo(&mut self) {
        self.commit_text();
        if let Some(annotation) = self.annotations.pop() {
            self.undone.push(annotation);
        }
    }

    fn redo(&mut self) {
        self.commit_text();
        if let Some(annotation) = self.undone.pop() {
            self.annotations.push(annotation);
        }
    }

    /// Turn the text being typed (if any) into an annotation.
    fn commit_text(&mut self) {
        if let Some(TextEdit { at, text }) = self.editing.take() {
            self.push_annotation(Annotation::Text { at, text });
        }
    }

    /// The number the next step marker gets: one more than the highest one placed.
    fn next_step(&self) -> u32 {
        self.annotations
            .iter()
            .filter_map(|annotation| match annotation {
                Annotation::Step { number, .. } => Some(*number),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }

    /// The topmost window under `point`.
    fn window_at(&self, point: (f64, f64)) -> Option<Rect> {
        self.windows
            .iter()
            .find(|rect| rect.contains(point))
            .copied()
    }
}

/// Show the overlay over `snapshot` (the whole screen, see `ty_common::screen`). `windows`
/// (screen pixels, topmost first) are the windows a click can select. `on_done` gets the
/// annotated selection, or `None` if the user cancelled (Escape, right-click or Close). Must
/// be called from the GTK main loop's thread; the result arrives from inside the loop.
pub fn select(
    snapshot: Pixbuf,
    windows: Vec<ScreenRect>,
    config: &AppConfig,
    on_done: impl FnOnce(Option<Capture>) + 'static,
) -> Result<()> {
    let overlay = Overlay::new("tyshot")?;
    let (width, height) = overlay.size();
    // The snapshot can be larger than the overlay (HiDPI): one logical pixel covers this many
    // snapshot pixels.
    let scale = (f64::from(snapshot.width()) / width).max(1.0);
    let blur_radius = (f64::from(config.annotation.blur_radius) * scale).round() as usize;
    let screen_scale = screen::logical_scale();
    let windows = windows
        .into_iter()
        .map(|window| {
            let logical = |value: i64| value as f64 / screen_scale;
            Rect::from_corners(
                (logical(window.x.into()), logical(window.y.into())),
                (
                    logical(i64::from(window.x) + i64::from(window.width)),
                    logical(i64::from(window.y) + i64::from(window.height)),
                ),
            )
            .clamp_to(width, height)
        })
        .filter(|rect| rect.width() >= MIN_DRAG && rect.height() >= MIN_DRAG)
        .collect();

    let state = Rc::new(RefCell::new(State {
        canvas: Canvas::new(snapshot, scale, blur_radius)?,
        scale,
        width,
        height,
        selection: None,
        drag: None,
        tool: Tool::Select,
        annotations: Vec::new(),
        undone: Vec::new(),
        pending: None,
        editing: None,
        windows,
        hover_window: None,
        hover: None,
        cursor: gdk::CursorType::Crosshair,
        hint_center: overlay.pointer_monitor_center(),
        color: config.overlay.rgb().unwrap_or((0.9, 0.28, 0.3)),
        dim: config.overlay.dim,
        style: Style {
            color: config.annotation.rgb().unwrap_or((0.9, 0.28, 0.3)),
            line_width: config.annotation.line_width,
            highlight: config
                .annotation
                .highlight_rgb()
                .unwrap_or((1.0, 0.84, 0.04)),
            font_size: config.annotation.font_size,
        },
        frame: Frame::None,
        button_size: config.toolbar.button_size,
        targets: Vec::new(),
        picker: None,
        notice: None,
        done: Some(Box::new(on_done)),
    }));

    {
        let state = state.clone();
        overlay.connect_draw(move |cr| draw(cr, &mut state.borrow_mut()));
    }
    {
        let state = state.clone();
        overlay.connect_button_press(move |overlay, event, position| {
            on_press(overlay, &state, event, position);
        });
    }
    {
        let state = state.clone();
        overlay.connect_motion(move |overlay, position| {
            on_motion(overlay, &mut state.borrow_mut(), position);
        });
    }
    {
        let state = state.clone();
        overlay.connect_button_release(move |overlay, event, position| {
            if event.button() == 1 {
                let mut s = state.borrow_mut();
                on_motion(overlay, &mut s, position);
                on_release(overlay, &mut s);
                // Refresh hover/cursor now that the toolbar may be back.
                on_motion(overlay, &mut s, position);
            }
        });
    }
    {
        let state = state.clone();
        overlay.connect_key_press(move |overlay, event| on_key(overlay, &state, event));
    }

    let on_grab_failed = {
        let (overlay, state) = (overlay.clone(), state.clone());
        move || finish(&overlay, &state, None)
    };
    overlay.show(gdk::CursorType::Crosshair, on_grab_failed);
    Ok(())
}

fn on_press(
    window: &Overlay,
    state: &Rc<RefCell<State>>,
    event: &gdk::EventButton,
    position: (f64, f64),
) {
    if event.button() == 3 {
        finish(window, state, None);
        return;
    }
    if event.button() != 1 {
        return;
    }
    let mut s = state.borrow_mut();
    if s.notice.take().is_some() {
        window.queue_draw();
    }
    if let Some(picker) = s.picker.take() {
        // A click picks a row, or closes the picker anywhere else.
        if let Some(target) = picker.hit(position).map(|index| s.targets[index].clone()) {
            drop(s);
            finish(window, state, Some(Action::SendToOpencode(target)));
        } else {
            window.queue_draw();
        }
        return;
    }
    // Clicking anywhere (a toolbar button, another spot) finishes the text being typed.
    s.commit_text();

    if let Some(bar) = s.toolbar().filter(|bar| bar.contains(position)) {
        // Ignore the second press of a double-click so buttons don't fire twice.
        if event.event_type() == gdk::EventType::ButtonPress {
            if let Some(button) = bar.hit(position) {
                drop(s);
                press_button(window, state, button);
            }
        }
        return;
    }

    if event.event_type() == gdk::EventType::DoubleButtonPress {
        if s.tool == Tool::Select && s.selection.is_some_and(|rect| rect.contains(position)) {
            drop(s);
            finish(window, state, Some(Action::Copy));
        }
        return;
    }

    let (width, height) = (s.width, s.height);
    let position = (position.0.clamp(0.0, width), position.1.clamp(0.0, height));
    if s.tool == Tool::Text && s.selection.is_some() {
        s.editing = Some(TextEdit {
            at: position,
            text: String::new(),
        });
        window.queue_draw();
        return;
    }
    s.drag = Some(if s.tool != Tool::Select && s.selection.is_some() {
        s.pending = s.tool.start(position, s.next_step());
        Drag::Draw { start: position }
    } else {
        match s.selection.map(|rect| (rect, hit(&rect, position))) {
            Some((original, Hit::Resize(edges))) => Drag::Resize {
                start: position,
                original,
                edges,
            },
            Some((original, Hit::Move)) => Drag::Move {
                start: position,
                original,
            },
            _ => {
                s.selection = None;
                Drag::Create { anchor: position }
            }
        }
    });
    window.queue_draw();
}

fn press_button(window: &Overlay, state: &Rc<RefCell<State>>, button: Button) {
    let mut s = state.borrow_mut();
    match button {
        Button::Select => s.set_tool(Tool::Select),
        Button::Box => s.set_tool(Tool::Box),
        Button::Arrow => s.set_tool(Tool::Arrow),
        Button::Pen => s.set_tool(Tool::Pen),
        Button::Highlight => s.set_tool(Tool::Highlight),
        Button::Text => s.set_tool(Tool::Text),
        Button::Step => s.set_tool(Tool::Step),
        Button::Blur => s.set_tool(Tool::Blur),
        Button::Frame => s.frame = s.frame.next(),
        Button::Undo => s.undo(),
        Button::Redo => s.redo(),
        Button::Agent => {
            drop(s);
            send_to_agent(window, state);
            return;
        }
        Button::Copy | Button::Save | Button::Close => {
            drop(s);
            let action = match button {
                Button::Copy => Some(Action::Copy),
                Button::Save => Some(Action::Save),
                _ => None,
            };
            finish(window, state, action);
            return;
        }
    }
    window.queue_draw();
}

/// Open the opencode session picker, or say why it can't be opened. Asks the local opencode
/// service for its sessions (a quick local request).
fn send_to_agent(window: &Overlay, state: &Rc<RefCell<State>>) {
    let mut s = state.borrow_mut();
    match opencode::Service::discover().and_then(|service| service.sessions(PICKER_SESSIONS)) {
        Ok(sessions) => {
            s.targets = opencode::targets(sessions, PICKER_NEW);
            s.picker = Some(Picker::default());
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            s.notice = Some(format!("{err}"));
        }
    }
    window.queue_draw();
}

/// Keys while the session picker is open: 1–9 or arrows + Enter pick, Escape closes it.
fn picker_key(window: &Overlay, state: &Rc<RefCell<State>>, keyval: gdk::keys::Key) {
    use gdk::keys::constants as key;

    let mut s = state.borrow_mut();
    let count = s.targets.len();
    let Some(picker) = s.picker.as_mut() else {
        return;
    };
    let digit = keyval
        .to_unicode()
        .and_then(|c| c.to_digit(10))
        .filter(|&digit| (1..=count as u32).contains(&digit));
    let chosen = match keyval {
        _ if digit.is_some() => digit.map(|digit| digit as usize - 1),
        key::Return | key::KP_Enter => Some(picker.selected),
        key::Up => {
            picker.selected = (picker.selected + count - 1) % count;
            None
        }
        key::Down | key::Tab => {
            picker.selected = (picker.selected + 1) % count;
            None
        }
        key::Escape => {
            s.picker = None;
            None
        }
        _ => None,
    };
    if let Some(index) = chosen {
        let target = s.targets[index].clone();
        s.picker = None;
        drop(s);
        finish(window, state, Some(Action::SendToOpencode(target)));
        return;
    }
    window.queue_draw();
}

fn on_motion(window: &Overlay, s: &mut State, (x, y): (f64, f64)) {
    if let Some(picker) = s.picker.as_mut() {
        if let Some(index) = picker.hit((x, y)).filter(|&index| index != picker.selected) {
            picker.selected = index;
            window.queue_draw();
        }
        set_cursor(window, gdk::CursorType::Hand2);
        return;
    }
    let (width, height) = (s.width, s.height);
    let (cx, cy) = (x.clamp(0.0, width), y.clamp(0.0, height));
    match s.drag {
        None => {
            let bar = s.toolbar();
            let hover = bar.as_ref().and_then(|bar| bar.hit((x, y)));
            let cursor = if bar.is_some_and(|bar| bar.contains((x, y))) {
                gdk::CursorType::Hand2
            } else {
                match (s.tool, &s.selection) {
                    (Tool::Select, Some(rect)) => cursor_for(hit(rect, (x, y))),
                    (Tool::Text, Some(_)) => gdk::CursorType::Xterm,
                    _ => gdk::CursorType::Crosshair,
                }
            };
            let hover_window = match s.selection {
                None => s.window_at((x, y)),
                Some(_) => None,
            };
            if hover_window != s.hover_window {
                s.hover_window = hover_window;
                window.queue_draw();
            }
            if cursor != s.cursor {
                s.cursor = cursor;
                set_cursor(window, cursor);
            }
            if hover != s.hover {
                s.hover = hover;
                window.queue_draw();
            }
            return;
        }
        Some(Drag::Create { anchor }) => {
            s.selection = Some(Rect::from_corners(anchor, (cx, cy)));
        }
        Some(Drag::Move { start, original }) => {
            s.selection = Some(original.translate_within(x - start.0, y - start.1, width, height));
        }
        Some(Drag::Resize {
            start,
            original,
            edges,
        }) => {
            s.selection = Some(
                selection::resize(&original, edges, x - start.0, y - start.1)
                    .clamp_to(width, height),
            );
        }
        Some(Drag::Draw { start }) => {
            if let Some(pending) = &mut s.pending {
                extend(pending, start, (cx, cy));
            }
        }
    }
    window.queue_draw();
}

fn on_release(window: &Overlay, s: &mut State) {
    let Some(drag) = s.drag.take() else {
        return;
    };
    if let Drag::Draw { .. } = drag {
        if let Some(annotation) = s.pending.take() {
            s.push_annotation(annotation);
        }
    } else if s
        .selection
        .is_some_and(|rect| rect.width() < MIN_DRAG || rect.height() < MIN_DRAG)
    {
        // A click without a drag selects the window under it; a resize down to nothing
        // clears the selection.
        s.selection = match drag {
            Drag::Create { anchor } => s.window_at(anchor),
            _ => None,
        };
        s.hover_window = None;
    }
    window.queue_draw();
}

fn on_key(window: &Overlay, state: &Rc<RefCell<State>>, event: &gdk::EventKey) {
    use gdk::keys::constants as key;

    let modifiers = event.state();
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    if state.borrow().picker.is_some() {
        return picker_key(window, state, event.keyval());
    }
    if state.borrow_mut().notice.take().is_some() {
        window.queue_draw();
    }
    if edit_text(window, &mut state.borrow_mut(), event, ctrl, shift) {
        return;
    }
    let keyval = event.keyval().to_lower();

    let action = match keyval {
        key::Escape => return finish(window, state, None),
        key::Return | key::KP_Enter if ctrl => return send_to_agent(window, state),
        key::Return | key::KP_Enter => Some(Action::Copy),
        key::c if ctrl => Some(Action::Copy),
        key::s if ctrl => Some(Action::Save),
        _ => None,
    };
    if action.is_some() {
        return finish(window, state, action);
    }

    let mut s = state.borrow_mut();
    let (width, height) = (s.width, s.height);
    if ctrl {
        match keyval {
            key::a => {
                s.selection = Some(Rect::from_corners((0.0, 0.0), (width, height)));
                s.drag = None;
            }
            key::z if shift => s.redo(),
            key::z => s.undo(),
            key::y => s.redo(),
            _ => return,
        }
        window.queue_draw();
        return;
    }

    if keyval == key::f {
        if s.selection.is_some() && s.drag.is_none() {
            s.frame = s.frame.next();
            window.queue_draw();
        }
        return;
    }

    // Tool shortcuts, once there is something to annotate.
    let tool = match keyval {
        key::v => Some(Tool::Select),
        key::r => Some(Tool::Box),
        key::a => Some(Tool::Arrow),
        key::p => Some(Tool::Pen),
        key::h => Some(Tool::Highlight),
        key::t => Some(Tool::Text),
        key::n => Some(Tool::Step),
        key::b => Some(Tool::Blur),
        _ => None,
    };
    if let Some(tool) = tool {
        if s.selection.is_some() && s.drag.is_none() {
            s.set_tool(tool);
            window.queue_draw();
        }
        return;
    }

    let (dx, dy) = match keyval {
        key::Left => (-1.0, 0.0),
        key::Right => (1.0, 0.0),
        key::Up => (0.0, -1.0),
        key::Down => (0.0, 1.0),
        _ => return,
    };
    let Some(rect) = s.selection.filter(|_| s.drag.is_none()) else {
        return;
    };
    s.selection = Some(if shift {
        // Shift+arrows grow/shrink from the bottom-right corner.
        let edges = Edges {
            right: true,
            bottom: true,
            ..Edges::default()
        };
        let resized = selection::resize(&rect, edges, dx, dy).clamp_to(width, height);
        if resized.width() < 1.0 || resized.height() < 1.0 {
            rect
        } else {
            Rect {
                x0: rect.x0,
                y0: rect.y0,
                ..resized
            }
        }
    } else {
        rect.translate_within(dx, dy, width, height)
    });
    window.queue_draw();
}

/// Typing into the text being edited. Returns whether the key was used: Ctrl shortcuts
/// commit the text and are then handled as usual.
fn edit_text(
    window: &Overlay,
    s: &mut State,
    event: &gdk::EventKey,
    ctrl: bool,
    shift: bool,
) -> bool {
    use gdk::keys::constants as key;

    let Some(edit) = &mut s.editing else {
        return false;
    };
    if ctrl {
        s.commit_text();
        window.queue_draw();
        return false;
    }
    match event.keyval() {
        // Esc throws the text away (and doesn't close the overlay).
        key::Escape => s.editing = None,
        key::Return | key::KP_Enter if shift => edit.text.push('\n'),
        key::Return | key::KP_Enter => s.commit_text(),
        key::BackSpace => {
            edit.text.pop();
        }
        keyval => match keyval.to_unicode() {
            Some(c) if !c.is_control() => edit.text.push(c),
            // Other keys (arrows, modifiers, …) do nothing while typing.
            _ => return true,
        },
    }
    window.queue_draw();
    true
}

fn hit(rect: &Rect, position: (f64, f64)) -> Hit {
    selection::hit_test(rect, position, HIT_SLOP)
}

fn cursor_for(hit: Hit) -> gdk::CursorType {
    use gdk::CursorType as C;
    match hit {
        Hit::Outside => C::Crosshair,
        Hit::Move => C::Fleur,
        Hit::Resize(edges) => match (edges.left, edges.right, edges.top, edges.bottom) {
            (true, _, true, _) => C::TopLeftCorner,
            (_, true, true, _) => C::TopRightCorner,
            (true, _, _, true) => C::BottomLeftCorner,
            (_, true, _, true) => C::BottomRightCorner,
            (true, ..) => C::LeftSide,
            (_, true, ..) => C::RightSide,
            (_, _, true, _) => C::TopSide,
            _ => C::BottomSide,
        },
    }
}

fn set_cursor(window: &Overlay, cursor: gdk::CursorType) {
    window.set_cursor(cursor);
}

fn draw(cr: &cairo::Context, s: &mut State) {
    s.canvas.paint_snapshot(cr);
    let selection = s.selection.map(|rect| snap(&rect, s.scale));

    // Annotations, clipped to the selection (and frame) like the final image.
    if let Some(rect) = &selection {
        let editing = s.editing.as_ref().map(|edit| Annotation::Text {
            at: edit.at,
            text: edit.text.clone(),
        });
        let in_progress: Vec<Annotation> = s.pending.iter().cloned().chain(editing).collect();
        if !s.annotations.is_empty() || !in_progress.is_empty() {
            cr.save().ok();
            s.frame.clip_content(cr, rect);
            annotate::paint_annotations(cr, &mut s.canvas, &s.annotations, &s.style);
            annotate::paint_annotations(cr, &mut s.canvas, &in_progress, &s.style);
            cr.restore().ok();
        }
    }

    // Shade everything except the selection, or the window a click would select (even-odd
    // leaves the inner rectangle untouched).
    let hover_window = match (selection, s.drag) {
        (None, None) => s.hover_window.map(|rect| snap(&rect, s.scale)),
        _ => None,
    };
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.rectangle(0.0, 0.0, s.width, s.height);
    if let Some(rect) = selection.or(hover_window) {
        cr.rectangle(rect.x0, rect.y0, rect.width(), rect.height());
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, s.dim);
    let _ = cr.fill();
    cr.set_fill_rule(cairo::FillRule::Winding);

    let Some(rect) = selection else {
        if let Some(rect) = hover_window {
            draw_window_hover(cr, s, &rect);
        }
        if s.drag.is_none() {
            draw_hint(cr, s);
        }
        return;
    };

    // The text cursor.
    if let Some(edit) = &s.editing {
        let (top, bottom) = annotate::text_caret(cr, edit.at, &edit.text, s.style.font_size);
        let (r, g, b) = s.style.color;
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.0);
        cr.move_to(top.0 + 2.0, top.1);
        cr.line_to(bottom.0 + 2.0, bottom.1);
        let _ = cr.stroke();
    }

    // The frame, undimmed, exactly as it will appear in the image.
    s.frame.paint(cr, &rect);
    let outer = s.frame.outer(&rect);

    // The outline would cover the frame's edge, so with a frame it only shows while the
    // selection can be adjusted.
    let (r, g, b) = s.color;
    if s.frame == Frame::None || s.tool == Tool::Select {
        cr.set_source_rgb(r, g, b);
        cr.set_line_width(2.0);
        cr.rectangle(
            rect.x0 - 1.0,
            rect.y0 - 1.0,
            rect.width() + 2.0,
            rect.height() + 2.0,
        );
        let _ = cr.stroke();
    }

    // Handles only where they work: with the select tool.
    if s.tool == Tool::Select {
        let show_mid = rect.width() >= MID_HANDLES_MIN && rect.height() >= MID_HANDLES_MIN;
        for (index, (x, y)) in selection::handles(&rect).into_iter().enumerate() {
            if index >= 4 && !show_mid {
                break;
            }
            cr.arc(x, y, HANDLE_RADIUS, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(r, g, b);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }
    }

    let snapshot = &s.canvas.snapshot;
    if let Some(pixels) = rect.to_pixels(s.scale, snapshot.width() as u32, snapshot.height() as u32)
    {
        let label = match s.frame {
            Frame::None => format!("{} × {}", pixels.width, pixels.height),
            frame => format!(
                "{} × {}  ·  {} frame",
                pixels.width,
                pixels.height,
                frame.name()
            ),
        };
        let (label_w, label_h) = pill_size(cr, &label);
        // Above the top-left corner (of the frame, if any), or just inside the selection when
        // there is no room above.
        let y = if outer.y0 >= label_h + 8.0 {
            outer.y0 - label_h - 8.0
        } else {
            rect.y0 + 8.0
        };
        let x = outer.x0.min(s.width - label_w).max(0.0);
        draw_pill(cr, &label, x, y);
    }

    if let Some(bar) = s.toolbar() {
        let toggled: &[Button] = match s.frame {
            Frame::None => &[],
            _ => &[Button::Frame],
        };
        toolbar::draw(cr, &bar, s.tool.button(), toggled, s.hover, s.color);
        if let Some((button, button_rect)) = s
            .hover
            .and_then(|hover| bar.buttons.iter().find(|(button, _)| *button == hover))
        {
            let text = match button {
                Button::Frame => format!(
                    "Frame: {} (F for {})",
                    s.frame.name(),
                    s.frame.next().name()
                ),
                _ => button.tooltip().to_string(),
            };
            if s.picker.is_none() {
                draw_tooltip(cr, s, &bar.background, button_rect, &text);
            }
        }
        if s.picker.is_some() {
            if let Some((_, anchor)) = bar.buttons.iter().find(|(b, _)| *b == Button::Agent) {
                draw_picker(cr, s, &bar.background, anchor);
            }
        }
    }
    if let Some(notice) = &s.notice {
        let (w, h) = pill_size(cr, notice);
        let (cx, cy) = s.hint_center;
        draw_pill(cr, notice, cx - w / 2.0, cy - h / 2.0);
    }
}

/// Sessions offered in the picker, and new-session rows after them; 1–9 pick rows by number,
/// so together they stay within 9.
const PICKER_SESSIONS: usize = 6;
const PICKER_NEW: usize = 3;
/// Session titles longer than this many characters are cut short with "…".
const TITLE_MAX: usize = 60;
const PICKER_ROW: f64 = 30.0;
const PICKER_PAD: f64 = 6.0;

/// The session list, on the far side of the toolbar from the selection when it fits, lined up
/// with the agent button. Records the row rectangles for hit testing.
fn draw_picker(cr: &cairo::Context, s: &mut State, bar: &Rect, anchor: &Rect) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    let lines: Vec<(String, String)> = s
        .targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let (title, detail) = target.describe(now_ms);
            let title = match title.char_indices().nth(TITLE_MAX) {
                Some((cut, _)) => format!("{}…", &title[..cut]),
                None => title,
            };
            (format!("{}   {title}", index + 1), detail)
        })
        .collect();

    set_font(cr);
    let measure = |text: &str| cr.text_extents(text).map(|e| e.x_advance()).unwrap_or(0.0);
    let label_w = lines
        .iter()
        .map(|(label, _)| measure(label))
        .fold(0.0, f64::max);
    let detail_w = lines
        .iter()
        .map(|(_, detail)| measure(detail))
        .fold(0.0, f64::max);
    let w = label_w + detail_w + 40.0 + PICKER_PAD * 2.0;
    let h = lines.len() as f64 * PICKER_ROW + PICKER_PAD * 2.0;

    let below_selection = s.selection.is_some_and(|rect| bar.y0 >= rect.y1);
    let y = if below_selection && bar.y1 + 6.0 + h <= s.height {
        bar.y1 + 6.0
    } else if bar.y0 - 6.0 - h >= 0.0 {
        bar.y0 - 6.0 - h
    } else {
        (bar.y1 + 6.0).min(s.height - h).max(0.0)
    };
    let x = (anchor.x1 - w).min(s.width - w).max(0.0);

    toolbar::rounded_rect(cr, &Rect::from_corners((x, y), (x + w, y + h)), 9.0);
    cr.set_source_rgba(0.08, 0.08, 0.1, 0.95);
    let _ = cr.fill();

    let (ascent, descent) = cr
        .font_extents()
        .map(|e| (e.ascent(), e.descent()))
        .unwrap_or((FONT_SIZE, 0.0));
    let (r, g, b) = s.color;
    let mut rows = Vec::new();
    let selected = s.picker.as_ref().map_or(0, |picker| picker.selected);
    for (index, (label, detail)) in lines.iter().enumerate() {
        let top = y + PICKER_PAD + index as f64 * PICKER_ROW;
        let row = Rect::from_corners(
            (x + PICKER_PAD, top),
            (x + w - PICKER_PAD, top + PICKER_ROW),
        );
        if index == selected {
            toolbar::rounded_rect(cr, &row, 6.0);
            cr.set_source_rgb(r, g, b);
            let _ = cr.fill();
        }
        let baseline = top + (PICKER_ROW + ascent - descent) / 2.0;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(row.x0 + 10.0, baseline);
        let _ = cr.show_text(label);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.65);
        cr.move_to(row.x1 - 10.0 - measure(detail), baseline);
        let _ = cr.show_text(detail);
        rows.push(row);
    }
    if let Some(picker) = s.picker.as_mut() {
        picker.rows = rows;
    }
}

/// Outline the window a click would select, with its size.
fn draw_window_hover(cr: &cairo::Context, s: &State, rect: &Rect) {
    let (r, g, b) = s.color;
    cr.set_source_rgb(r, g, b);
    cr.set_line_width(2.0);
    cr.set_dash(&[6.0, 4.0], 0.0);
    cr.rectangle(
        rect.x0 + 1.0,
        rect.y0 + 1.0,
        rect.width() - 2.0,
        rect.height() - 2.0,
    );
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);

    let snapshot = &s.canvas.snapshot;
    if let Some(pixels) = rect.to_pixels(s.scale, snapshot.width() as u32, snapshot.height() as u32)
    {
        let label = format!("{} × {}  ·  click to select", pixels.width, pixels.height);
        let (label_w, label_h) = pill_size(cr, &label);
        let x = (rect.x0 + 8.0).min(s.width - label_w).max(0.0);
        draw_pill(cr, &label, x, (rect.y0 + 8.0).min(s.height - label_h));
    }
}

/// Tooltip centered on the button, on the far side of the toolbar from the selection when
/// possible.
fn draw_tooltip(cr: &cairo::Context, s: &State, bar: &Rect, button: &Rect, text: &str) {
    let (w, h) = pill_size(cr, text);
    let below_selection = s.selection.is_some_and(|rect| bar.y0 >= rect.y1);
    let y = if below_selection && bar.y1 + 6.0 + h <= s.height {
        bar.y1 + 6.0
    } else if bar.y0 - 6.0 - h >= 0.0 {
        bar.y0 - 6.0 - h
    } else {
        bar.y1 + 6.0
    };
    let x = ((button.x0 + button.x1) / 2.0 - w / 2.0)
        .min(s.width - w)
        .max(0.0);
    draw_pill(cr, text, x, y);
}

fn draw_hint(cr: &cairo::Context, s: &State) {
    let (w, h) = pill_size(cr, HINT);
    let (cx, cy) = s.hint_center;
    draw_pill(cr, HINT, cx - w / 2.0, cy - h / 2.0);
}

const PILL_PAD_X: f64 = 10.0;
const PILL_PAD_Y: f64 = 6.0;
const FONT_SIZE: f64 = 13.0;

fn set_font(cr: &cairo::Context) {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(FONT_SIZE);
}

fn pill_size(cr: &cairo::Context, text: &str) -> (f64, f64) {
    set_font(cr);
    let width = cr.text_extents(text).map(|e| e.x_advance()).unwrap_or(0.0);
    (width + PILL_PAD_X * 2.0, FONT_SIZE + PILL_PAD_Y * 2.0)
}

/// White text on a rounded dark background with its top-left corner at `(x, y)`.
fn draw_pill(cr: &cairo::Context, text: &str, x: f64, y: f64) {
    let (w, h) = pill_size(cr, text);
    toolbar::rounded_rect(cr, &Rect::from_corners((x, y), (x + w, y + h)), h / 2.0);
    cr.set_source_rgba(0.08, 0.08, 0.1, 0.85);
    let _ = cr.fill();

    let (ascent, descent) = cr
        .font_extents()
        .map(|e| (e.ascent(), e.descent()))
        .unwrap_or((FONT_SIZE, 0.0));
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(x + PILL_PAD_X, y + (h + ascent - descent) / 2.0);
    let _ = cr.show_text(text);
}

/// Round to whole device pixels, so what is outlined is exactly what gets cropped.
fn snap(rect: &Rect, scale: f64) -> Rect {
    let round = |value: f64| (value * scale).round() / scale;
    Rect {
        x0: round(rect.x0),
        y0: round(rect.y0),
        x1: round(rect.x1),
        y1: round(rect.y1),
    }
}

/// Close the overlay and report the result. With no selection, `action` applies to the whole
/// screen.
fn finish(window: &Overlay, state: &Rc<RefCell<State>>, action: Option<Action>) {
    let mut s = state.borrow_mut();
    let Some(done) = s.done.take() else {
        return;
    };
    // Text still being typed is part of the image.
    s.commit_text();
    let capture = action.and_then(|action| {
        let snapshot = &s.canvas.snapshot;
        let (snapshot_w, snapshot_h) = (snapshot.width() as u32, snapshot.height() as u32);
        let rect = s
            .selection
            .unwrap_or_else(|| Rect::from_corners((0.0, 0.0), (s.width, s.height)));
        let crop = rect.to_pixels(s.scale, snapshot_w, snapshot_h)?;
        let scale = s.scale;
        let State {
            canvas,
            annotations,
            style,
            frame,
            ..
        } = &mut *s;
        let rendered = match *frame {
            Frame::None => annotate::render(canvas, annotations, style, crop),
            // The frame goes around the whole selection; the image grows to fit it.
            frame => {
                annotate::render_framed(canvas, annotations, style, &snap(&rect, scale), frame)
            }
        };
        match rendered {
            Ok(image) => Some(Capture { action, image }),
            Err(err) => {
                eprintln!("error: {err:#}");
                None
            }
        }
    });
    drop(s);

    window.close();
    done(capture);
}
