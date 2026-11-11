//! The prompt: one window per request from polkitd, running one helper attempt at a time.
//!
//! What the window shows follows PAM. If PAM asks for a password, the password field is
//! shown. If it asks for a finger (pam_fprintd), a fingerprint glyph is shown instead, with
//! the password field behind a "Use password" button; what you type there is kept and sent
//! the moment PAM asks for a password. PAM's errors ("Failed to match fingerprint",
//! "Authentication failure") are shown under the field or glyph.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use dbus::channel::Sender as _;
use dbus::strings::ErrorName;
use dbus::Message;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;
use gtk::{cairo, gdk, pango};
use ty_common::icons::ATTENTION_COLOR;
use ty_common::overlay;
use ty_common::svgpath;
use ty_common::tray;

use crate::appinfo::App;
use crate::config::{AppConfig, APP_NAME};
use crate::fprint;
use crate::helper::{self, Event};
use crate::polkit::{Bus, Request, ERROR_CANCELLED, SUBJECT_PID_DETAIL};
use crate::reasons::Reasons;
use crate::users::{self, User};

const DEFAULT_ICON: &str = "dialog-password";
/// Details polkitd consumes itself; everything else is shown.
const HIDDEN_DETAILS: &[&str] = &[
    "polkit.gettext_domain",
    "polkit.message",
    "polkit.icon_name",
];
const WIDTH: i32 = 420;
const WRAP_CHARS: i32 = 44;
/// Header icon size, logical px.
const ICON_PX: i32 = 48;
/// How dark `dim_screen` makes the rest of the screen.
const DIM_ALPHA: f64 = 0.6;
/// The fingerprint glyph, logical px.
const GLYPH_SIZE: i32 = 84;
/// Lucide's fingerprint (ISC), drawn on a 24px grid with 2px strokes.
const FINGERPRINT_SVG: &str = include_str!("../icons/fingerprint.svg");
const ICON_GRID: f64 = 24.0;
const ICON_STROKE: f64 = 1.75;
/// The glyph breathes while PAM waits for a finger, and flashes red when one is refused.
const FRAME: Duration = Duration::from_millis(33);
const PULSE: Duration = Duration::from_millis(2400);
const FLASH: Duration = Duration::from_millis(900);

/// The open prompts, by cookie.
pub struct Dialogs {
    config: AppConfig,
    conn: Rc<Bus>,
    open: RefCell<HashMap<String, Rc<Prompt>>>,
    /// Who has fingerprints enrolled, by user name (asked of fprintd once per user).
    fingerprints: RefCell<HashMap<String, bool>>,
    /// What `typk verify --reason` said, by pid.
    pub reasons: RefCell<Reasons>,
}

impl Dialogs {
    pub fn new(config: AppConfig, conn: Rc<Bus>) -> Rc<Self> {
        let dialogs = Rc::new(Self {
            config,
            conn,
            open: RefCell::default(),
            fingerprints: RefCell::default(),
            reasons: RefCell::default(),
        });
        // Most prompts are for the current user: find out now, off the GTK thread.
        if let Some(me) = users::by_uid(users::current_uid()) {
            dialogs.lookup_fingerprints(me.name, |_| {});
        }
        dialogs
    }

    /// Ask fprintd (once) whether `user` has fingerprints enrolled; `then` runs on the
    /// GTK thread with the answer.
    fn lookup_fingerprints(self: &Rc<Self>, user: String, then: impl FnOnce(bool) + 'static) {
        if let Some(&enrolled) = self.fingerprints.borrow().get(&user) {
            then(enrolled);
            return;
        }
        let dialogs = Rc::downgrade(self);
        let name = user.clone();
        tray::spawn_blocking(
            move || fprint::enrolled(&name),
            move |enrolled| {
                if let Some(dialogs) = dialogs.upgrade() {
                    dialogs.fingerprints.borrow_mut().insert(user, enrolled);
                }
                then(enrolled);
            },
        );
    }

