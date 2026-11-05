# typort

`typort` shows the TCP ports your user is listening on (Vite, Next.js, `python -m
http.server`, Rails, anything) in the tray. Open one in the browser, copy its URL, or stop it.

## Features

- Tray menu with one entry per port: `:5173  My App · vite · my-app`. The first part is the
  page's `<title>` (fetched from the port, for anything that speaks HTTP). Names come from the command
  line, so a Node or Python server shows as `vite`, `next` or `http.server` rather than
  `node`, and the project is the server's working directory.
- Each entry shows the URL (click it to open it in the browser), **Copy URL**, the PIDs and
  command, directory, and whether it is reachable from other machines (`localhost only` /
  `all interfaces`).
- **Stop** (SIGTERM) and **Force kill** (SIGKILL), plus **Stop all**.
- The icon only shows while something is listening, and is yellow then.
- CLI for scripts and keybindings, and `status --follow` for bars.
- Only reads `/proc`: no root, no `ss` or `lsof`.

Only your own processes are listed. A hardened service (e.g. a systemd unit that blocks
access to its file descriptors) still shows its port, with the process as `unknown`.

## Commands

```sh
typort run                 # tray applet (the default)
typort list                # PORT PID PROCESS REACH URL TITLE
typort list --json         # or --all to include ignored ports
typort open 5173           # in the browser
typort url 5173            # print the URL
typort kill 5173           # SIGTERM; --force for SIGKILL
typort status [--follow] [--format plain|json|polybar]
typort validate-config
```

## Configuration

`./typort.yaml` or `~/.config/typort/config.yaml`. Every key is optional. These are the
defaults:

```yaml
ignore_ports: []              # never shown
ignore_processes: [kdeconnectd]  # by name as `typort list` prints it; case-insensitive
https_ports: []               # opened with https:// (no title is fetched)
# browser: [firefox, --new-tab]  # URL is appended; default xdg-open
refresh_seconds: 2
```

With Home Manager:

```nix
programs.typort = {
  enable = true;
  settings.ignore_ports = [8384 22000]; # Syncthing (its process is hidden, so match by port)
};
```
