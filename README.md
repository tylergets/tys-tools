# ty's tools

Small desktop tools for Linux. Each one does one job, most sit in the system tray, and all of
them can be scripted from the command line. Built for X11 and used daily on i3; Wayland works
too on wlroots compositors such as sway and Hyprland (see [Wayland](#wayland)). Written in
Rust, packaged with Nix, MIT licensed.

Website: <https://tys.tools> (with every tool's [dependencies](https://tys.tools/deps)).

| Tool | What it does |
| --- | --- |
| [tynotif](crates/tynotif) | Desktop notification daemon with actions, history and do-not-disturb. Opt-in alternative to Dunst. |
| [tyboard](crates/tyboard) | Clipboard history in your tray, with a rofi picker. |
| [tykey](crates/tykey) | Text expansion in every app. |
| [tycapture](crates/tycapture) | Screen recording of the screen, a region or one window, with zoom on clicks. |
| [tyshot](crates/tyshot) | Screenshots you can mark up with arrows, text and blur. |
| [tyspeak](crates/tyspeak) | Dictation that stays on your machine. |
| [tylight](crates/tylight) | Warmer screen colors at night, plus monitor brightness. X11 and wlroots. |
| [tygesture](crates/tygesture) | Touchpad gestures (swipe, pinch, rotate, hold) mapped to commands, read straight from libinput. X11 and Wayland. |
| [typort](crates/typort) | Your running dev servers in the tray: open them in the browser or stop them. |
| [tyci](crates/tyci) | Running CI and recently opened PRs from GitHub, in your tray. |
| [typk](crates/typk) | Polkit authentication agent: the password or fingerprint prompt for actions that need one. |
| [tylock](crates/tylock) | Screen locker: your wallpaper and a clock, unlocked with your password or a finger. X11 and Wayland. |

Each tool's README has the full details.

## Why use them

- **Private.** No telemetry, no accounts, no cloud. Your data stays on your machine.
- **Works offline.** Everything runs locally, including tyspeak's speech models.
- **Plain config.** One YAML file per tool, easy to keep in your dotfiles.
- **Scriptable.** Every tool has a CLI and works with your bar and keybindings.
- **Small and fast.** Native Rust. No Electron.

## Wayland

Every tool runs on Wayland compositors that support the wlroots protocols (layer-shell,
screencopy, data-control and virtual-keyboard): sway, Hyprland, river and similar. KDE covers
most of it; GNOME does not offer these protocols.

| Tool | On Wayland |
| --- | --- |
| tyboard | Watches the clipboard with `wl-paste`; active-window info from sway or Hyprland. |
| tykey | Reads keyboards from `/dev/input` (you need to be in the `input` group) and types with `wtype`. |
| tycapture | Records with `wf-recorder`, one monitor at a time. Window recording uses sway like i3; zoom needs Hyprland. |
| tyshot | Screenshots with `grim`; the overlay covers every monitor. Window snapping on sway and Hyprland. |
| tyspeak | Types with `wtype`, copies with `wl-copy`. |
| tylight | Sets gamma with `wlr-gamma-control` (sway, Hyprland, river, niri, labwc). Brightness over DDC works anywhere. Not on GNOME or KDE. |
| tygesture | Unchanged: it reads libinput directly. Bind gestures to `wtype` and `swaymsg`/`hyprctl` instead of `xdotool` and `i3-msg`. |
| typort, tyci | Unchanged: tray icon and `xdg-open` only. |
| typk | The prompt is a layer-shell surface on the overlay layer, so it takes the keyboard wherever the focus was. |
| tylock | Locks with `ext-session-lock-v1` (sway, Hyprland, river, niri, KDE), drawn at each monitor's real resolution. |

The Nix packages put the Wayland helpers on `PATH`.

## Try it

```sh
# run one
nix run github:tylergets/tys-tools#tyshot

# install all of them
nix profile install github:tylergets/tys-tools
```

## Install with Home Manager

Add the flake:

```nix
{
  inputs.tys-tools.url = "github:tylergets/tys-tools";
  # optional, avoids a second nixpkgs:
  inputs.tys-tools.inputs.nixpkgs.follows = "nixpkgs";
}
```

Then turn on the tools you want:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.default]; # or .tyboard, .tykey, ...

  programs.tyboard.enable = true;
  programs.tyshot = {
    enable = true;
    settings.save.output_dir = "~/Pictures/Screenshots";
  };
  programs.tykey = {
    enable = true;
    expansions = [{trigger = "sig;"; expansion = "Thanks, Tyler";}];
  };
}
```

Every module has the same options:

- `enable`
- `package` — defaults to `pkgs.<tool>` when the overlay is applied, otherwise this flake's package
- `settings` — free-form YAML, written to `~/.config/<tool>/config.yaml`
- `service.enable` (default `true`) — a systemd user service in `graphical-session.target`,
  restarted when the config changes

tykey also has typed options (`expansions`, `snippets`, `matchBehavior`, `boundaryChars`,
`globals`, `notifications.*`), which are merged into `settings`.

### Overlay

```nix
nixpkgs.overlays = [inputs.tys-tools.overlays.default];
# adds pkgs.tyboard, pkgs.tykey, pkgs.tycapture, pkgs.tyshot, pkgs.tyspeak, pkgs.tylight, pkgs.tygesture, pkgs.typort, pkgs.tyci, pkgs.typk, pkgs.tylock
```

## Configuration

Each tool reads its config from `./<tool>.yaml`, then `~/.config/<tool>/config.yaml`, or from
the path given with `--config`.

Every tool's config can also have an `env:` section: environment variables set for the tool
and the programs it runs. A value is a string, or `{ file: <path> }` to read it from a file,
which keeps secrets out of the config (and out of the Nix store):

```yaml
env:
  GH_TOKEN: { file: /run/secrets/github }
```

And an `appearance:` section, the same in every tool. Text the tools draw and their GTK
widgets follow your GTK font unless `font` names another family:

```yaml
appearance:
  font: Iosevka
```

In Home Manager that is `programs.<tool>.appearance.font`.

## Development

Build a single app with `nix build .#tynotif` (or another tool name). `crate2nix` gives
each Rust crate its own derivation, including `ty-common`. Apps reuse dependencies
with matching versions, features, and build settings instead of compiling a separate
dependency cache per app. Different feature sets can still produce separate builds.
Changes to another app's Rust source do not invalidate its build. Multi-binary tools
such as tyboard build all of their own binaries together.

`nix build` installs all wrapped apps; `nix build .#workspace` builds the entire
unwrapped workspace. CI still checks all apps.

```sh
nix develop
cargo test --workspace
cargo run -p tyboard -- run
crate2nix generate   # regenerate Cargo.nix after manifest/lockfile/version changes
nix flake check      # build all tools, run tests, clippy and rustfmt
```

Commit the generated `Cargo.nix` alongside dependency changes. Generation can also
run without entering the development shell:
`nix shell --inputs-from . nixpkgs#crate2nix nixpkgs#cargo --command crate2nix generate`.
The committed graph keeps package evaluation free of import-from-derivation.
Native-library overrides live in `nix/packages.nix`; Crane still runs Cargo tests,
Clippy, and formatting checks.

Shared code (tray applet, single-instance lock, config loading, notifications, `status`
output) lives in [`crates/ty-common`](crates/ty-common). The project website is in
[`site/`](site).

## License

MIT
