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
```

Quotes live in `$XDG_DATA_HOME/tyquote/quotes` (normally
`~/.local/share/tyquote/quotes`). Sync that folder between machines. Each addition
creates an independently named JSON file atomically, so concurrent additions do not
overwrite each other. Ignore `.tyquote-*.tmp` in Syncthing. Edit or remove a JSON
file to edit or delete a quote; simultaneous edits to the same quote use Syncthing's
normal conflict handling. No database, index, or machine-specific state is synced.

Files contain `text`, optional `author`, and `created_at` (an RFC3339 timestamp).
`latest` uses that timestamp, not filesystem modification times. `today` hashes the
local calendar date and filenames: machines with the same collection and date show
the same quote, regardless of directory order. Syncing additions or deletions can
change today's selection; dates in different time zones can differ. `random` chooses
anew on each invocation. An empty collection or invalid quote returns an error on
stderr and exits nonzero.

Install with `programs.tyquote.enable = true` using this flake's Home Manager module.
For tylock, set `quote_command: [tyquote, today]` in its YAML configuration, or:

```nix
programs.tylock.settings.quote_command = [
  "${config.programs.tyquote.package}/bin/tyquote" "today"
];
```
