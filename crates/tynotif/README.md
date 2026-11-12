# tynotif

An opt-in desktop notification daemon for X11 and layer-shell Wayland desktops.
Implements `org.freedesktop.Notifications` 1.2 methods and lifecycle signals, so
`notify-send` and applications work without changing their notification clients.
GTK popup cards have rich text, app icons/raw images, urgency borders, action buttons,
dismiss buttons, and progress bars. Popups use the primary monitor (first monitor if none is primary).
The layout puts a 24px icon beside the text, with an app header, a separate title
(up to two lines), and up to three body lines. Progress bars and wrapping action
buttons have extra spacing to keep controls distinct. Hover truncated text to
read it in a tooltip; history retains the full text.
Each card has its own native popup window, with no shared scrolling viewport or
backing panel between notifications. Excess notifications queue when they reach
the visible limit (four by default) or do not fit the monitor's available height.
A separate footer shows the queue count; queued notifications keep their timeouts
until displayed. Existing cards stay in place when another notification arrives.
When more than one notification is active, hovering the stack slides in a
**Dismiss all (N)** button below the cards. It dismisses both visible and queued
notifications, using the same history and close signals as `tynotif close-all`.
The footer is sized to its controls and right-aligned below the cards, rather than
occupying a notification-wide transparent rectangle. It appears without shifting
the cards. A short hover grace
period lets you cross the gaps to reach the control; single notifications keep
their individual dismiss button.

On X11 the popup declares the notification window type and `Tynotif` WM_CLASS.
Compositors should blend its alpha over underlying windows and leave its corners
to GTK. With Picom, disable `transparent-clipping`, `shadow`, `blur-background`,
and stack-wide animations for `class_g = 'Tynotif'`, and set `corner-radius = 0`.
Clipping transparent windows to the wallpaper produces cut-out gaps and jagged
card corners instead of showing the application behind the notifications.

## Try it

Replace your current notification daemon, then send a notification:

```sh
nix run .#tynotif -- run --force
# In another terminal:
notify-send 'Hello' 'This is tynotif'
notify-send -A open=Open 'Action example' 'Click Open'
notify-send -h int:value:65 -h string:x-dunst-stack-tag:volume 'Volume'
```

Only one daemon can own the notification service. `--replace` requests D-Bus
ownership from a daemon that permits replacement. Dunst does not, so use `--force`:
it first tries replacement, then stops `dunst.service` if that service's MainPID
matches the current notification owner's PID. This requires a systemd user session
and `systemctl` (included in the Nix wrapper). Other daemons or manually launched
Dunst processes must be stopped manually. Notifications are not duplicated.
Without this flag, tynotif fails if another daemon owns the service. tynotif allows
subsequent replacement and exits cleanly when replaced, so its `Restart=on-failure`
service does not fight the new owner. Stop tynotif and restart your usual daemon to
switch back (`systemctl --user start dunst.service` for Dunst); a stopped service is
not automatically restarted. Existing notifications/history are not transferred.
There is no D-Bus activation file, so installing the package alone cannot select it
as your notification daemon.

## CLI

### Tray and history

The bell icon opens a menu with a newest-first **History** submenu, like tyboard's.
Select an entry to read its full, selectable text in a scrollable detail window
(Close or Escape dismisses it). Reading history works while notifications are
paused and does not replay stale actions or change do-not-disturb.

**Sleep notifications** offers 15, 30, 60, or 120 minutes, with automatic resume and
a remaining-time label. The bell turns yellow while sleeping or paused. You can
also pause indefinitely, resume immediately, dismiss all active/queued notifications,
clear history, or quit. Middle-click toggles pause/resume. New notifications queue
while sleeping; manual pause/resume cancels any timer. Sleep state is memory-only
and resets when the daemon restarts. A StatusNotifierItem-compatible tray is needed
to display the icon; the CLI remains available without one.

### Commands and demos

Each successful daemon start shows a short, transient **tynotif is running** popup
and prints a readiness message in the terminal. To preview the notification types:

```sh
nix run .#tynotif -- --force --test 3
```

`--test [SECONDS]` starts the daemon and sends examples through its notification D-Bus API,
at the chosen interval after the startup message (default: 3 seconds; range: 1–3600): low/normal/critical urgency, icons,
long text, actions, inline replies, resident actions, transient notifications, stacked progress
updates, rich text, URLs/context menus, raw images, color hints, replacement by ID,
and sticky/default-critical notifications. Actions
print their key in the terminal; inline replies print the submitted text. Most examples expire 1.5 seconds after the chosen
interval (at least 2.5 seconds), so replacement targets remain visible; the final
two are sticky. The daemon stays running after the sequence. The normal visible
limit and hover-to-pause behavior still apply, so queued examples may appear later.

