# tys-tools

Small Linux/X11 desktop tools that live in the system tray. Built for i3, packaged with Nix.

| Tool | What it does | Was |
| --- | --- | --- |
| [tyboard](crates/tyboard) | Clipboard history daemon (+ `tyboard-rofi` picker) | slyboard |
| [tykey](crates/tykey) | Text expander with tray snippets | slykey |
| [tycapture](crates/tycapture) | Screen recorder with region/window capture and zoom | slycapture |
| [tyshot](crates/tyshot) | Screenshot + annotation overlay | slyshot |
| [tyspeak](crates/tyspeak) | Offline speech-to-text dictation | slyspeak |

Shared code (tray applet, single-instance lock, config discovery, notifications, `status`
output) lives in [`crates/ty-common`](crates/ty-common).

Every tool reads its config from `./<tool>.yaml`, then `~/.config/<tool>/config.yaml`, or from
`--config PATH`.

## Nix

```nix
{
  inputs.tys-tools.url = "github:tylergets/tys-tools";
  # optional, avoids a second nixpkgs:
  inputs.tys-tools.inputs.nixpkgs.follows = "nixpkgs";
}
```

### Overlay

```nix
nixpkgs.overlays = [inputs.tys-tools.overlays.default];
# adds pkgs.tyboard, pkgs.tykey, pkgs.tycapture, pkgs.tyshot, pkgs.tyspeak
# (and pkgs.tys-tools: the unwrapped workspace build)
```

### Home Manager

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
- `service.enable` (default `true`) — systemd user service in `graphical-session.target`,
  restarted when the config changes

tykey also keeps typed options (`expansions`, `snippets`, `matchBehavior`, `boundaryChars`,
`globals`, `notifications.*`), which are merged under `settings`.

### Packages

```sh
nix run github:tylergets/tys-tools#tyshot
nix build .#tyspeak
nix build            # all tools
```

## Development

```sh
nix develop
cargo test --workspace
cargo run -p tycapture -- status
nix flake check      # build all tools + tests + clippy + rustfmt
```

The workspace is built once with [crane](https://github.com/ipetkov/crane). All tools share
one dependency build, and each tool package is a small wrapper over the workspace binaries
that adds its runtime PATH and GTK environment. Evaluation uses no IFD.

## License

MIT
