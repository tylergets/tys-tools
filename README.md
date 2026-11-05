# ty's tools

Small desktop tools for Linux. Each one does one job, most sit in the system tray, and all of
them can be scripted from the command line. Built for X11 and used daily on i3. Written in
Rust, packaged with Nix, MIT licensed.

| Tool | What it does |
| --- | --- |
| [tyboard](crates/tyboard) | Clipboard history in your tray, with a rofi picker. |
| [tykey](crates/tykey) | Text expansion in every app. |
| [tycapture](crates/tycapture) | Screen recording of the screen, a region or one window, with zoom on clicks. |
| [tyshot](crates/tyshot) | Screenshots you can mark up with arrows, text and blur. |
| [tyspeak](crates/tyspeak) | Dictation that stays on your machine. |
| [tylight](crates/tylight) | Warmer screen colors at night, plus monitor brightness. X11 and wlroots. |
| [tygesture](crates/tygesture) | Touchpad gestures (swipe, pinch, rotate, hold) mapped to commands, read straight from libinput. X11 and Wayland. |
| [typort](crates/typort) | Your running dev servers in the tray: open them in the browser or stop them. |
| [tyci](crates/tyci) | Running CI and recently opened PRs from GitHub, in your tray. |

Each tool's README has the full details.

## Why use them

- **Private.** No telemetry, no accounts, no cloud. Your data stays on your machine.
- **Works offline.** Everything runs locally, including tyspeak's speech models.
- **Plain config.** One YAML file per tool, easy to keep in your dotfiles.
- **Scriptable.** Every tool has a CLI and works with your bar and keybindings.
- **Small and fast.** Native Rust. No Electron.

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
# adds pkgs.tyboard, pkgs.tykey, pkgs.tycapture, pkgs.tyshot, pkgs.tyspeak, pkgs.tylight, pkgs.tygesture, pkgs.typort, pkgs.tyci
```

## Configuration

Each tool reads its config from `./<tool>.yaml`, then `~/.config/<tool>/config.yaml`, or from
the path given with `--config`.

## Development

```sh
nix develop
cargo test --workspace
cargo run -p tyboard -- run
nix flake check      # build all tools, run tests, clippy and rustfmt
```

Shared code (tray applet, single-instance lock, config loading, notifications, `status`
output) lives in [`crates/ty-common`](crates/ty-common). The project website is in
[`site/`](site).

## License

MIT
