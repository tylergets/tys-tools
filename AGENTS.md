# Agent Instructions for `tys-tools`

Monorepo for the ty* X11 desktop tools. One Cargo workspace, one Nix flake.

## Layout
- `crates/ty-common` — shared code: tray applet (`tray`), single-instance lock (`lock`),
  config discovery/loading (`config`), notifications (`notify`), `status` CLI helpers.
  If two tools need the same thing, it goes here, not in both.
- `crates/<tool>` — tyboard, tykey, tycapture, tyshot, tyspeak.
- `nix/packages.nix` — one crane build of the whole workspace + a thin wrapper per tool
  (runtime PATH / GTK env). Add a tool's runtime programs to its `runtimeDeps` there.
- `nix/modules/lib.nix` — shared Home Manager module builder; `nix/modules/<tool>.nix` per tool.

## Environment
- Use `nix develop`; `cargo` is available inside it.

## Required Workflow
- When you create a new file, stage it immediately with `git add <path>` (flakes only see tracked files).
- When a tool's config format changes, update its README and, if it has typed options,
  `nix/modules/<tool>.nix`.
- Shared dependencies are declared once in the root `Cargo.toml` `[workspace.dependencies]`;
  crates use `dep.workspace = true`.

## Validation
- `nix flake check` builds every tool and runs tests, clippy and rustfmt. Run it (or at least
  `nix build .#<tool>` plus `cargo test -p <tool>`) after changes that affect program behavior.
  Not required for docs-only changes.

## Deployment
- All tools share one version: bump `[workspace.package] version` in the root `Cargo.toml`
  (the flake reads it from there).
- Run `cargo update` (inside `nix develop`) and `nix flake update`; include both lock files.
- Commit and push to main.
