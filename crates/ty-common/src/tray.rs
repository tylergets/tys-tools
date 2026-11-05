//! Tray applet plumbing shared by every tool: an AppIndicator (StatusNotifierItem)
//! described by a plain [`TraySpec`], bundled SVG icons, and running blocking work off the
//! GTK thread.
//!
//! Menu clicks are delivered as values of the app's action type `A` on a [`channel`]; the
//! app handles them on the GTK thread (see [`drain_actions`]).
//!
//! Nothing here polls: the GTK thread sleeps until a click, a message or a [`Waker`] wakes it.

use std::cell::RefCell;
use std::env;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;

pub use crate::tray_sni::keep_registered;
pub use crate::wake::{Waker, Wakeups};
pub use async_channel::{Receiver, Sender};

/// `APP_INDICATOR_CATEGORY_APPLICATION_STATUS`.
const CATEGORY_APPLICATION_STATUS: u32 = 0;
/// `APP_INDICATOR_STATUS_PASSIVE`: tray hosts hide the icon.
const STATUS_PASSIVE: u32 = 0;
/// `APP_INDICATOR_STATUS_ACTIVE`.
const STATUS_ACTIVE: u32 = 1;

/// An SVG icon written to the user's icon theme on startup.
#[derive(Debug, Clone)]
pub struct BundledIcon {
    pub name: String,
    pub svg: String,
}

/// The icons an applet switches between. `attention` and `busy` fall back to `idle`.
/// Usually built with [`crate::icons::Tool::tray_icons`].
#[derive(Debug, Clone)]
pub struct Icons {
    pub idle: BundledIcon,
    pub attention: Option<BundledIcon>,
    pub busy: Option<BundledIcon>,
    /// Theme icon used if the bundled ones can't be written.
    pub fallback: String,
}

/// Resolved icon names, after installation.
struct IconNames {
    idle: String,
    attention: String,
    busy: String,
}

impl IconNames {
    fn name(&self, state: IconState) -> &str {
        match state {
            IconState::Normal => &self.idle,
            IconState::Attention => &self.attention,
            IconState::Busy => &self.busy,
        }
    }
}

/// Which icon is shown: neutral, attention (e.g. red while recording) or busy (yellow, working
/// in the background).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconState {
    #[default]
    Normal,
    Attention,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry<A> {
    /// Insensitive informational line.
    Label(String),
    Action {
        label: String,
        enabled: bool,
        action: A,
    },
    Submenu {
        label: String,
        entries: Vec<MenuEntry<A>>,
    },
    Separator,
}

/// Everything the tray shows. Rebuilt by the app whenever its state changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraySpec<A> {
    pub icon: IconState,
    /// Window title shown by tray hosts that support it.
    pub title: String,
    pub menu: Vec<MenuEntry<A>>,
    /// Sent on a secondary activation (middle click). AppIndicator always opens the menu on
    /// a left click.
    pub click_action: Option<A>,
}

/// A built menu item and, for submenus, its children (entry for entry).
struct Slot {
    widget: gtk::MenuItem,
    children: Vec<Slot>,
}

/// The tray icon and its menu. Updates are held while the menu is open, because tray hosts
/// (e.g. eww) don't resize an open menu when its items change; they apply when it closes.
pub struct Applet<A: Clone + Send + 'static> {
    inner: Rc<RefCell<Inner<A>>>,
}

/// How long an open menu holds back updates, in case the host never says it closed.
const MAX_OPEN: Duration = Duration::from_secs(60);