```sh
tynotif run                      # default command
tynotif run --replace            # take over from a replaceable daemon
tynotif run --force              # also stop Dunst's owning user service if needed
tynotif --test 3                 # start and send demo notifications 3 seconds apart
tynotif --test                   # same as --test 3
tynotif status                   # JSON: paused, active count, history count
tynotif list                     # JSON array of active/queued notifications
tynotif close [ID]               # dismiss ID, or the first visible notification
tynotif close-all                # dismiss active and queued notifications
tynotif action [ID] [KEY]         # default/only action, or invoke an explicit key
tynotif context [ID]             # actions/URLs for ID, or all displayed notifications
tynotif open-url [ID]            # open the only URL, otherwise show a URL picker
tynotif is-paused                # true / false
tynotif set-paused true          # also false / toggle
tynotif snooze 30                # sleep for 30 minutes (1–1440), then resume
tynotif history                  # JSON, newest closed notification first
tynotif history-pop [ID]         # redisplay history, returns a new ID
tynotif history-clear
tynotif --config ./config.yaml validate-config
```

History is memory-only, bounded, and cleared on daemon restart. JSON entries have
`id`, `app`, `summary`, `body`, `icon`, `urgency`, `timestamp` (Unix seconds),
`actions` (alternating keys/labels), nullable `progress`, `category`, and
`desktop_entry`. Raw image bytes and color overrides are not exported. This is tynotif's
schema, not `dunstctl history`'s schema. Replayed notifications are sticky copies
with new IDs and no stale client actions. They do not add duplicate history entries.
`status` includes `snooze_remaining_seconds` (null when no timed sleep is set).

Critical notifications default to no expiration. Other timeouts start while a
notification is visible; queued notifications and do-not-disturb do not consume
their timeout. Hovering the popup stack pauses expiration. Do-not-disturb hides
all popups, including critical ones, and queues new arrivals. The queue holds up
to 512 entries and returns a D-Bus error when full.

Supports replacement IDs, `urgency`, `resident`, `transient`, `image-path`/`image_path`,
`image-data`/`image_data`/`icon_data`, `desktop-entry`, `category`, `value` (signed or
unsigned progress), and `x-dunst-stack-tag`/`x-canonical-private-synchronous`/
`synchronous`/`private-synchronous` hints. Boolean hints also accept integer values.
Stack tags are scoped by app name.
Resident notifications remain after an action; transient notifications skip history.
Raw images must be 8-bit RGB/RGBA with valid dimensions, stride, and backing data
(at most 4096 pixels per axis and 16 MiB). They are displayed proportionally, up to
96px. Image data takes precedence over image paths and app icons; legacy `icon_data`
is a fallback. Paths may be absolute paths or `file://` URIs. `desktop-entry` supplies
a fallback application icon when no icon was provided.

`fgcolor`, `bgcolor`, `frcolor`, and `hlcolor` override individual notification
colors. Colors accept `#RGB`, `#RGBA`, `#RRGGBB`, and `#RRGGBBAA`. `hlcolor` also
accepts a comma-separated string or string array for a progress-bar gradient.
Malformed image/color hints are ignored.

Sounds, Dunst rules and the Dunst control D-Bus API are not implemented. Existing
`dunstctl` scripts need migration to the CLI above before replacing Dunst.

## Rich text and interaction

`markup: full` (default) renders Pango markup, including bold, italic, underline,
strikethrough, and spans. `strip` removes tags; `no` displays incoming text literally.
Malformed markup falls back to readable text. `<a href="…">` labels are rendered as
text and their targets appear in the URL/context menu; `<br>` produces a newline.
Inline `<img>` tags show their alt text; images are supplied through the image hints.
History details render the same markup and retain selectable, untruncated text.

Set `format` to a Pango template to replace the separate summary/body labels:

```yaml
format: '<b>%s</b>\n%b'
```

Supported substitutions: `%a` app, `%s` summary, `%b` body, `%c` category, `%S` stack
tag, `%i` icon path/name, `%I` icon basename, `%p` formatted progress, `%n` numeric
progress, and `%%`. `\n` and `\t` become newlines and tabs. Template markup is always
interpreted; notification fields are escaped according to the markup mode.

