# tynotif

An opt-in desktop notification daemon for X11 and layer-shell Wayland desktops.
Implements `org.freedesktop.Notifications` 1.2 methods and lifecycle signals, so
`notify-send` and applications work without changing their notification clients.
GTK popup cards have app icons, urgency borders, action buttons, dismiss buttons,
and progress bars. Popups use the primary monitor (first monitor if none is primary).
The compact layout puts a 24px icon beside an inline app/title header, with up to
three body lines underneath and wrapping action buttons. Hover truncated text to
read it in a tooltip; history retains the full text.
Cards have opaque backgrounds with transparent gaps between them, and a footer shows how many notifications are
queued beyond the visible limit (four by default). Existing cards stay in place
when another notification arrives.

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

Each successful daemon start shows a short, transient **tynotif is running** popup
and prints a readiness message in the terminal. To preview the notification types:

```sh
nix run .#tynotif -- --force --test 3
```

`--test [SECONDS]` starts the daemon and sends examples through its notification D-Bus API,
at the chosen interval after the startup message (default: 3 seconds; range: 1–3600): low/normal/critical urgency, icons,
long text, actions, resident actions, transient notifications, stacked progress
updates, replacement by ID, and sticky/default-critical notifications. Actions
print their key in the terminal. Most examples expire 1.5 seconds after the chosen
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