impl<A: Clone + Send + PartialEq + 'static> Applet<A> {
    /// Create the indicator and keep it registered with the tray host. GTK must already be
    /// initialized, and this must run on the GTK thread.
    pub fn new(id: &str, icons: &Icons, spec: TraySpec<A>, actions: Sender<A>) -> Result<Self> {
        let inner = Rc::new(RefCell::new(Inner::new(id, icons, spec, actions)?));
        watch_open(&inner);
        Ok(Self { inner })
    }

    /// The latest spec, shown or waiting for the menu to close.
    pub fn spec(&self) -> TraySpec<A> {
        let inner = self.inner.borrow();
        inner.pending.clone().unwrap_or_else(|| inner.spec.clone())
    }

    /// Show or hide the icon. Hidden icons are marked passive, which tray hosts don't show.
    pub fn set_visible(&mut self, visible: bool) {
        let status = if visible { STATUS_ACTIVE } else { STATUS_PASSIVE };
        unsafe {
            libappindicator::app_indicator_set_status(self.inner.borrow().indicator, status);
        }
    }

    pub fn update(&mut self, spec: TraySpec<A>) {
        let mut inner = self.inner.borrow_mut();
        if inner.open {
            inner.pending = Some(spec);
        } else {
            inner.update(spec);
        }
    }
}

/// Follow the host's `opened` / `closed` events on the menu's root item.
fn watch_open<A: Clone + Send + PartialEq + 'static>(inner: &Rc<RefCell<Inner<A>>>) {
    let indicator: glib::Object = unsafe {
        glib::translate::from_glib_none(inner.borrow().indicator as *mut glib::gobject_ffi::GObject)
    };
    let Some(root) = menu_root(&indicator) else {
        return;
    };
    let weak = Rc::downgrade(inner);
    root.connect_local("event", false, move |args| {
        let name = args.get(1).and_then(|value| value.get::<String>().ok());
        if let Some(inner) = weak.upgrade() {
            match name.as_deref() {
                Some("opened") => set_open(&inner, true),
                Some("closed") => set_open(&inner, false),
                _ => {}
            }
        }
        // Not handled: let libdbusmenu carry on as usual.
        Some(false.to_value())
    });
}

/// libappindicator's dbusmenu server, and the root item the host's events arrive on.
fn menu_root(indicator: &glib::Object) -> Option<glib::Object> {
    indicator.find_property("dbus-menu-server")?;
    let server = indicator.property::<Option<glib::Object>>("dbus-menu-server")?;
    server.find_property("root-node")?;
    server.property::<Option<glib::Object>>("root-node")
}

fn set_open<A: Clone + Send + PartialEq + 'static>(inner: &Rc<RefCell<Inner<A>>>, open: bool) {
    let Ok(mut state) = inner.try_borrow_mut() else {
        return;
    };
    state.open = open;
    state.opened += 1;
    if open {
        let (weak, opened) = (Rc::downgrade(inner), state.opened);
        glib::timeout_add_local_once(MAX_OPEN, move || {
            if let Some(inner) = weak.upgrade() {
                if inner.borrow().opened == opened {
                    set_open(&inner, false);
                }
            }
        });
    } else if let Some(spec) = state.pending.take() {
        state.update(spec);
    }
}

struct Inner<A: Clone + Send + 'static> {
    indicator: *mut libappindicator::_AppIndicator,
    actions: Sender<A>,
    /// The spec shown.
    spec: TraySpec<A>,
    menu: gtk::Menu,
    /// Widgets for `spec.menu`, index for index.
    slots: Vec<Slot>,
    icons: IconNames,
    /// The host has the menu open.
    open: bool,
    /// Counts opens and closes, so a stale [`MAX_OPEN`] timer does nothing.
    opened: u64,
    /// The latest update while open.
    pending: Option<TraySpec<A>>,
}

