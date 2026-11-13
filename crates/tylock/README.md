# tylock

`tylock` locks your screen: your wallpaper with a large clock and the date, and your
password or a finger to unlock. It works on X11 and on Wayland compositors with
`ext-session-lock-v1` (sway, Hyprland, river, niri, KDE, ...).

## Features

- **Fingerprint or password**, whichever comes first. With a reader and enrolled prints
  the lock screen shows a fingerprint and what the reader says ("Place your finger on the
  fingerprint reader", "Failed to match fingerprint"); start typing and it switches to the
  password field. Esc goes back.
- **Unlocks through PAM**: one PAM service for passwords, another for fingers (see
  [PAM](#pam)), so your system decides what is accepted.
- **Your wallpaper**, scaled to cover each monitor, dimmed (and blurred, if you like) so the
  clock stays readable.
- **Sharp on any monitor**: every monitor is drawn at its real resolution (fractional
  scaling included on Wayland), and the layout scales with its size, so a 4K monitor looks
  like a 1080p one, only crisper. Every monitor shows the clock; hotplugging works.
- Animated: the fingerprint and the password field cross-fade, dots pop in as you type
  and shimmer while the password is checked, a wrong password or finger shakes, and a
  matching finger ripples before the screen unlocks.
- 12-hour clocks (`time_format: "%-I:%M %p"`) draw AM/PM smaller next to the digits.
- The password field grows with long passwords.
- Caps Lock warning, PAM's own messages (e.g. pam_faillock's) on failure, and a hidden
  cursor.
- Keeps Dunst, tynotif, menus, and other desktop popups behind the lock screen,
  including composited X11 notifications that do not report the locker as obscured.
- Safe: on Wayland the compositor keeps the session locked even if tylock crashes. On X11,
  tylock grabs the keyboard and pointer and keeps its window on top.

## Requirements

- Wayland: a compositor with `ext-session-lock-v1`. X11: any window manager.
- PAM services `tylock` and, for fingers, `tylock-fingerprint` (below).
- Fingerprints: `fprintd` with your prints enrolled (`fprintd-enroll`).

## PAM

Passwords are checked with the `tylock` PAM service, fingers with `tylock-fingerprint`.
They run side by side, so the password service must not include `pam_fprintd` (it would
wait for the reader before asking for the password), and the fingerprint service should
have only `pam_fprintd`.

On NixOS:

```nix
security.pam.services.tylock.fprintAuth = false;
security.pam.services.tylock-fingerprint = lib.mkIf config.services.fprintd.enable {
  unixAuth = false;
  fprintAuth = true;
};
```

Elsewhere, `/etc/pam.d/tylock` can include your usual login stack without fprintd, and
`/etc/pam.d/tylock-fingerprint`:

```
auth     required  pam_fprintd.so
account  include   login
```

Without a working fingerprint service tylock just shows the password field.

## Usage

```sh
tylock                    # lock; returns when unlocked
tylock --daemonize        # lock; return as soon as the screen is locked
tylock validate-config
```

Running it while the screen is locked does nothing. tylock is meant to be run by your idle
daemon:

- **X11** with `xss-lock` (`loginctl lock-session`, idle and sleep):
  `xss-lock --transfer-sleep-lock -- tylock`. tylock releases the sleep lock once the
  screen is locked, so the machine never suspends unlocked.
- **Wayland** with `swayidle -w`: `lock 'tylock -f'` and `before-sleep 'tylock -f'`.

## Configuration

`./tylock.yaml` or `~/.config/tylock/config.yaml`. Every key is optional. These are the
defaults:

```yaml
wallpaper: ~/.background-image   # unset: this file if it exists, else colors.background
blur: 0                  # wallpaper blur radius, in px on a 1080p screen
dim: 0.3                 # darken the wallpaper, 0–1
time_format: "%H:%M"     # strftime (chrono), e.g. "%-I:%M" for 12-hour
date_format: "%A, %B %-d"
clock_font: null         # font for the clock; unset uses appearance.font
quote_command: null      # e.g. [tyquote, today]; executable and arguments, no shell
fingerprint: true        # also unlock with a finger, when fprintd has your prints
pam_service: tylock
fingerprint_pam_service: tylock-fingerprint
colors:                  # #rrggbb or #rrggbbaa
  text: "#f2f2f2"
  muted: "#c0c0c0"       # hints
  accent: "#a3b8d8"      # password field border, fingerprint
  error: "#e99089"
  success: "#a7d7b0"
  warning: "#e7d39a"     # Caps Lock
  background: "#242424"  # field and button fill, and the screen without a wallpaper
appearance:
  font: SF Pro Text      # the shared ty* font for all other text
```

## Home Manager

`quote_command` runs once after the screen is locked. Its plain-text stdout is shown
below the unlock controls (wrapped, up to four lines). Commands have a three-second
timeout and a 16 KiB output limit; failure leaves the quote blank. It never blocks
locking or authentication. Use an absolute executable path when configuring Nix.

```nix
programs.tylock = {
  enable = true;
  wallpaper = config.stylix.image;   # or any image path
  settings = {
    blur = 12;
    colors.accent = "#88c0d0";
  };
};
```

The module installs tylock and writes its config; hook it up to your idle daemon as
shown above. The PAM services are system configuration (see [PAM](#pam)).

## How it works

The screen is locked by `ty-common`'s `screenlock` layer: on Wayland an
`ext-session-lock-v1` lock with one lock surface per output, drawn into shared-memory
buffers; on X11 an override-redirect window over the root window with the keyboard and
pointer grabbed, raised again whenever something covers it or another root window is
mapped or restacked, and redrawn on RandR changes. Notifications stay behind the locker;
their daemon's pause state is unchanged. On Wayland the compositor isolates lock surfaces
from ordinary desktop windows.
tylock draws each monitor with cairo and pango. A password thread runs `pam_authenticate`
for each password you submit, and a fingerprint thread keeps a fingerprint-only PAM
transaction running, restarting it after timeouts and failed scans.