    /// Show a prompt for `request`. `call` is answered once it is done.
    pub fn begin(self: &Rc<Self>, request: Request, call: Message) {
        let users = users::candidates(&request.identities);
        if users.is_empty() {
            eprintln!(
                "warning: nobody can authenticate for {}: {:?}",
                request.action_id, request.identities
            );
            self.reply(call.method_return());
            return;
        }
        if self.open.borrow().contains_key(&request.cookie) {
            eprintln!("warning: duplicate request {}", request.cookie);
            self.reply(call.method_return());
            return;
        }
        let cookie = request.cookie.clone();
        let user = users[0].name.clone();
        let subject_pid: Option<u32> = request
            .details
            .get(SUBJECT_PID_DETAIL)
            .and_then(|pid| pid.parse().ok());
        let announcement = subject_pid
            .and_then(|pid| self.reasons.borrow_mut().take(pid))
            .unwrap_or_default();
        let asking = Asking {
            app: announcement
                .app
                .or_else(|| subject_pid.and_then(App::of_process)),
            reason: announcement.reason,
            subject_pid,
        };
        let prompt = Prompt::show(
            Rc::downgrade(self),
            request,
            asking,
            call,
            users,
            &self.config,
        );
        self.open.borrow_mut().insert(cookie, prompt.clone());
        // Show the fingerprint or the password field right away; PAM may still change it.
        let weak = Rc::downgrade(&prompt);
        self.lookup_fingerprints(user, move |enrolled| {
            if let Some(prompt) = weak.upgrade() {
                prompt.expect_fingerprint(enrolled);
            }
        });
    }

    /// polkitd withdrew a request (the caller gave up).
    pub fn cancel(&self, cookie: &str) {
        let prompt = self.open.borrow().get(cookie).cloned();
        if let Some(prompt) = prompt {
            prompt.finish(Done::Cancelled);
        }
    }