impl<A: Clone + Send + PartialEq + 'static> Inner<A> {
    fn new(id: &str, icons: &Icons, spec: TraySpec<A>, actions: Sender<A>) -> Result<Self> {
        if env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("warning: DBus session is not set; appindicator may not be visible");
        }

        let attention = icons.attention.as_ref().unwrap_or(&icons.idle);
        let busy = icons.busy.as_ref().unwrap_or(&icons.idle);
        let (theme_path, names) = match install_icons(&[&icons.idle, attention, busy]) {
            Some(dir) => (
                dir.to_string_lossy().into_owned(),
                IconNames {
                    idle: icons.idle.name.clone(),
                    attention: attention.name.clone(),
                    busy: busy.name.clone(),
                },
            ),
            None => (
                String::new(),
                IconNames {
                    idle: icons.fallback.clone(),
                    attention: icons.fallback.clone(),
                    busy: icons.fallback.clone(),
                },
            ),
        };

        let id_c = CString::new(id).context("invalid applet id")?;
        let icon_c = CString::new(names.idle.as_str())?;
        let theme_path_c = CString::new(theme_path)?;
        // The indicator lives for the rest of the process, so it is never unreferenced.
        let indicator = unsafe {
            let indicator = libappindicator::app_indicator_new_with_path(
                id_c.as_ptr(),
                icon_c.as_ptr(),
                CATEGORY_APPLICATION_STATUS,
                theme_path_c.as_ptr(),
            );
            libappindicator::app_indicator_set_status(indicator, STATUS_ACTIVE);
            indicator
        };

        let menu = gtk::Menu::new();
        let mut applet = Self {
            indicator,
            actions,
            spec: spec.clone(),
            menu,
            slots: Vec::new(),
            icons: names,
            open: false,
            opened: 0,
            pending: None,
        };
        applet.rebuild_menu();
        unsafe {
            libappindicator::app_indicator_set_menu(applet.indicator, applet.menu.as_ptr());
        }
        keep_registered(applet.indicator);
        applet.apply_icon_title_and_click_action();
        Ok(applet)
    }

    fn update(&mut self, spec: TraySpec<A>) {
        if spec == self.spec {
            return;
        }
        let same_shape = same_shape(&spec.menu, &self.spec.menu);
        self.spec = spec;

        // Update in place when only labels/sensitivity changed, so the menu doesn't flicker.
        if same_shape {
            update_in_place(&self.spec.menu, &self.slots);
        } else {
            self.rebuild_menu();
        }
        self.apply_icon_title_and_click_action();
    }

    fn rebuild_menu(&mut self) {
        for child in self.menu.children() {
            self.menu.remove(&child);
        }
        self.slots = build_entries(&self.menu, &self.spec.menu, &self.actions);
        self.menu.show_all();
    }

    fn apply_icon_title_and_click_action(&self) {
        let name = self.icons.name(self.spec.icon);
        if let Ok(name) = CString::new(name) {
            unsafe {
                libappindicator::app_indicator_set_icon_full(
                    self.indicator,
                    name.as_ptr(),
                    name.as_ptr(),
                );
            }
        }
        if let Ok(title) = CString::new(self.spec.title.as_str()) {
            unsafe {
                libappindicator::app_indicator_set_title(self.indicator, title.as_ptr());
            }
        }

        let target = self.spec.click_action.as_ref().and_then(|click| {
            self.spec
                .menu
                .iter()
                .zip(&self.slots)
                .find(|(entry, _)| {
                    matches!(entry, MenuEntry::Action { action, .. } if action == click)
                })
                .map(|(_, slot)| slot.widget.upcast_ref::<gtk::Widget>().as_ptr())
        });
        unsafe {
            libappindicator::app_indicator_set_secondary_activate_target(
                self.indicator,
                target.unwrap_or(std::ptr::null_mut()),
            );
        }
    }
}

fn build_entries<A: Clone + Send + 'static>(
    menu: &gtk::Menu,
    entries: &[MenuEntry<A>],
    actions: &Sender<A>,
) -> Vec<Slot> {
    entries
        .iter()
        .map(|entry| {
            let mut children = Vec::new();
            let widget = match entry {
                MenuEntry::Label(label) => {
                    let item = gtk::MenuItem::with_label(label);
                    item.set_sensitive(false);
                    item
                }
                MenuEntry::Action {
                    label,
                    enabled,
                    action,
                } => {
                    let item = gtk::MenuItem::with_label(label);
                    item.set_sensitive(*enabled);
                    let actions = actions.clone();
                    let action = action.clone();
                    item.connect_activate(move |_| {
                        let _ = actions.try_send(action.clone());
                    });
                    item
                }
                MenuEntry::Submenu { label, entries } => {
                    let item = gtk::MenuItem::with_label(label);
                    let submenu = gtk::Menu::new();
                    children = build_entries(&submenu, entries, actions);
                    submenu.show_all();
                    item.set_submenu(Some(&submenu));
                    item
                }
                MenuEntry::Separator => gtk::SeparatorMenuItem::new().upcast(),
            };
            menu.append(&widget);
            Slot { widget, children }
        })
        .collect()
}

