# ty-common

Shared code for the ty* tools (tyboard, tycapture, tykey, tyshot, tyspeak, tylight,
tygesture, typort, tyci). It is a library crate, not a tool: nothing here is installed on
its own. If two tools need the same thing, it belongs here.

| Module   | What it does |
|----------|--------------|
| `config` | Finds and loads a tool's YAML config, including the shared `env:` section |
| `lock`   | Single-instance lock, a Unix socket that can double as the control socket |
| `notify` | Desktop notifications through `notify-send`, with optional action buttons |
| `status` | The `status [--follow] [--format plain\|json\|polybar]` subcommand for bar modules |
| `tray`   | AppIndicator tray applet built from a plain `TraySpec` (Linux only) |
| `icons`  | The ty* icon set and its idle/attention/busy tray colors (Linux only) |
| `wake`   | A "something changed" signal from any thread to the GTK thread |
| `session` | X11 or Wayland, and which compositor (sway, Hyprland, other) |
| `screen` | Screen bounds, monitors, scale and screenshots (X11, or `grim` on Wayland) |
| `wm`     | Windows, the focused window, staging a window for recording, the pointer (X11/i3, sway, Hyprland) |
| `overlay` | Full-screen input overlay for picking regions and windows (layer-shell on Wayland) |
| `clipboard` | Read, write and watch the clipboard (GTK on X11, wl-clipboard on Wayland) |
| `input`  | Type text and keys (enigo / `wtype`) and listen to every key press (XRecord / `/dev/input`) |

The display modules (`session` to `input`) pick the X11 or Wayland backend at runtime, so
the tools don't talk to X11 directly. On Wayland they lean on the wlroots protocols and
helpers: layer-shell, screencopy (`grim`), data-control (`wl-clipboard`) and
virtual-keyboard (`wtype`). `clipboard::ClipboardWatcher` and the `/dev/input` listener
sleep until something happens (`wl-paste --watch`, inotify), in keeping with the tray.

Plus `runtime_dir()` (`$XDG_RUNTIME_DIR`, or the temp dir) and `user_hint()` (`$USER`, or
`"user"`), for per-user runtime files.

## config

A tool's config is found at, in order:

1. `--config <path>` (the Home Manager modules pass this), skipping discovery
2. `./<tool>.yaml`
3. `$XDG_CONFIG_HOME/<tool>/config.yaml` (usually `~/.config/<tool>/config.yaml`)

An empty file is the same as all defaults. `config::load` falls back to `T::default()` when
no file exists; `config::load_required` fails and lists the paths it tried.

Every tool accepts a top-level `env:` map. The variables are set for the tool and the
programs it runs, overriding the inherited environment. A value is a string, or
`{ file: <path> }` to read it from a file (trimmed, `~` expanded), which is handy for
secrets:

```yaml
env:
  GH_TOKEN: { file: /run/secrets/github }
  LANG: en_US.UTF-8
```

`env:` is applied only on the first load, before the tool starts threads; reloading the
config doesn't change it. `env:` is removed before the tool's own config is parsed, so tools
with `deny_unknown_fields` accept it.

Also: `expand_user` / `expand_home` for leading `~` in paths.

## lock

```rust
let lock = ty_common::lock::InstanceLock::acquire("tygesture")?;
// lock.listener() is the bound UnixListener, if the tool takes commands over it.
```

The socket is `$XDG_RUNTIME_DIR/<tool>-<user>.sock` (`lock::socket_path`). If another
instance answers on it, `acquire` fails; a dead socket file is treated as stale and replaced.
The file is removed when the lock is dropped.

## notify

- `notify` / `notify_with_icon`: fire and forget. Failures are logged, never fatal.
- `notify_with_actions_blocking`: shows buttons and blocks until the notification closes,
  returning the chosen key (`"default"` for a click on the body). Falls back to a plain
  notification on libnotify older than 0.7.10.
- `tray::notify_with_actions`: the same without blocking the GTK thread.

Needs `notify-send` on `PATH` at runtime.

## status

Add `StatusArgs` (clap `Args`) to a `status` subcommand and implement `Report` for the
tool's status type (`format_plain`, `format_polybar`; JSON comes from `Serialize`).
`report.print(format)` writes one line and flushes, so `--follow` output works in bar
modules. The polybar format prints an empty line when idle.

## tray

The tool describes the whole tray as data and the applet keeps the AppIndicator in sync.
Nothing polls: the GTK thread sleeps until a click, a message or a wake arrives.

```rust
use ty_common::icons::Tool;
use ty_common::tray::{self, Applet, IconState, MenuEntry, TraySpec};

let (waker, wakeups) = ty_common::wake::pair();
// Other threads call waker.wake() after changing the shared state.
tray::spawn_gtk_thread(move || {
    let (tx, rx) = tray::channel();
    let applet = Rc::new(RefCell::new(Applet::new(
        "tygesture", &Tool::Gesture.tray_icons(), spec(&state), tx,
    )?));
    let on_wake = applet.clone();
    tray::on_wake(wakeups, move || { on_wake.borrow_mut().update(spec(&state)); ControlFlow::Continue });
    tray::drain_actions(rx, |action| { /* handle Action */ ControlFlow::Continue });
    Ok(applet)
});
```

- `TraySpec { icon, title, menu, click_action }`: `icon` is `Normal`, `Attention` or `Busy`;
  `menu` is a list of `MenuEntry` (`Label`, `Action`, `Submenu`, `Separator`) carrying the
  tool's own action type; `click_action` is sent on a middle click (a left click always opens
  the menu).
- `Applet::update` is cheap: it does nothing if the spec is unchanged, and updates labels
  in place when the menu's shape is the same. While the menu is open, updates wait until it
  closes, since tray hosts (e.g. eww) don't resize an open menu (at most a minute, in case a
  host never reports closing).
- `tray::channel()` is the channel for menu actions and other messages; `drain_actions`
  handles them on the GTK thread as they arrive. `on_wake` does the same for a `wake::Waker`,
  which coalesces repeated wakes into one.
- For things that change on their own (a recording's elapsed time), run a timer only while
  they are in progress, not all the time.
- `spawn_gtk_thread` runs GTK on its own thread. Without `$WAYLAND_DISPLAY` or `$DISPLAY`, or if setup fails, it
  logs and returns `None`, so tools keep running without a tray.
- `spawn_blocking` runs work off the GTK thread and hands the result back to it.
- Icons are written to the user's hicolor theme on startup. The applet also re-registers
  with the tray host when the bar starts after the tool or restarts (libappindicator
  doesn't do this on its own).

Don't hold a `RefCell` borrow across GTK calls that run a nested main loop (such as
`gtk::Clipboard::wait_for_*`): `drain_actions`, `on_wake` and timers can run inside them.

## icons

One outline SVG per tool in [`icons/`](icons/): 24px grid, 2px round strokes, drawn in
`currentColor`. `Tool::tray_icons()` recolors them into `<tool>` (idle, `#d3d2d1`),
`<tool>-attention` (`#e5484d`) and `<tool>-busy` (`#f5c542`), with a freedesktop icon as a
fallback.

To add a tool: add its SVG to `icons/` in the same style and a variant to `Tool` (`ALL`,
`name`, `svg`, `fallback`).
