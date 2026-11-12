# tylight

`tylight` warms your screen colors after sunset and cools them back at sunrise, like
Redshift or Gammastep. It also controls your monitors' real brightness. It works on X11 and on
wlroots Wayland compositors (Sway, Hyprland, river, niri, labwc, ...).

## Features

- Color temperature follows the sun in steps: day, golden hour, sunset, dusk and night by
  default, blended smoothly as the sun sinks from 8° above to 14° below the horizon (and
  back in the morning). Add, remove or retune the steps in `schedule`.
- Location from **GeoClue2**, with the last fix cached so it starts right offline. A fixed
  location in the config overrides GeoClue.
- **Smooth fades** when you toggle, disable or set a temperature.
- **Disable for a while**: 30 minutes, 1 hour, 2 hours, or until morning.
- **Monitor brightness**: laptop backlights (through logind, no root needed) and external
  monitors over DDC/CI (through `ddcutil`). All detected monitors adjust together; external
  monitors are addressed by I²C bus, including multiple monitors of the same model.
  Each DDC adjustment sends one target without read-back verification, letting the monitor
  handle its own transition instead of waiting through extra software fade steps.
- **Auto brightness** from the ambient light sensor, tuned not to jump around (see below).
- **Presets**: named temperature, dimming and monitor brightness, picked from the tray or
  with `tylight preset <name>`.
- **Graph window** (tray → Show graph, or `tylight graph`): the last and next 12 hours of
  temperature and dimming (dashed = schedule, solid = applied), monitor brightness and
  ambient light, with night shaded.
- Tray applet. Middle-click toggles.
- CLI for keybindings, and `status --follow` for bars.

## Requirements

- X11 (RandR 1.3+), or a Wayland compositor with `wlr-gamma-control-unstable-v1`. GNOME and
  KDE don't support it; use their built-in night light instead.
- GeoClue2, unless you set `location`.
- Optional: `ddcutil` for external monitors (the Nix package includes it). DDC needs
  access to `/dev/i2c-*`: on NixOS set `hardware.i2c.enable = true` and add yourself to
  the `i2c` group.

Stop Gammastep, Redshift, wlsunset or hyprsunset first. Only one program can set the gamma
at a time.

### GeoClue on NixOS

```nix
services.geoclue2 = {
  enable = true;
  appConfig.tylight = {
    isAllowed = true;
    isSystem = true;
  };
};
```

Check it with `tylight location`.

## Commands

```sh
tylight run                     # daemon + tray (the default)
tylight toggle                  # on/off, with a fade
tylight disable --for 1h        # or 30m, 1h30m, ...
tylight disable --until-morning
tylight enable
tylight set 3400                # hold a temperature
tylight auto                    # follow the sun again (also leaves a preset)
tylight preset late-night       # apply a preset
tylight presets                 # list them
tylight auto-brightness [on|off|toggle]
tylight graph                   # open the graph window
tylight brightness              # print monitor brightness
tylight brightness 60           # or +10, -10
tylight status [--follow] [--format plain|json|polybar]
tylight location                # the location tylight would use
tylight validate-config
```

`brightness` goes through the daemon when it's running, so the tray stays in sync. Without
the daemon it changes the displays directly.

## Configuration

`./tylight.yaml` or `~/.config/tylight/config.yaml`. Every key is optional. These are the
defaults:

```yaml
schedule:            # by sun elevation (degrees), highest first; blended in between
  # temperature: kelvin, 1700-25000 (6500 is neutral)
  # brightness: gamma dimming, 0.1-1.0, default 1.0 (not the monitors' own brightness)
  - {name: day,         elevation: 8,   temperature: 6500}
  - {name: golden hour, elevation: 2,   temperature: 5500}
  - {name: sunset,      elevation: -3,  temperature: 4600}
  - {name: dusk,        elevation: -8,  temperature: 4000}
  - {name: night,       elevation: -14, temperature: 3600}
gamma: 1.0
# location:          # set this to skip GeoClue
#   latitude: 39.10
#   longitude: -84.51
fade_seconds: 2      # 0 switches instantly
backend: auto        # auto | x11 | wayland
tray: true
monitor_brightness:
  backlight: true    # /sys/class/backlight panels
  ddc: true          # external monitors via ddcutil
  step: 10           # percent for the tray's Brighter/Dimmer
ambient_light: true  # show the light sensor reading in the tray
auto_brightness:
  enabled: false     # start with it on (toggle at runtime from the tray)
  curve:             # [lux, percent], interpolated in log-lux
    - [0, 10]
    - [10, 30]
    - [100, 55]
    - [1000, 85]
    - [10000, 100]
  smoothing_seconds: 4
  hysteresis: 0.25   # ignore light changes within ±25%
  brighten_delay: 3  # seconds the light must stay brighter
  dim_delay: 15      # seconds the light must stay darker
  fade_seconds: 1.5
  ddc: false         # also adjust external monitors (slow)
presets:             # setting this replaces the defaults; omitted fields follow the schedule
  reading: {temperature: 5000}
  evening: {temperature: 3400, brightness: 0.9}
  late-night: {temperature: 2400, brightness: 0.75, monitor_brightness: 15}
```

### Auto brightness without jumping around

- The light level is smoothed in log scale (eyes respond to ratios, not lux).
- Changes within `hysteresis` of the level at the last adjustment are ignored.
- The light must stay past that band for `brighten_delay` (short) or `dim_delay` (long), so
  a hand or passing shadow over the sensor does nothing. Then the screen fades once to the
  level the light settled at.
- Changing brightness by hand (tylight, the tray or your brightness keys) while auto is on
  shifts the curve by that much instead of being undone.

### The schedule

Each stop is the setting once the sun is at that elevation. Above the first stop it's
day, below the last it's night, and in between tylight blends the two stops around the
sun. The same steps run in reverse at dawn. Setting `schedule` replaces the whole list,
and `name` is what `tylight status` shows ("4600K · sunset").

For reference: the sun sets at about -1°, civil twilight ends at -6°, nautical at -12°.
Far from the equator in summer the sun may never get to the last stop, so the night
setting isn't reached.

Old configs with `temperature: {day, night}` translate to two stops:

```yaml
schedule:
  - {elevation: 3, temperature: 6500}
  - {elevation: -6, temperature: 4000}
```

## i3

```
bindsym $mod+F9 exec tylight toggle
bindsym XF86MonBrightnessUp exec tylight brightness +10
bindsym XF86MonBrightnessDown exec tylight brightness -10
```

## Notes

- On X11, each display's original gamma is saved and put back when tylight exits. On
  Wayland the compositor restores it on its own.
- The ambient light reading comes from IIO sysfs (`/sys/bus/iio/devices/*/in_illuminance_*`).