fn update_in_place<A>(entries: &[MenuEntry<A>], slots: &[Slot]) {
    for (entry, slot) in entries.iter().zip(slots) {
        match entry {
            MenuEntry::Label(label) => slot.widget.set_label(label),
            MenuEntry::Action { label, enabled, .. } => {
                slot.widget.set_label(label);
                slot.widget.set_sensitive(*enabled);
            }
            MenuEntry::Submenu { label, entries } => {
                slot.widget.set_label(label);
                update_in_place(entries, &slot.children);
            }
            MenuEntry::Separator => {}
        }
    }
}

/// Whether `new` can reuse the widgets built for `old`: same kinds in the same places, and
/// the same actions (the activate handlers capture them).
fn same_shape<A: PartialEq>(new: &[MenuEntry<A>], old: &[MenuEntry<A>]) -> bool {
    new.len() == old.len()
        && new.iter().zip(old).all(|pair| match pair {
            (MenuEntry::Label(_), MenuEntry::Label(_)) => true,
            (MenuEntry::Separator, MenuEntry::Separator) => true,
            (MenuEntry::Action { action: a, .. }, MenuEntry::Action { action: b, .. }) => a == b,
            (MenuEntry::Submenu { entries: a, .. }, MenuEntry::Submenu { entries: b, .. }) => {
                same_shape(a, b)
            }
            _ => false,
        })
}

/// Write `icons` into the user's hicolor theme and return that directory, which is also
/// passed to the tray host as the indicator's icon theme path so hosts that cached the theme
/// before the icons existed still find them.
pub fn install_icons(icons: &[&BundledIcon]) -> Option<PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))?;
    let dir = data_home.join("icons/hicolor/scalable/apps");

    if let Err(err) = fs::create_dir_all(&dir) {
        eprintln!(
            "warning: failed to create icon directory {}: {err}",
            dir.display()
        );
        return None;
    }
    for icon in icons {
        let path = dir.join(format!("{}.svg", icon.name));
        if let Err(err) = fs::write(&path, &icon.svg) {
            eprintln!(
                "warning: failed to write tray icon {}: {err}",
                path.display()
            );
            return None;
        }
    }
    Some(dir)
}

/// An unbounded channel whose receiver can be handed to [`drain_actions`]. The sender works
/// from any thread (`send_blocking` never blocks, as the channel is unbounded).
pub fn channel<A>() -> (Sender<A>, Receiver<A>) {
    async_channel::unbounded()
}

/// Handle messages from `actions` on the GTK thread as they arrive. Stops when `handle`
/// returns [`ControlFlow::Break`] or every sender is dropped. Must be called on the GTK thread.
///
/// Note: GTK calls such as `gtk::Clipboard::wait_for_*` and `store()` run a nested main loop,
/// so this (and timers) can run inside them. Don't hold a `RefCell` borrow shared with
/// those handlers across such calls.
pub fn drain_actions<A: 'static>(
    actions: Receiver<A>,
    mut handle: impl FnMut(A) -> ControlFlow + 'static,
) {
    glib::MainContext::default().spawn_local(async move {
        while let Ok(action) = actions.recv().await {
            if handle(action) == ControlFlow::Break {
                break;
            }
        }
    });
}

/// Run `handle` on the GTK thread after each [`Waker::wake`] (several wakes in a row may run
/// it once). Stops like [`drain_actions`].
pub fn on_wake(wakeups: Wakeups, mut handle: impl FnMut() -> ControlFlow + 'static) {
    drain_actions(wakeups.0, move |()| handle());
}

/// A tray running on its own GTK thread (for apps whose main thread is busy, e.g. with a key
/// listener). Keep it alive for as long as the tray should run.
pub struct TrayThread {
    _thread: JoinHandle<()>,
}

