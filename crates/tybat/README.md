# tybat

Battery monitor and tray applet for Linux, replacing batsignal. Critical laptop battery
warnings take keyboard focus over a dimmed desktop and count down to a power action.
Plugging in cancels the countdown; Escape or the Snooze button postpones it.

The tray uses the shared ty-tools icon palette (neutral, yellow when low, red when critical)
and laptop percentage (if the tray host displays labels).
Its menu lists laptop batteries and connected peripherals with percentages and charging
states. UPower supplies device discovery, with BlueZ Battery1 as a fallback for Bluetooth
devices. Devices must report a battery level to Linux; not all headphones do. Discovery
refreshes every ten seconds in a worker thread. Only system batteries trigger warnings
and power actions; a low mouse or headset never suspends the laptop.

While a battery is discharging, its tray menu and `tybat status` show measured
percentage loss per hour and approximate time remaining. Laptop time remaining also
appears beside the tray percentage on hosts that display labels. Watts are shown
when UPower provides them, but estimates need only percentage readings.

The daemon measures up to the last 30 minutes of percentage readings, refreshing
every ten seconds. Estimates appear after at least a minute and a one-percentage-point
drop, and change with workload. Charging, a rising percentage, device removal,
discovery failures or sampling gaps over a minute reset the measurement.
`tybat status` reads the daemon's latest measurements; without the daemon it shows
current battery levels and available watts. Each discharging battery has its own
measurement; devices that do not report a discharging state show levels only.

## Usage

```sh
tybat run       # daemon and tray; also the default command
tybat status    # battery levels, measured drain and estimated time remaining
tybat preview   # focus-grabbing countdown; never performs a power action
tybat --config /path/to/config.yaml run
```

## Home Manager

```nix
imports = [ inputs.tys-tools.homeManagerModules.tybat ];
programs.tybat = {
  enable = true; # includes the graphical-session user service
  settings = {
    warning_percent = 20;
    critical_percent = 5;
    countdown_seconds = 60;
    snooze_seconds = 60;
    poll_seconds = 5;
    action = "suspend"; # suspend | hibernate | none
  };
};
```

Enable `services.upower.enable = true` in NixOS. Bluetooth batteries require BlueZ
and a connected device exposing its battery. Remove `services.batsignal.enable` from
Home Manager. Keep UPower's emergency action threshold below tybat's critical threshold
so it does not preempt the popup. Hibernate requires working system hibernation.

### Framework maximum charge

Set `programs.tybat.chargeLimit` to enable charge control:

```nix
programs.tybat.chargeLimit = {
  percent = 80;
  command = [ "/run/wrappers/bin/sudo" "-n" "/path/to/restricted-charge-limit-helper" ];
};
```

The helper receives one appended percentage argument and must allow values 25–100.
It should invoke `framework_tool --charge-limit` with appropriate privileges. Grant
passwordless access only to the restricted helper, rather than all framework_tool commands.
Disable any old service setting a fixed limit so it does not overwrite tray selections.

The tray's **Maximum charge** submenu offers 25–100% in 5% steps and **Reset to declared
default**. The declared percentage can be any integer in that range. Successful selections
are saved under `$XDG_STATE_HOME/tybat/charge-limit` (normally `~/.local/state/tybat/charge-limit`).
Tybat applies the selected value at startup and after logind reports resume; changing the
declared default discards the previous override. Charge control only runs while the daemon
is running. Failed helper commands are shown in the menu and sent as notifications.

Equivalent YAML (omitting `charge_limit` disables the feature):

```yaml
charge_limit:
  percent: 80
  command: [/run/wrappers/bin/sudo, -n, /path/to/restricted-charge-limit-helper]
```

## YAML

Config discovery: `./tybat.yaml`, then `$XDG_CONFIG_HOME/tybat/config.yaml`
(usually `~/.config/tybat/config.yaml`). An explicit `--config` overrides discovery.

```yaml
warning_percent: 20
critical_percent: 5
countdown_seconds: 60
snooze_seconds: 60
poll_seconds: 5
action: suspend
```

These are the defaults. Percentages must satisfy `0 < critical < warning <= 100`;
all durations must be positive integers in seconds. `action: none` closes the popup
when the countdown finishes and repeats after the snooze interval. Shared `appearance.font`
and `env` settings are supported. Restart tybat after changing YAML; Home Manager's
service restarts when its generated config changes.

The lowest discharging system battery from `/sys/class/power_supply` drives alerts.
While a popup is active, power is checked every second and again before acting.
External power, recovery above the critical threshold, or unreadable battery data
cancels it. No system battery is a normal idle state. Failed power actions are reported
and retried with a fresh countdown after the snooze interval.

## Requirements

Linux, GTK 3, UPower on the system bus, a graphical session and an optional SNI tray.
The Nix package supplies `notify-send` and `systemctl`. Power actions use logind through
`systemctl` with the session's normal permissions. The shared overlay grabs focus on
X11 and uses exclusive layer-shell keyboard focus on sway/Hyprland and other supported
Wayland compositors. Compositors without layer-shell may deny focus to the fallback window.