    fn reply(&self, msg: Message) {
        if self.conn.send(msg).is_err() {
            eprintln!("warning: failed to reply to polkitd");
        }
        self.conn.channel().flush();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Done {
    Authenticated,
    Failed,
    Cancelled,
}

/// Who asks, as far as we can tell.
#[derive(Debug, Clone, Default)]
struct Asking {
    app: Option<App>,
    /// From `typk verify --reason`.
    reason: Option<String>,
    /// The process polkit is checking.
    subject_pid: Option<u32>,
}

struct Prompt {
    dialogs: Weak<Dialogs>,
    request: Request,
    asking: Asking,
    /// Darkens the screen while the prompt is open.
    dim: Option<overlay::Dim>,
    /// The pending `BeginAuthentication`, until it is answered.
    call: RefCell<Option<Message>>,
    users: Vec<User>,
    max_attempts: u32,
    helper: Option<PathBuf>,
    window: gtk::Window,
    chooser: Option<gtk::ComboBoxText>,
    /// The fingerprint glyph, shown while PAM waits for a finger.
    glyph: gtk::DrawingArea,
    /// Its strokes, as SVG path data.
    glyph_paths: Vec<String>,
    shown_at: Instant,
    /// The prompt label and the entry.
    password_row: gtk::Box,
    prompt_label: gtk::Label,
    entry: gtk::Entry,
    status: gtk::Label,
    spinner: gtk::Spinner,
    use_password: gtk::Button,
    ok: gtk::Button,
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    session: Option<helper::Session>,
    /// Events are tagged with the session they come from; older ones are ignored.
    generation: u32,
    /// PAM is waiting for what is typed.
    awaiting: bool,
    /// Typed before PAM asked (while the fingerprint reader was listening, say).
    stashed: Option<String>,
    /// PAM is listening for a finger.
    fingerprint: bool,
    /// The glyph is red until then (a finger was refused).
    flash_until: Option<Instant>,
    /// The redraw timer for the glyph is running.
    animating: bool,
    /// The password field has been revealed (by PAM or the button); it stays.
    password_shown: bool,
    attempts: u32,
    finished: bool,
}

impl Prompt {
    fn show(
        dialogs: Weak<Dialogs>,
        request: Request,
        asking: Asking,
        call: Message,
        users: Vec<User>,
        config: &AppConfig,
    ) -> Rc<Self> {
        // Before the window, so it stacks above.
        let dim = config
            .dim_screen
            .then(|| overlay::dim_screen(APP_NAME, DIM_ALPHA));
        let window = overlay::dialog_window(APP_NAME, "Authentication required");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin(18);
        content.set_size_request(WIDTH, -1);
        window.add(&content);

        // Icon, title and the reason.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let icon = header_icon(asking.app.as_ref(), &request.icon_name);
        icon.set_valign(gtk::Align::Start);
        header.pack_start(&icon, false, false, 0);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        // The asking program is the headline when we know it.
        let title = gtk::Label::new(None);
        let headline = asking
            .app
            .as_ref()
            .map_or("Authentication required", |app| app.name.as_str());
        title.set_markup(&format!(
            "<b><big>{}</big></b>",
            glib::markup_escape_text(headline)
        ));
        title.set_xalign(0.0);
        text.pack_start(&title, false, false, 0);
        let message = wrapped_label(&request.message);
        text.pack_start(&message, false, false, 0);
        // What `typk verify --reason` said.
        if let Some(reason) = &asking.reason {
            let reason = wrapped_label(reason);
            reason.set_attributes(Some(&italic()));
            text.pack_start(&reason, false, false, 0);
        }
        header.pack_start(&text, true, true, 0);
        content.pack_start(&header, false, false, 0);

        // Who.
        let who_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        who_row.pack_start(&dim_label("Authenticate as"), false, false, 0);
        let chooser = if users.len() > 1 {
            let chooser = gtk::ComboBoxText::new();
            for user in &users {
                chooser.append_text(&user.display());
            }
            chooser.set_active(Some(0));
            who_row.pack_start(&chooser, true, true, 0);
            Some(chooser)
        } else {
            let user = gtk::Label::new(Some(&users[0].display()));
            user.set_xalign(0.0);
            who_row.pack_start(&user, true, true, 0);
            None
        };
        content.pack_start(&who_row, false, false, 0);

        // A finger, while PAM listens for one.
        let glyph = gtk::DrawingArea::new();
        glyph.set_size_request(GLYPH_SIZE, GLYPH_SIZE);
        glyph.set_halign(gtk::Align::Center);
        glyph.set_margin_top(6);
        glyph.set_no_show_all(true);
        content.pack_start(&glyph, false, false, 0);

        // The password, once PAM asks for one or the user insists.
        let password_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        password_row.set_no_show_all(true);
        let prompt_label = dim_label("Password");
        password_row.pack_start(&prompt_label, false, false, 0);
        let entry = gtk::Entry::new();
        entry.set_visibility(false);
        entry.set_input_purpose(gtk::InputPurpose::Password);
        entry.set_activates_default(true);
        password_row.pack_start(&entry, true, true, 0);
        prompt_label.show();
        entry.show();
        content.pack_start(&password_row, false, false, 0);

        // What PAM says.
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spinner = gtk::Spinner::new();
        spinner.set_no_show_all(true);
        status_row.pack_start(&spinner, false, false, 0);
        let status = wrapped_label("");
        status_row.pack_start(&status, true, true, 0);
        content.pack_start(&status_row, false, false, 0);

        let details = details_grid(&request);
        if config.show_details {
            content.pack_start(&details, false, false, 0);
        } else {
            let expander = gtk::Expander::new(Some("Details"));
            expander.add(&details);
            content.pack_start(&expander, false, false, 0);
        }

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let use_password = gtk::Button::with_label("Use password");
        use_password.set_no_show_all(true);
        use_password.set_relief(gtk::ReliefStyle::None);
        buttons.pack_start(&use_password, false, false, 0);
        // Ending the asking program, for prompts that shouldn't have happened.
        let kill = asking.app.as_ref().map(|app| {
            let kill = gtk::Button::with_label(&format!("Kill {}", app.name));
            kill.set_relief(gtk::ReliefStyle::None);
            kill.style_context().add_class("dim-label");
            buttons.pack_start(&kill, false, false, 0);
            kill
        });
        let ok = gtk::Button::with_label("Authenticate");
        ok.set_can_default(true);
        ok.set_no_show_all(true);
        ok.style_context().add_class("suggested-action");
        buttons.pack_end(&ok, false, false, 0);
        let cancel = gtk::Button::with_label("Cancel");
        buttons.pack_end(&cancel, false, false, 0);
        content.pack_start(&buttons, false, false, 0);

        let prompt = Rc::new(Self {
            dialogs,
            request,
            asking,
            dim,
            call: RefCell::new(Some(call)),
            users,
            max_attempts: config.max_attempts,
            helper: config.helper.clone(),
            window: window.clone(),
            chooser: chooser.clone(),
            glyph: glyph.clone(),
            glyph_paths: svgpath::path_data(FINGERPRINT_SVG),
            shown_at: Instant::now(),
            password_row,
            prompt_label,
            entry: entry.clone(),
            status,
            spinner,
            use_password: use_password.clone(),
            ok: ok.clone(),
            state: RefCell::default(),
        });

        if prompt.dim.is_some() {
            // The window manager restacks on map and focus changes; stay above the dimming.
            let weak = Rc::downgrade(&prompt);
            window.connect_map_event(move |window, _| {
                if let Some(dim) = weak.upgrade().and_then(|p| p.dim.as_ref().cloned()) {
                    dim.stay_below(window);
                }
                glib::Propagation::Proceed
            });
            let weak = Rc::downgrade(&prompt);
            window.connect_focus_in_event(move |window, _| {
                if let Some(dim) = weak.upgrade().and_then(|p| p.dim.as_ref().cloned()) {
                    dim.stay_below(window);
                }
                glib::Propagation::Proceed
            });
        }
        let weak = Rc::downgrade(&prompt);
        glyph.connect_draw(move |area, cr| {
            if let Some(prompt) = weak.upgrade() {
                prompt.paint_glyph(area, cr);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&prompt);
        ok.connect_clicked(move |_| {
            if let Some(prompt) = weak.upgrade() {
                prompt.submit();
            }
        });
        let weak = Rc::downgrade(&prompt);
        use_password.connect_clicked(move |_| {
            if let Some(prompt) = weak.upgrade() {
                prompt.show_password();
            }
        });
        if let Some(kill) = &kill {
            let weak = Rc::downgrade(&prompt);
            kill.connect_clicked(move |_| {
                if let Some(prompt) = weak.upgrade() {
                    prompt.kill_app();
                }
            });
        }
        let weak = Rc::downgrade(&prompt);
        cancel.connect_clicked(move |_| {
            if let Some(prompt) = weak.upgrade() {
                prompt.finish(Done::Cancelled);
            }
        });
        let weak = Rc::downgrade(&prompt);
        window.connect_delete_event(move |_, _| {
            if let Some(prompt) = weak.upgrade() {
                prompt.finish(Done::Cancelled);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&prompt);
        window.connect_key_press_event(move |_, key| {
            if key.keyval() != gdk::keys::constants::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(prompt) = weak.upgrade() {
                prompt.finish(Done::Cancelled);
            }
            glib::Propagation::Stop
        });
        if let Some(chooser) = &chooser {
            let weak = Rc::downgrade(&prompt);
            chooser.connect_changed(move |_| {
                if let Some(prompt) = weak.upgrade() {
                    prompt.restart();
                }
            });
        }

        window.show_all();
        // Above the dimming windows.
        window.present();
        window.set_default(Some(&ok));
        prompt.start_session();
        prompt
    }

    /// End the asking program (and the process polkit is checking, if different), then
    /// refuse.
    fn kill_app(&self) {
        let mut pids: Vec<u32> = self.asking.app.iter().map(|app| app.pid).collect();
        if let Some(pid) = self.asking.subject_pid {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        for pid in pids {
            // SAFETY: kill has no memory preconditions; a wrong pid fails harmlessly.
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
                eprintln!(
                    "warning: failed to kill {pid}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        self.finish(Done::Cancelled);
    }

    fn selected_user(&self) -> &User {
        let index = self
            .chooser
            .as_ref()
            .and_then(|chooser| chooser.active())
            .unwrap_or(0) as usize;
        self.users.get(index).unwrap_or(&self.users[0])
    }

    /// Start (or start over with) a helper for the selected user.
    fn start_session(self: &Rc<Self>) {
        let user = self.selected_user().name.clone();
        let (events, receiver) = tray::channel();
        let generation = {
            let mut state = self.state.borrow_mut();
            // Ends the previous attempt, if any.
            state.session = None;
            state.awaiting = false;
            state.fingerprint = false;
            state.generation += 1;
            state.generation
        };
        match helper::Session::start(self.helper.as_deref(), &user, &self.request.cookie, events) {
            Ok(session) => self.state.borrow_mut().session = Some(session),
            Err(err) => {
                eprintln!("error: {err:#}");
                self.set_status(&format!("{err:#}"), true);
                self.entry.set_sensitive(false);
                self.ok.set_sensitive(false);
                return;
            }
        }
        let weak = Rc::downgrade(self);
        tray::drain_actions(receiver, move |event| {
            let Some(prompt) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            let current = {
                let state = prompt.state.borrow();
                !state.finished && state.generation == generation
            };
            if !current {
                return ControlFlow::Break;
            }
            prompt.on_event(event);
            ControlFlow::Continue
        });
    }

    /// Another user was picked.
    fn restart(self: &Rc<Self>) {
        self.set_status("", false);
        self.set_busy(false);
        self.entry.set_text("");
        self.state.borrow_mut().stashed = None;
        self.start_session();
    }

    fn on_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Prompt { text, secret } => {
                self.state.borrow_mut().fingerprint = false;
                self.show_password();
                self.prompt_label.set_text(&prompt_text(&text));
                self.entry.set_visibility(!secret);
                let stashed = self.state.borrow_mut().stashed.take();
                match stashed {
                    Some(answer) => self.send(&answer),
                    None => {
                        self.state.borrow_mut().awaiting = true;
                        self.set_busy(false);
                        self.set_status("", false);
                        self.entry.grab_focus();
                    }
                }
            }
            Event::Info(text) => {
                if is_fingerprint_hint(&text) {
                    self.show_fingerprint();
                } else {
                    self.set_status(&text, false);
                }
            }
            Event::Error(text) => {
                let mut state = self.state.borrow_mut();
                if state.fingerprint {
                    state.flash_until = Some(Instant::now() + FLASH);
                }
                drop(state);
                self.set_status(&text, true);
            }
            Event::Success => self.finish(Done::Authenticated),
            Event::Failure => {
                let attempts = {
                    let mut state = self.state.borrow_mut();
                    state.attempts += 1;
                    state.attempts
                };
                if attempts >= self.max_attempts {
                    self.finish(Done::Failed);
                    return;
                }
                let left = self.max_attempts - attempts;
                let noun = if left == 1 { "attempt" } else { "attempts" };
                self.set_status(&format!("Authentication failed. {left} {noun} left."), true);
                self.set_busy(false);
                self.entry.set_text("");
                self.start_session();
            }
        }
    }

    /// What fprintd says before PAM has said anything: pick the initial layout, unless
    /// PAM already did.
    fn expect_fingerprint(self: &Rc<Self>, enrolled: bool) {
        if self.state.borrow().password_shown || self.glyph.is_visible() {
            return;
        }
        if enrolled {
            self.show_fingerprint();
        } else {
            self.show_password();
        }
    }

    /// PAM is listening for a finger: the glyph instead of its instructions, and the
    /// password behind a button (unless it is already out).
    fn show_fingerprint(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        state.fingerprint = true;
        if state.password_shown {
            return;
        }
        let animate = !state.animating;
        state.animating = true;
        drop(state);
        self.glyph.show();
        self.use_password.show();
        self.set_status("", false);
        if animate {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local(FRAME, move || {
                let Some(prompt) = weak.upgrade() else {
                    return ControlFlow::Break;
                };
                if prompt.state.borrow().finished || !prompt.glyph.is_visible() {
                    prompt.state.borrow_mut().animating = false;
                    return ControlFlow::Break;
                }
                prompt.glyph.queue_draw();
                ControlFlow::Continue
            });
        }
    }

    /// The glyph, breathing; red while a refused finger is flashed.
    fn paint_glyph(&self, area: &gtk::DrawingArea, cr: &cairo::Context) {
        let now = Instant::now();
        let flashing = self
            .state
            .borrow()
            .flash_until
            .is_some_and(|until| now < until);
        let color = area.style_context().color(gtk::StateFlags::NORMAL);
        let (rgb, alpha, pulse) = if flashing {
            (hex_rgb(ATTENTION_COLOR), 1.0, 1.0)
        } else {
            let phase = (now - self.shown_at).as_secs_f64() / PULSE.as_secs_f64() * TAU;
            let pulse = 0.5 - 0.5 * phase.cos();
            (
                (color.red(), color.green(), color.blue()),
                color.alpha() * (0.45 + 0.55 * pulse),
                pulse,
            )
        };
        paint_fingerprint(
            cr,
            &self.glyph_paths,
            f64::from(area.allocated_width()),
            f64::from(area.allocated_height()),
            (rgb.0, rgb.1, rgb.2, alpha),
            pulse,
        );
    }

    /// Reveal the password field (and keep it).
    fn show_password(&self) {
        if self.state.borrow().password_shown {
            return;
        }
        self.state.borrow_mut().password_shown = true;
        self.glyph.hide();
        self.use_password.hide();
        self.password_row.show();
        self.ok.show();
        self.window.set_default(Some(&self.ok));
        self.entry.grab_focus();
    }

    /// Enter or the Authenticate button.
    fn submit(&self) {
        let answer = self.entry.text().to_string();
        self.entry.set_text("");
        let (awaiting, fingerprint) = {
            let state = self.state.borrow();
            (state.awaiting, state.fingerprint)
        };
        if awaiting {
            self.send(&answer);
        } else {
            // PAM hasn't asked yet; it will once the fingerprint reader gives up.
            self.state.borrow_mut().stashed = Some(answer);
            let text = if fingerprint {
                "Waiting for the fingerprint reader…"
            } else {
                "Checking…"
            };
            self.set_status(text, false);
            self.set_busy(true);
        }
    }

    fn send(&self, answer: &str) {
        let mut state = self.state.borrow_mut();
        state.awaiting = false;
        if let Some(session) = state.session.as_mut() {
            if let Err(err) = session.respond(answer) {
                eprintln!("warning: {err:#}");
            }
        }
        drop(state);
        self.set_status("Checking…", false);
        self.set_busy(true);
    }

    fn set_status(&self, text: &str, error: bool) {
        if error && !text.is_empty() {
            self.status.set_markup(&format!(
                "<span foreground=\"{ATTENTION_COLOR}\">{}</span>",
                glib::markup_escape_text(text)
            ));
        } else {
            self.status.set_text(text);
        }
    }

    fn set_busy(&self, busy: bool) {
        self.spinner.set_visible(busy);
        if busy {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    /// Answer polkitd and close. Safe to call more than once.
    fn finish(&self, done: Done) {
        {
            let mut state = self.state.borrow_mut();
            if state.finished {
                return;
            }
            state.finished = true;
            state.session = None;
        }
        println!(
            "{} for {} as {}",
            match done {
                Done::Authenticated => "Authenticated",
                Done::Failed => "Failed",
                Done::Cancelled => "Cancelled",
            },
            self.request.action_id,
            self.selected_user().name
        );
        let Some(dialogs) = self.dialogs.upgrade() else {
            return;
        };
        if let Some(call) = self.call.borrow_mut().take() {
            let reply = if done == Done::Cancelled {
                let name = ErrorName::new(ERROR_CANCELLED).expect("valid error name");
                let text = CString::new("Authentication was cancelled").expect("no NUL");
                call.error(&name, &text)
            } else {
                // Without a response from the helper, polkitd treats this as not authorized.
                call.method_return()
            };
            dialogs.reply(reply);
        }
        dialogs.open.borrow_mut().remove(&self.request.cookie);
        self.window.close();
        if let Some(dim) = &self.dim {
            dim.close();
        }
    }
}

/// pam_fprintd's instructions ("Place your finger on the fingerprint reader", "Swipe your
/// finger across ...", "Place your right index finger on ...").
fn is_fingerprint_hint(text: &str) -> bool {
    text.to_lowercase().contains("finger")
}

/// Draw the fingerprint strokes centered in `width` x `height`, grown a little with
/// `pulse` (0 to 1).
fn paint_fingerprint(
    cr: &cairo::Context,
    paths: &[String],
    width: f64,
    height: f64,
    rgba: (f64, f64, f64, f64),
    pulse: f64,
) {
    let scale = width.min(height) / ICON_GRID * (0.94 + 0.06 * pulse);
    cr.translate(width / 2.0, height / 2.0);
    cr.scale(scale, scale);
    cr.translate(-ICON_GRID / 2.0, -ICON_GRID / 2.0);
    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    cr.set_line_width(ICON_STROKE);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    for path in paths {
        cr.new_path();
        match svgpath::trace(cr, path) {
            Ok(()) => {
                let _ = cr.stroke();
            }
            Err(err) => eprintln!("warning: bad icon path: {err}"),
        }
    }
}

/// `#rrggbb` as cairo components.
fn hex_rgb(hex: &str) -> (f64, f64, f64) {
    let channel = |index: usize| {
        u8::from_str_radix(&hex[1 + 2 * index..3 + 2 * index], 16)
            .map_or(0.0, |value| f64::from(value) / 255.0)
    };
    (channel(0), channel(1), channel(2))
}

/// The asking app's icon when its desktop entry has one, else the theme icon polkitd
/// named, else a lock.
fn header_icon(app: Option<&App>, requested: &str) -> gtk::Image {
    if let Some(icon) = app.and_then(App::icon) {
        if Path::new(&icon).is_absolute() {
            if let Ok(pixbuf) = Pixbuf::from_file_at_scale(&icon, ICON_PX, ICON_PX, true) {
                return gtk::Image::from_pixbuf(Some(&pixbuf));
            }
        } else if theme_has(&icon) {
            let image = gtk::Image::from_icon_name(Some(&icon), gtk::IconSize::Dialog);
            image.set_pixel_size(ICON_PX);
            return image;
        }
    }
    gtk::Image::from_icon_name(Some(icon_name(requested)), gtk::IconSize::Dialog)
}

fn theme_has(name: &str) -> bool {
    gtk::IconTheme::default().is_some_and(|theme| theme.has_icon(name))
}

/// The theme icon polkitd named, or a lock.
fn icon_name(requested: &str) -> &str {
    let known = !requested.is_empty() && theme_has(requested);
    if known {
        requested
    } else {
        DEFAULT_ICON
    }
}

/// `Password: ` -> `Password`.
fn prompt_text(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches(':').trim_end();
    if trimmed.is_empty() {
        "Password".to_string()
    } else {
        trimmed.to_string()
    }
}

fn italic() -> pango::AttrList {
    let attrs = pango::AttrList::new();
    attrs.insert(pango::AttrInt::new_style(pango::Style::Italic));
    attrs
}

fn wrapped_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_line_wrap_mode(pango::WrapMode::WordChar);
    label.set_max_width_chars(WRAP_CHARS);
    label
}

fn dim_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.style_context().add_class("dim-label");
    label
}

/// The action id and polkit's details, as `key: value` rows.
fn details_grid(request: &Request) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_row_spacing(4);
    grid.set_column_spacing(10);
    grid.set_margin_top(6);
    let mut row = 0;
    let mut add = |key: &str, value: &str| {
        grid.attach(&dim_label(key), 0, row, 1, 1);
        let value = wrapped_label(value);
        value.set_selectable(true);
        grid.attach(&value, 1, row, 1, 1);
        row += 1;
    };
    add("Action", &request.action_id);
    for (key, value) in &request.details {
        if !HIDDEN_DETAILS.contains(&key.as_str()) {
            add(key, value);
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_text_strips_colon() {
        assert_eq!(prompt_text("Password: "), "Password");
        assert_eq!(prompt_text("Enter PIN:"), "Enter PIN");
        assert_eq!(prompt_text("  "), "Password");
    }

    /// Paints the glyph into an image. Set `TYPK_GLYPH_PPM=<path>` to look at it.
    #[test]
    fn paints_fingerprint() {
        let size = 144;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().unwrap();
            let paths = svgpath::path_data(FINGERPRINT_SVG);
            assert_eq!(paths.len(), 9, "Lucide's fingerprint has nine strokes");
            paint_fingerprint(
                &cr,
                &paths,
                f64::from(size),
                f64::from(size),
                (0.1, 0.1, 0.1, 1.0),
                1.0,
            );
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        let dark = |x: usize, y: usize| data[y * stride + x * 4] < 128;
        assert!(dark(size as usize / 2, 10), "outer ridge at the top");
        assert!(
            !dark(size as usize / 2, size as usize - 4),
            "open at the bottom"
        );
        if let Ok(path) = std::env::var("TYPK_GLYPH_PPM") {
            let mut ppm = format!("P6 {size} {size} 255\n").into_bytes();
            for y in 0..size as usize {
                for x in 0..size as usize {
                    let px = &data[y * stride + x * 4..][..4];
                    ppm.extend([px[2], px[1], px[0]]);
                }
            }
            std::fs::write(path, ppm).unwrap();
        }
    }

    #[test]
    fn recognizes_fingerprint_hints() {
        assert!(is_fingerprint_hint(
            "Place your right index finger on the fingerprint reader"
        ));
        assert!(is_fingerprint_hint("Swipe your finger across the reader"));
        assert!(!is_fingerprint_hint("Welcome"));
    }
}
