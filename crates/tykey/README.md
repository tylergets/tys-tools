# tykey

`tykey` is a small global text expansion CLI for Linux X11 (mainly used on i3).

This project is an eternal work in progress, built for my personal setup and day-to-day use. PRs are welcome, especially for support in other desktop environments.

It listens globally for trigger strings and replaces them with configured expansion text or macro sequences.

## Features

- Global trigger detection on X11
- YAML-based expansion config
- Expansion action macros (keys, delays, caret movement)
- Template macros for datetime (`DATETIME`, `DATE`, `TIME`), Linux commands (`CMD`), and emoji shortcodes (`EMOJI`)
- Configurable global template macros (`globals`)
- Nix flake packaging
- Home Manager module with declarative expansions

## Requirements

- Linux with X11 session
- Input simulation support for your session/environment
- For Nix usage: flakes enabled

## Install and run

### Cargo (development)

```bash
cargo run -p tykey -- run
```

Validate config only:

```bash
cargo run -p tykey -- validate-config
```

### Nix flake app

```bash
nix run .#tykey
```

### Nix dev shell

```bash
nix develop
cargo check
```

## CLI usage

```bash
tykey [OPTIONS] [COMMAND]
```

Commands:

- `run` (default when omitted)
- `validate-config`

Options:

- `-c, --config <PATH>`: explicit config path override (including Nix store paths)

Examples:

```bash
tykey run
tykey validate-config
tykey --config /path/to/config.yaml run
tykey -c /path/to/config.yaml validate-config
```

## Configuration

### Config lookup order (without `--config`)

1. `tykey.yaml` in current working directory
2. `~/.config/tykey/config.yaml`

If both exist, the CWD config takes precedence.

### Config schema

```yaml
watch: false # optional, auto-reload config when file changes
match_behavior: immediate # immediate | boundary
boundary_chars: " \t\n.,;:!?)]}>'\"" # optional
notifications: # optional desktop notifications (via notify-send)
  on_expansion: false
  on_snippet_copy: false
globals: # optional template macro definitions
  SIGNOFF: "Thanks, Tyler{{KEY:ENTER}}"
  TODAY_NOTE: "Generated on {{DATE}}"
  ROCKET: "{{EMOJI:rocket}}"
expansions:
  - trigger: "tg@"
    expansion: "tylergetsay@gmail.com"
  - trigger: "sig;"
    expansion: "{{SIGNOFF}}"
  - trigger: "ship;"
    expansion: "Shipped {{ROCKET}}"
snippets: # optional tray menu clipboard items
  - title: "Personal email"
    content: "tylergetsay@gmail.com"
  - title: "Address"
    content: "123 Main St ({{TODAY_NOTE}})"
  - title: "Ship status"
    content: "Shipped {{EMOJI:rocket}}"
```

### Expansion action macros

Supported action macros inside `expansion`:

- `{{KEY:...}}` for key presses
- `{{SLEEP_MS:...}}` for timing pauses
- `{{MOVE_CARET:...}}` for caret movement steps

Examples:

- `{{KEY:ENTER}}`
- `{{KEY:TAB}}`
- `{{KEY:ESC}}`
- `{{KEY:BACKSPACE}}`
- arrow keys, home/end, delete, page keys, `F1..F12`
- `{{SLEEP_MS:100}}`
- `{{MOVE_CARET:-5}}` (left 5), `{{MOVE_CARET:3}}` (right 3)

Any non-macro text in `expansion` is typed literally.

### Notifications

Desktop notifications are optional and sent with `notify-send` (libnotify; the Nix package puts it on `PATH`):

- `notifications.on_expansion`: notify when a trigger expansion fires
- `notifications.on_snippet_copy`: notify when a tray snippet is copied to clipboard

### Config auto-reload

Set `watch: true` to watch the loaded config file and hot-reload expansions when it changes.

### Template macros

Template macros work in `expansion`, `snippets[].content`, and `globals` values:

- `{{DATETIME}}` -> local datetime (`YYYY-MM-DD HH:MM:SS`)
- `{{DATE}}` -> local date (`YYYY-MM-DD`)
- `{{TIME}}` -> local time (`HH:MM:SS`)
- `{{CMD:<linux shell command>}}` -> command stdout with trailing newlines trimmed
- `{{EMOJI:<emoji-shortcode>}}` -> emoji character (for example `{{EMOJI:rocket}}` -> `🚀`)

`globals` entries become new template macros. Macro names are case-insensitive and can reference other globals, e.g. `{{SIGNOFF}}` or `{{today_note}}`.

Examples:

- `Meeting on {{DATE}} at {{TIME}}`
- `Generated {{DATETIME}}`
- `Git branch: {{CMD:git branch --show-current}}`
- `Shipped {{EMOJI:rocket}}`
- `globals: { SIGNOFF: "Thanks, Tyler{{KEY:ENTER}}" }`

## Home Manager Module

Part of the [tys-tools](../../README.md) flake:

```nix
{inputs, ...}: {
  imports = [inputs.tys-tools.homeManagerModules.tykey];
  programs.tykey = {
    enable = true;
    settings = { /* same keys as the YAML config above */ };
  };
}
```

Options: `enable`, `package` (defaults to `pkgs.tykey` with the tys-tools overlay), `settings`
(written to `~/.config/tykey/config.yaml`), and `service.enable` (systemd user service, default
on). tykey also has typed `expansions`, `snippets`, `matchBehavior`, `boundaryChars`, `globals` and `notifications.onExpansion`/`onSnippetCopy` options, merged under `settings`.

## Project layout

- `src/config.rs`: config loading + validation
- `src/core/engine.rs`: trigger matching and expansion execution
- `src/core/expansion.rs`: macro parsing
- `src/io/`: input/output interfaces
- `src/platform/x11_rdev.rs`: X11 backend (`rdev` listener + `enigo` output)
- `../../nix/modules/tykey.nix`: Home Manager module

## License

Add a license section/file if you plan to publish this project.
