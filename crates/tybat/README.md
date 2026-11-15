# tybat

Battery monitor and tray applet for Linux, replacing batsignal. Critical laptop battery
warnings take keyboard focus over a dimmed desktop and count down to a power action.
Plugging in cancels the countdown; Escape or the Snooze button postpones it.

The tray shows a battery-level icon and laptop percentage (if the tray host displays labels).
Its menu lists laptop batteries and connected peripherals with percentages and charging
states. UPower supplies device discovery, with BlueZ Battery1 as a fallback for Bluetooth
devices. Devices must report a battery level to Linux; not all headphones do. Discovery
refreshes every ten seconds in a worker thread. Only system batteries trigger warnings
and power actions; a low mouse or headset never suspends the laptop.

## Usage

```sh
tybat run       # daemon and tray; also the default command
tybat status    # laptop and device battery levels
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
