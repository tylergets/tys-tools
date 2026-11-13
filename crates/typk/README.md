# typk

`typk` is a polkit authentication agent: the window that asks for your password when a
program needs an admin action (`pkexec`, mounting a disk, changing a system setting, ...).
Run it in your session and polkit sends those requests to it.

## Features

- Password or fingerprint, whichever your PAM setup for `polkit-1` asks for. With a
  fingerprint reader the prompt shows a fingerprint and waits for a touch; the password
  field is behind **Use password**, and what you type there is sent as soon as PAM asks
  for it.
- Shows what PAM says (`Place your finger on the fingerprint reader`, `Authentication
  failure`), with a few attempts before giving up.
- A chooser when several accounts may authenticate (a `wheel` group, say), with you first.
- The action id and polkit's details behind **Details**.
- `typk verify`: a command for scripts and AI agents to ask you to confirm it's you,
  with the answer as an exit code. The password or finger never passes through them.
- The asking program is the headline, with its icon when it has a desktop entry, and a
  quiet **Kill <app>** button ends it and refuses.
- Optionally darkens the rest of the screen while the prompt is open (`dim_screen`).
- Centered on the screen, always on top and focused: a layer-shell overlay on Wayland,
  a floating sticky dialog on X11 (it opens on the current workspace and follows you if
  you switch).
- Nothing polls: idle, it sleeps on the system bus socket.

## Requirements

- polkit (`polkitd` on the system bus) and systemd-logind (or elogind). On polkit 125 and
  later the helper is a socket-activated service; before that, polkit's setuid
  `polkit-agent-helper-1` is run.
- A graphical session: typk must run inside the login session polkit attributes you to. A
  systemd user service is fine; polkit uses your display session then.
- No other polkit agent in the session (polkit allows one; typk fails to start otherwise).
- Fingerprint: `pam_fprintd` in the `polkit-1` PAM service. On NixOS, `services.fprintd.enable`
  turns it on for every PAM service, including this one.

## Commands

```sh
typk run                                  # the agent (the default)
typk verify --reason "Deploy to prod?"    # prompt; exit 0 if you confirmed
typk verify --app Claude --reason "..."   # name the asker (default: the program that ran typk)
typk verify --action org.freedesktop.login1.reboot   # any polkit action
typk validate-config
```

## Confirming from scripts

`typk verify` asks polkit for typk's own action, `tools.tys.typk.verify`, which needs you
to authenticate as yourself (`auth_self`). The running agent shows the prompt, headed by
the asking program (`--app`, or the nearest non-shell ancestor of `typk verify`, so a
script run by Claude Code shows "Claude") with the `--reason` text under polkit's message.
`verify` exits 0 when you authenticated, 1 when you refused or cancelled (or the prompt's
Kill button ended the asker), 2 on an error. The script never sees the password:

```sh
typk verify --reason "Claude wants to run the migration" && ./migrate
```

polkitd must know the action, so the policy file has to be installed system-wide: on NixOS
put `pkgs.typk` (from the overlay) in `environment.systemPackages`; elsewhere copy
`typk.policy` to `/usr/share/polkit-1/actions/tools.tys.typk.policy`. Until then `verify`
falls back to `org.freedesktop.policykit.exec`, which asks for an admin instead, with a
warning.

## Configuration

`./typk.yaml` or `~/.config/typk/config.yaml`. Every key is optional. These are the
defaults:

```yaml
max_attempts: 3         # wrong passwords (or fingers) before the request is refused
show_details: false     # true: action id and details always visible, not behind "Details"
dim_screen: false       # true: darken the rest of the screen while the prompt is open
# helper: /usr/lib/polkit-1/polkit-agent-helper-1   # if polkit's helper is somewhere unusual
```

## Home Manager

```nix
programs.typk.enable = true;    # runs the agent as a user service in graphical-session.target
```

On NixOS, `security.polkit.enable = true` provides polkit and its helper.

## How it works

polkit's agent protocol: typk registers itself on the system bus for your login session,
and polkitd calls it with the action, a message and the identities allowed to authenticate.
Checking the password is not done by typk: it talks to polkit's own `polkit-agent-helper-1`,
which runs the `polkit-1` PAM stack as root and reports success to polkitd directly. typk
only relays PAM's prompts and your answers, like every other polkit agent.
