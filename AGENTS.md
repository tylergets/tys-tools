# Agent Instructions for `tys-tools`

Monorepo for the ty* X11 desktop tools. One Cargo workspace, one Nix flake.

## Layout
- `crates/ty-common` — shared code: tray applet (`tray`), single-instance lock (`lock`),
  config discovery/loading (`config`), notifications (`notify`), `status` CLI helpers, and the
  X11/Wayland display layer: `session` (which one), `screen` (size, screenshot), `wm`
  (windows, i3/sway/Hyprland IPC, pointer), `overlay` (full-screen input overlay, floating
  windows), `clipboard`, `input` (typing, global key listening).
  If two tools need the same thing, it goes here, not in both. Tools should not talk to X11
  or Wayland directly.
- `crates/<tool>` — tyboard, tykey, tycapture, tyshot, tyspeak.
- `nix/packages.nix` — a separate crane build and dependency cache per tool + a thin
  wrapper (runtime PATH / GTK env). Sources include the selected tool and `ty-common`,
  with unrelated workspace members stubbed for Cargo resolution. The explicit `workspace`
  package and workspace-wide checks still build everything. Add runtime programs to the
  tool's `runtimeDeps` there.
- `nix/modules/lib.nix` — shared Home Manager module builder; `nix/modules/<tool>.nix` per tool.

## Environment
- Use `nix develop`; `cargo` is available inside it.

## Required Workflow
- When you create a new file, stage it immediately with `git add <path>` (flakes only see tracked files).
- When a tool's config format changes, update its README and, if it has typed options,
  `nix/modules/<tool>.nix`.
- Shared dependencies are declared once in the root `Cargo.toml` `[workspace.dependencies]`;
  crates use `dep.workspace = true`.
- Those versions are pinned exactly (`"=1.2.3"`), so `cargo update` won't move them. To
  upgrade a dependency, change its pin in the root `Cargo.toml`.

## Validation
Builds are slow and CPU-heavy, so run them sparingly.
- While iterating, use the cheapest check that covers the change: `cargo check -p <tool>`,
  then `cargo test -p <tool>` only if you touched tested logic. Scope to the crate you changed.
- Do not run `nix build` or `nix flake check` after every edit. Run `nix flake check` once,
  at the end, only before a release/deploy or when the user asks.
- Run `nix build .#<tool>` only when you changed Nix files (`nix/`, `flake.nix`) or runtime deps.
- Skip validation entirely for docs-only, README, or comment changes.
- Don't rerun a build that just passed unless the code changed since.

## Deployment
- All tools share one version: bump `[workspace.package] version` in the root `Cargo.toml`
  (the flake reads it from there).
- Run `cargo update` (inside `nix develop`) and `nix flake update`; include both lock files.
- Commit and push to main.
