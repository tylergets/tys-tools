# tynotif

An opt-in desktop notification daemon for X11 and layer-shell Wayland desktops.
Implements `org.freedesktop.Notifications` 1.2 methods and lifecycle signals, so
`notify-send` and applications work without changing their notification clients.
GTK popup cards have app icons, urgency borders, action buttons, dismiss buttons,
and progress bars. Popups use the primary monitor (first monitor if none is primary).
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
updates, replacement by ID, and sticky/default-critical notifications. Actions
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
tynotif action ID KEY            # invoke an advertised action
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
`actions` (alternating keys/labels), and nullable `progress`. This is tynotif's
schema, not `dunstctl history`'s schema. Replayed notifications are sticky copies
with new IDs and no stale client actions. They do not add duplicate history entries.
`status` includes `snooze_remaining_seconds` (null when no timed sleep is set).

Critical notifications default to no expiration. Other timeouts start while a
notification is visible; queued notifications and do-not-disturb do not consume
their timeout. Hovering the popup stack pauses expiration. Do-not-disturb hides
all popups, including critical ones, and queues new arrivals. The queue holds up
to 512 entries and returns a D-Bus error when full.

Supports replacement IDs, `urgency`, `resident`, `transient`, `image-path`,
`value` (progress), `x-dunst-stack-tag`, and
`x-canonical-private-synchronous` hints. Stack tags are scoped by app name.
Resident notifications remain after an action; transient notifications skip history.
Body text is plain text: markup, inline/raw image data, sounds, URL opening,
Dunst rules and the Dunst control D-Bus API are not implemented. Existing
`dunstctl` scripts need migration to the CLI above before replacing Dunst.

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
