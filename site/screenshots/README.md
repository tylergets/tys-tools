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

All scenes use Adwaita dark, pinned DejaVu fonts and a plain background. Menus and
dialogs are cropped to their real content with 12px of breathing room; tylock and
tyshot retain their full-screen view. There is no synthetic desktop frame or website
border. Native window decorations are kept only where the app has them. The lock
screen clock is fixed. The applications themselves are real processes. The minimal tray host
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

Add its views to `scenes.json`, add scenes in `scene.py` and any sample inputs in
`fixtures.py`. Capture it, inspect the PNG, and commit it with `manifest.json`. Every
entry in `site/src/tools.js` should have a corresponding image. Do not silently replace
a failed scene with a blank desktop or a fabricated app window.

`scenes.json` maps each image name to its app and caption. App detail pages show a
side-by-side gallery when an app has multiple captures, stacking on narrow screens. `--only tyshot` captures all of
tyshot's views; `--only tyshot-annotate` captures only its annotation scene.

Additional views cover tybat's device/charge menu, tynotif's history, tycapture's
active recording controls, tyshot's annotations, and tylight's schedule graph.
The battery menu uses a private UPower fixture and an inert charge-limit command;
the recording scene starts a real recording inside the sandbox.