By default, left-click invokes the configured `action_name` (default: `default`),
or the only ordinary action, otherwise opens a context menu. Middle-click opens
the only URL or a URL picker; right-click opens the notification's context menu.
HTTP(S), FTP, file and mailto URLs are recognized in text and anchor targets.
`show_indicators` adds `(A)` and `(U)` to the header for actions and URLs.
Explicit action buttons, dismiss buttons, and reply fields keep their own behavior.

Mouse bindings are ordered YAML lists of `none`, `do_action`, `open_url`,
`close_current`, `close_all`, `context`, or `context_all`. For example:

```yaml
mouse_left_click: [do_action, close_current]
mouse_middle_click: [open_url, close_current]
mouse_right_click: [context]
dmenu: [vicinae, dmenu, --no-quick-look, --navigation-title, Notifications, --placeholder, Action]
browser: [xdg-open]
```

With an empty `dmenu` list, a native keyboard-navigable GTK menu is used.
Commands are executable/argument arrays, not shell strings. A selected URL is
appended as one argument to `browser`. Menus run asynchronously and pause popup
expiry. If a binding opens a menu, later actions run after selection; cancelling
leaves the notification open. Actions from closed/replaced notifications are discarded.
Inline replies remain in the popup field rather than the menu.

Optional X11 global shortcuts use `modifier+keysym` syntax (Ctrl, Shift, Alt/mod1,
mod2–mod5, Super/Meta/mod4). Caps Lock and Num Lock are handled automatically:

```yaml
shortcuts:
  close: ctrl+mod4+space
  close-all: ctrl+shift+mod4+space
  history-pop: ctrl+mod4+grave
  context: ctrl+mod4+period
  action: ctrl+mod4+Return
```

Shortcuts are disabled by default; conflicts with existing X11 bindings report an
error on startup. On Wayland, bind `tynotif close`, `close-all`, `history-pop`,
`context`, and `action` in your compositor instead. Configured X11 shortcuts are
ignored on Wayland with a startup message.

Try the new rendering with:

```sh
notify-send 'Rich text' '<b>Bold</b>, <i>italic</i>, <a href="https://example.org">a link</a>'
notify-send -h string:bgcolor:#243040 -h string:hlcolor:'#8bd5ca,#c6a0f6' -h int:value:65 'Progress'
```

## Inline replies

Apps can use the KDE `inline-reply` extension to offer a text field directly in
the notification. Type a reply and press Enter or click its submit button.
Keyboard focus is requested only on interaction, and expiration pauses while a
popup has keyboard focus. Empty replies are disabled. Resident notifications
remain open after sending; other notifications close after sending.

tynotif advertises `inline-reply` and sends `NotificationReplied(id, text)` to the
originating app, honoring `x-kde-reply-placeholder-text`,
`x-kde-reply-submit-button-text`, and `x-kde-reply-submit-button-icon-name` hints.
Ordinary actions labeled “Reply” retain their usual app-defined behavior.
`tynotif action` cannot submit an inline reply; use the popup text field.

## Configuration

Shared config discovery: `./tynotif.yaml`, then
`$XDG_CONFIG_HOME/tynotif/config.yaml`; `--config PATH` overrides discovery.
Restart the daemon after changes. All defaults:

```yaml
markup: full
format: null
show_indicators: true
action_name: default
mouse_left_click: [do_action]
mouse_middle_click: [open_url]
mouse_right_click: [context]
dmenu: []
browser: [xdg-open]
shortcuts: {}
width: 480
margin: 18
top: 48
gap: 12
limit: 4
history_limit: 200
timeout_ms: 20000
low_timeout_ms: 5000
background: '#202432'
foreground: '#eef0f6'
border: '#596580'
critical: '#e78284'
# appearance:
#   font: Iosevka
```

Timeout `0` means sticky. Colors must be `#RRGGBB`. `limit` is 1–10;
`history_limit` is 0–10000 (0 disables history), width is 200–1600 logical pixels,
and offsets/gap are 0–1000. Shared `env` and `appearance.font` settings work as in
the other ty tools.

## Home Manager

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tynotif];
  services.dunst.enable = false;
  programs.tynotif = {
    enable = true;
    appearance.font = "Iosevka";
    settings = { width = 580; timeout_ms = 20000; };
    # service.enable = false; # install/configure, but run manually
  };
}
```

The user service follows `graphical-session.target`. Disable any other
notification daemon's autostart when enabling this service.
