# tynotif

An opt-in desktop notification daemon for X11 and layer-shell Wayland desktops.
Implements `org.freedesktop.Notifications` 1.2 methods and lifecycle signals, so
`notify-send` and applications work without changing their notification clients.
GTK popup cards have app icons, urgency borders, action buttons, dismiss buttons,
and progress bars. Popups use the primary monitor (first monitor if none is primary).

## Try it

Stop your current notification daemon first, then run:

```sh
nix run .#tynotif -- run
# In another terminal:
notify-send 'Hello' 'This is tynotif'
notify-send -A open=Open 'Action example' 'Click Open'
notify-send -h int:value:65 -h string:x-dunst-stack-tag:volume 'Volume'
```

Only one daemon can own the notification service. tynotif fails with an error if
another daemon owns it. Stop tynotif and restart your usual daemon to switch back.
There is no D-Bus activation file, so installing the package alone cannot select it
as your notification daemon.

## CLI

```sh
tynotif run                      # default command
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
