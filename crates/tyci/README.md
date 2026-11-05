# tyci

GitHub in your tray: CI runs that are running right now, runs that just finished, and
recently opened pull requests. Click any of them to open it in the browser. Everything comes
from the [`gh` CLI](https://cli.github.com), so it uses your existing `gh auth login` and
needs no token of its own.

- Tray icon turns yellow while runs are going and red when a recent run failed
- Notification when a run it saw running finishes (click to open it)
- Optional notification for new PRs
- Whitelist and blacklist of repos, with `owner` and `*` globs
- `tyci list` and `tyci status --follow --format polybar` for the terminal and bars

## Requirements

- `gh`, logged in (`gh auth login`, or `GH_TOKEN` in the service's environment)
- `xdg-open`, `notify-send`
- A system tray (eww, waybar, KDE, …)

## Usage

```sh
tyci run                                # tray applet (the default)
tyci list [--json]                      # fetch once and print
tyci status --follow --format polybar   # counts for bars; empty when idle
tyci validate-config
```

Middle-clicking the tray icon (if your tray host forwards it) refreshes.

## Which repos are checked

PRs come from a GitHub search (`pull_requests.query`). CI runs are checked in:

1. exact `owner/name` entries in `repos.include`,
2. repos with one of the listed open PRs,
3. your `repos.discover` most recently pushed repos — or, when `repos.include` names owners
   (`my-org` or `my-org/*`), those owners' most recently pushed repos instead.

Then every repo (PRs and runs) must pass the filters: if `repos.include` is non-empty only
matching repos are shown, and anything matching `repos.exclude` is always hidden.
Patterns are matched case-insensitively; `*` matches anything, and a pattern without `/`
matches the owner.

## Config

`./tyci.yaml` or `~/.config/tyci/config.yaml` (or `--config`). Every key is optional:

```yaml
poll_seconds: 60          # at least 10
repos:
  include: []             # whitelist, e.g. [tylergets, my-org/api-*, other/repo]
  exclude: []             # blacklist, wins over include, e.g. [tylergets/old-thing]
  discover: 15            # recently pushed repos checked for runs (0: only include + PR repos)
pull_requests:
  query: "involves:@me"   # search qualifiers: author:@me, review-requested:@me, user:my-org, ...
  max_age_days: 14        # only PRs opened this recently (0: no limit)
  limit: 10
  drafts: true
runs:
  recent_minutes: 60      # show runs that finished this recently (0: hide finished runs)
  finished_limit: 8
  per_repo: 10            # runs fetched per repo per refresh
notifications:
  on_finish: true         # when a run tyci saw running finishes
  only_failures: false
  on_new_pr: false
```

Each refresh runs one `gh search`, one `gh repo list` per discovered owner and one
`gh run list` per repo, so keep `poll_seconds` × repos reasonable for GitHub's rate limits.

## Home Manager

```nix
programs.tyci = {
  enable = true;
  settings.repos.exclude = ["tylergets/old-thing"];
};
```

The systemd service needs gh's credentials. `gh auth login` stores them in
`~/.config/gh/hosts.yml` (or the keyring), which works as is; if you rely on `GH_TOKEN`, set it
for the service too, e.g. `systemd.user.services.tyci.Service.EnvironmentFile`.