/// How long [`spawn_gtk_thread`] waits for `setup` before carrying on without it.
const TRAY_STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

/// Start GTK on a new thread, run `setup` there (build the [`Applet`], timers, ...), then run
/// the GTK main loop. Whatever `setup` returns stays alive for the life of the loop.
///
/// Returns `None` (after logging) outside a graphical session or if setup fails. If setup takes longer
/// than a few seconds the thread keeps going and a handle is returned anyway.
pub fn spawn_gtk_thread<K, F>(setup: F) -> Option<TrayThread>
where
    K: 'static,
    F: FnOnce() -> Result<K> + Send + 'static,
{
    if crate::session::current().is_none() {
        eprintln!("warning: neither WAYLAND_DISPLAY nor DISPLAY is set; cannot create tray icon");
        return None;
    }

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread = thread::spawn(move || {
        let kept = gtk::init()
            .context("failed to initialize GTK")
            .and_then(|()| setup());
        match kept {
            Ok(_kept) => {
                let _ = ready_tx.send(Ok(()));
                gtk::main();
            }
            Err(err) => {
                let _ = ready_tx.send(Err(format!("{err:#}")));
            }
        }
    });

    match ready_rx.recv_timeout(TRAY_STARTUP_TIMEOUT) {
        Ok(Ok(())) => Some(TrayThread { _thread: thread }),
        Ok(Err(err)) => {
            eprintln!("failed to start tray icon: {err}");
            let _ = thread.join();
            None
        }
        Err(_) => {
            eprintln!("warning: tray startup timed out; keeping tray thread running");
            Some(TrayThread { _thread: thread })
        }
    }
}

/// Run `work` on a background thread and deliver its result to `done` on the GTK thread.
pub fn spawn_blocking<T, W, D>(work: W, done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::MainContext::default().spawn_local(async move {
        if let Ok(value) = rx.recv().await {
            done(value);
        }
    });
}

/// Show a notification with action buttons without blocking the GTK thread. `on_action` runs
/// on the GTK thread with the chosen action key (`"default"` for a body click).
pub fn notify_with_actions(
    app: &str,
    summary: &str,
    body: &str,
    timeout: Duration,
    actions: &[(&str, &str)],
    on_action: impl FnOnce(String) + 'static,
) {
    let (app, summary, body) = (app.to_string(), summary.to_string(), body.to_string());
    let actions: Vec<(String, String)> = actions
        .iter()
        .map(|(key, label)| (key.to_string(), label.to_string()))
        .collect();
    spawn_blocking(
        move || {
            let actions: Vec<(&str, &str)> = actions
                .iter()
                .map(|(k, l)| (k.as_str(), l.as_str()))
                .collect();
            crate::notify::notify_with_actions_blocking(&app, &summary, &body, timeout, &actions)
        },
        move |key| {
            if let Some(key) = key {
                on_action(key);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{same_shape, MenuEntry};

    fn label(text: &str) -> MenuEntry<u8> {
        MenuEntry::Label(text.into())
    }

    fn action(action: u8) -> MenuEntry<u8> {
        MenuEntry::Action {
            label: "x".into(),
            enabled: true,
            action,
        }
    }

    #[test]
    fn menu_shapes_match_by_kind_and_action() {
        assert!(same_shape(&[label("a")], &[label("b")]));
        assert!(same_shape(&[action(1)], &[action(1)]));
        assert!(!same_shape(&[action(1)], &[action(2)]));
        assert!(!same_shape(&[label("a")], &[MenuEntry::Separator]));
        assert!(!same_shape(&[label("a")], &[label("a"), label("b")]));
    }

    #[test]
    fn submenus_compare_their_entries() {
        let sub = |entries: Vec<MenuEntry<u8>>| MenuEntry::Submenu {
            label: "Model".into(),
            entries,
        };
        assert!(same_shape(
            &[sub(vec![action(1), label("a")])],
            &[sub(vec![action(1), label("b")])]
        ));
        assert!(!same_shape(
            &[sub(vec![action(1)])],
            &[sub(vec![action(2)])]
        ));
    }
}
