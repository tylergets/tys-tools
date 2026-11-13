# tyquote

A personal quote collection, ready to sync with Syncthing. No background service is
needed: commands read the collection directly.

```sh
tyquote add "The secret of getting ahead is getting started." --author "Unknown"
cat quote.txt | tyquote add - --author "Author"
tyquote random
tyquote latest
tyquote today
tyquote --directory ~/Quotes today
tyquote --config ~/other-quotes.yaml today
tyquote validate-config
```

Quotes live in `$XDG_DATA_HOME/tyquote/quotes` (normally
`~/.local/share/tyquote/quotes`). Sync that folder between machines. Each addition
creates an independently named JSON file atomically, so concurrent additions do not
overwrite each other. Ignore `.tyquote-*.tmp` in Syncthing. Edit or remove a JSON
file to edit or delete a quote; simultaneous edits to the same quote use Syncthing's
normal conflict handling. No database, index, or machine-specific state is synced.

Files contain `text`, optional `author`, and `created_at` (an RFC3339 timestamp).
`latest` uses that timestamp, not filesystem modification times. `today` hashes the
calendar date and filenames: machines with the same collection and date show
the same quote, regardless of directory order. Syncing additions or deletions can
change today's selection; local dates in different time zones can differ. Set
`day_boundary: utc` on all machines for a shared midnight. `random` chooses
anew on each invocation. An empty collection or invalid quote returns an error on
stderr and exits nonzero.

## Configuration

Start with [tyquote.example.yaml](tyquote.example.yaml).

Reads `./tyquote.yaml`, then `$XDG_CONFIG_HOME/tyquote/config.yaml` (normally
`~/.config/tyquote/config.yaml`). `-c` / `--config` selects a specific file instead.
No config file is required; every setting is optional. Unknown keys and invalid
values are rejected. `tyquote validate-config` checks settings without accessing
the quote collection.

```yaml
directory: null         # default: $XDG_DATA_HOME/tyquote/quotes
default_author: null    # used by add when --author is omitted
show_author: true       # append attribution when printing a quote
author_prefix: "— "     # prefix on the separate author line
day_boundary: local    # local | utc: when today's quote changes
```

`directory` accepts `~`; relative paths are relative to the working directory.
`--directory` overrides it. `--author` overrides `default_author`; `--author ""`
explicitly stores no author. Display settings apply to `random`, `latest`, and
`today` without modifying the stored quotes. Sync the quotes folder, not the config,
unless you also want the same settings on every machine.

Home Manager writes the YAML file for you:

```nix
imports = [ inputs.tys-tools.homeManagerModules.tyquote ];
programs.tyquote = {
  enable = true;
  settings = {
    directory = "~/.local/share/tyquote/quotes";
    default_author = null;
    show_author = true;
    author_prefix = "— ";
    day_boundary = "utc";
  };
};
```

## tylock integration

For tylock, set `quote_command: [tyquote, today]` in its YAML configuration, or:

```nix
programs.tylock.settings.quote_command = [
  "${config.programs.tyquote.package}/bin/tyquote" "today"
];
```
