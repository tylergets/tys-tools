# tygesture

`tygesture` turns touchpad gestures into shell commands. It reads gestures straight from
libinput (the `input` crate), so it works the same under X11 and Wayland, and it sees the
actual motion of each gesture instead of parsing `libinput debug-events` text.

Proof of concept. It supports swipe, pinch, rotate and hold, plus `repeat` bindings that fire
continuously while the gesture is still going (a 3-finger swipe as a volume slider, a 2-finger
twist as a brightness knob).

## Requirements

- Read access to `/dev/input/event*`, normally by being in the `input` group
  (NixOS: `users.users.<you>.extraGroups = ["input"];`).
- libinput doesn't grab devices, so tygesture runs alongside X or your compositor. Gestures
  your compositor already handles (e.g. sway `bindgesture`) will fire in both.

## Usage

```bash
tygesture debug            # print gestures as you make them (nothing is run)
tygesture debug --updates  # ...including every motion update
tygesture check            # validate the config and list bindings
tygesture run              # the daemon (default)
```

Use `debug` to tune thresholds: each gesture prints its total motion when it ends.

While running, a tray icon flashes when a gesture fires and shows the last one. Its menu
(or a middle click) pauses tygesture; the icon turns red while paused.

## Config

`./tygesture.yaml`, then `~/.config/tygesture/config.yaml`, or `--config <path>`.

```yaml
tray: true         # tray icon (default true)

thresholds:
  swipe: 200       # finger travel for a swipe, libinput units (1000 per inch)
  swipe_step: 120  # travel between firings of a repeat swipe
  pinch: 0.2       # scale change: "in" below 0.8, "out" above 1.2
  rotate: 25       # degrees (also the step for repeat rotates)

swipe:
  3:
    left: xdotool key alt+Right      # fingers move left: browser forward
    right: xdotool key alt+Left
  4:
    left: i3-msg workspace prev
    right: i3-msg workspace next
    up:
      command: pactl set-sink-volume @DEFAULT_SINK@ +5%
      repeat: true                   # keeps firing while you swipe
    down:
      command: pactl set-sink-volume @DEFAULT_SINK@ -5%
      repeat: true

pinch:
  2:
    in: xdotool key ctrl+minus
    out: xdotool key ctrl+plus

rotate:
  2:
    cw: { command: brightnessctl set +5%, repeat: true }
    ccw: { command: brightnessctl set 5%-, repeat: true }

hold:
  4: rofi -show drun
```

- Keys under `swipe`, `pinch`, `rotate` and `hold` are finger counts (`3` or `"3"`).
- A binding is a command string, or `{ command, repeat }`.
- Plain bindings fire once when the gesture ends. A gesture that fired a repeat binding
  doesn't also fire at the end.
- Pinch and rotate are the same libinput gesture; at the end, whichever went further past its
  threshold wins.
- Commands run with `sh -c`; `$TYGESTURE` holds the gesture name (e.g. `swipe 3 left`).

## Home Manager

```nix
programs.tygesture = {
  enable = true;
  settings.swipe."3" = {
    left = "xdotool key alt+Right";
    right = "xdotool key alt+Left";
  };
};
```
