# Repeatable app screenshots

From the repository root:

```sh
nix build .#default --out-link /tmp/tys-tools-apps
nix run .#screenshots -- --apps /tmp/tys-tools-apps
```

Or run `npm run screenshots` from `site/` to build and capture in one command.

The default destination is `site/public/screenshots`. Capture just one scene while
editing it, or use a scratch output directory:

```sh
nix run .#screenshots -- --apps /tmp/tys-tools-apps --only tyboard --only tyshot \
  --output /tmp/tys-tools-screenshots
```

Logs go to `$TMPDIR/tys-tools-screenshot-logs` (usually under `/tmp`), configurable with
`--logs`. Failed scenes make the command fail; successful scenes can be retained and
only failed scenes rerun. The website's build checks that every listed tool has a PNG
of the expected size and a matching manifest hash.

`--apps` must point to a built package containing all requested executables in `bin/`.
The app build and capture environment use this repository's `flake.lock`. No production
application needs screenshot flags or modified rendering code.

## Isolation and visual conventions

Each image gets a new Bubblewrap user/PID/network/mount namespace, Xvfb at 1200×800
and 96 DPI, an Openbox window manager, a private D-Bus, and an empty home directory.
There is no access to the real desktop, clipboard, credentials, hardware, system bus,
network, or other running processes. The store is read-only. Linux must permit
unprivileged user namespaces; the script fails rather than capturing the host desktop.

All scenes use Adwaita dark, pinned DejaVu fonts, the same sample desktop and a fixed
desktop clock. The applications themselves are real processes. The minimal tray host
renders their real exported menus using libdbusmenu-gtk3. GUI interactions use normal
CLI commands, D-Bus calls and xdotool. tyquote is captured in a real xterm.

`fixtures.py` holds sample configuration and data. Clipboard entries are copied into
the private clipboard. typort gets a local sample HTTP server. tyci receives synthetic
`gh` responses through PATH. typk receives a normal authentication request from a
private fixture authority; its sample helper always rejects authentication. tylock uses
an inert PAM configuration inside the namespace, a fixed clock format and a sample quote.
The whole namespace is destroyed afterward, including the isolated locked display.
tyspeak shows its model picker before downloading a model, and tygesture shows its
configured bindings while waiting for a touchpad; no hardware state is fabricated.

The goal is repeatable content and framing, not byte-identical pixels: timers, process
IDs and asynchronous rendering can vary. `manifest.json` records each captured binary's
version, dimensions, visual settings and image SHA-256. Review the PNGs after changing
scenes or upgrading GTK/fonts. Screenshots are committed so the website deploy does not
have to build every native app.

## Adding an app

Add its name to `capture.py`, add a scene in `scene.py` and any sample inputs in
`fixtures.py`. Capture it, inspect the PNG, and commit it with `manifest.json`. Every
entry in `site/src/tools.js` should have a corresponding image. Do not silently replace
a failed scene with a blank desktop or a fabricated app window.
