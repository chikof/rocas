<div align="center">
<picture>
    <img alt="rocas_image" src="https://i.chiko.dev/u/rocas.png" width="300">
</picture>

**A simple file watcher that automatically moves files into folders based on rules you define.**

</div>

---

## Features

- Watch any directory for new files (defaults to Downloads)
- Move files based on glob patterns — extensions, names, catch-alls
- Never loses a file: an existing file at the destination is kept (renamed, skipped or replaced, your choice)
- Waits for downloads to finish, and ignores browsers' partial files (`*.part`, `*.crdownload`, ...)
- Cheap when idle: no polling unless you ask for it, no wake-ups while nothing changes
- Picks up config changes without a restart
- Recursive watching with configurable depth
- Cross-platform (Windows, macOS, Linux)

## Installation

### Standalone

Download the appropriate binary for your OS from the [Releases](https://github.com/chikof/rocas/releases) page, extract it, and run the executable. These binaries can [update themselves](#updates).

### NixOS

Add rocas to your flake inputs and import the module:

```nix
# flake.nix
inputs.rocas.url = "github:chikof/rocas";
```

Then in your NixOS configuration:

```nix
{ inputs, ... }: {
  imports = [ inputs.rocas.nixosModules.default ];

  services.rocas = {
    enable = true;
    watcher.watchPath = "/home/chiko/Downloads";
    rules = [
      { patterns = [ "*.pdf" ];         destination = "/home/chiko/Documents"; }
      { patterns = [ "*.jpg" "*.png" ]; destination = "/home/chiko/Pictures"; }
    ];
  };
}
```

rocas will run as a system service under a dedicated `rocas` user. You can override this with `services.rocas.user` and `services.rocas.group`. The generated config is installed at `/etc/rocas/rocas.toml` and the log file lives in `/var/lib/rocas`.

### Home Manager

Prefer this for per-user installs or if you want rocas to autostart on graphical login.

```nix
# flake.nix
inputs.rocas.url = "github:chikof/rocas";
```

```nix
{ inputs, ... }: {
  imports = [ inputs.rocas.homeManagerModules.default ];

  programs.rocas = {
    enable    = true;
    autostart = true;
    rules = [
      { patterns = [ "*.pdf" ];         destination = "~/Documents"; }
      { patterns = [ "*.jpg" "*.png" ]; destination = "~/Pictures"; }
    ];
  };
}
```

When `autostart` is enabled, rocas runs as a systemd user service tied to your graphical session. You can manage it manually with:

```sh
systemctl --user status rocas
systemctl --user restart rocas
```

#### Available Nix options

Every option maps to the key of the same name in the [configuration](#configuration) below, in camelCase (`watcher.settleMs` is `settle_ms`).

| Option                    | Default       | Description                                                                          |
| ------------------------- | ------------- | ------------------------------------------------------------------------------------ |
| `watcher.watchPath`       | `null`        | Primary directory to watch; `null` is your Downloads directory (NixOS module: `/home/<user>/Downloads`) |
| `watcher.watchPaths`      | `[]`          | Watch multiple directories simultaneously (takes precedence over `watchPath`)        |
| `watcher.recursive`       | `true`        | Watch subdirectories                                                                 |
| `watcher.maxDepth`        | `null`        | Max recursion depth (`null` = unlimited)                                             |
| `watcher.poll`            | `false`       | Poll instead of using OS notifications (network drives)                              |
| `watcher.intervalMillis`  | `1000`        | Polling interval (ms); only used when `poll` is `true`                               |
| `watcher.debounceMs`      | `50`          | Collapse rapid events for the same path within this window (ms)                      |
| `watcher.renameTimeoutMs` | `50`          | Wait this long for a rename pair before treating the source as a delete              |
| `watcher.settleMs`        | `1000`        | Move a file once its size and mtime have been unchanged this long (ms)               |
| `watcher.maxWaitSecs`     | `300`         | Move a file anyway after waiting this long for it to settle (s)                      |
| `watcher.ignore`          | `null`        | Filename globs never moved; `null` keeps the built-in list, a list replaces it        |
| `watcher.scanOnStart`     | `false`       | Organize files already present at startup                                            |
| `misc.onConflict`         | `"rename"`    | `rename`, `skip` or `overwrite` when the destination already has that name           |
| `misc.reload`             | `true`        | Reload the config when it changes                                                    |
| `misc.logLevel`           | `"info"`      | `trace` `debug` `info` `warn` `error`                                                |
| `misc.logFile`            | `null`        | Path to the log file; `null` is the default path (NixOS module: `/var/lib/rocas/rocas.log`) |
| `misc.logMaxSizeMb`       | `10`          | Rotate the log file when it exceeds this size (MB); `0` disables rotation            |
| `misc.logKeepFiles`       | `3`           | Number of rotated log files to keep                                                  |
| `rules`                   | `[]`          | List of `{ patterns, destination }` rules                                            |

> `misc.checkForUpdates` and `misc.autoUpdate` are accepted for config compatibility but have no effect in Nix-managed installs: the package is built without self-update. Update rocas via `nix flake update` instead.

## Configuration

Rocas looks for its config file in the following locations depending on your OS:

- **Linux:** `~/.config/rocas/rocas.toml`
- **macOS:** `~/Library/Application Support/rocas/rocas.toml`
- **Windows:** `%APPDATA%\rocas\rocas.toml`

The config file is optional: if it does not exist, rocas starts with the defaults below (watching your Downloads directory, no rules).

```toml
[watcher]
watch_path = "/home/chiko/Downloads"  # directory to watch (single); omit for your Downloads directory
# watch_paths = ["/home/chiko/Downloads", "/home/chiko/Desktop"]  # watch multiple dirs simultaneously
recursive = true                      # watch subdirectories
max_depth = 1                         # max recursion depth: 0 = root only, 1 = root + one level, omit for unlimited
# poll = false                        # poll instead of using OS notifications (network drives)
# interval_millis = 1000              # polling interval (ms); only used when poll = true
# debounce_ms = 50                    # collapse events within this window (ms); increase for slow/network drives
# rename_timeout_ms = 50              # wait this long for a rename pair before treating From as a delete (ms)
# settle_ms = 1000                    # move a file once its size and mtime are unchanged this long (ms)
# max_wait_secs = 300                 # move a file anyway after this long (s), with a warning
# ignore = ["*.part", "*.crdownload"] # filename globs never moved; replaces the built-in list
# scan_on_start = false               # organize files already present at startup

[misc]
log_level = "info"                    # trace | debug | info | warn | error
on_conflict = "rename"                # rename | skip | overwrite: when the destination already has that name
# reload = true                       # reload this file when it changes
# log_file = "/var/log/rocas/rocas.log"  # omit for the default path (next to this file)
# log_max_size_mb = 10               # rotate when file exceeds this size (MB); 0 = no rotation
# log_keep_files = 3                 # number of rotated files to keep
check_for_updates = true              # check for updates on startup (builds with self-update only)
auto_update = false                   # install updates automatically (builds with self-update only)

[[rules]]
patterns = ["*.pdf", "*.docx"]
destination = "/home/chiko/Documents"

[[rules]]
patterns = ["*.jpg", "*.png", "*.gif"]
destination = "/home/chiko/Pictures"

[[rules]]
patterns = ["*"]                      # catch-all, matches anything not covered above
destination = "/home/chiko/Other"
```

The first rule whose pattern matches a file wins. A pattern without `/` is matched against the file name, a pattern with `/` against the whole path.

### Moving files

- **Settling.** A new file is moved only once its size and modification time have not changed for `settle_ms` (1 second by default), so a download that is still being written is left alone. If a file keeps changing, rocas moves it anyway after `max_wait_secs` (5 minutes) and logs a warning. Waiting never blocks other files or the watcher.
- **Ignored files.** Names matching `ignore` are never moved. The built-in list covers partial downloads and lock files: `*.part`, `*.crdownload`, `*.download`, `*.partial`, `*.tmp`, `*.opdownload` and `.~lock.*`. Setting `ignore` replaces the whole list. A browser's finished file is moved after it renames the download to its final name.
- **Conflicts.** rocas never overwrites a file by default. When the destination already has a file with that name, `on_conflict` decides: `rename` keeps both and names the incoming file `name (1).ext`, `name (2).ext`, and so on; `skip` leaves the incoming file where it is and logs it; `overwrite` replaces the existing file.
- **Destinations inside a watched directory.** With `recursive = true` a rule may send files into a subdirectory of the watched directory; files that land there are not moved again.
- **Scan on start.** By default rocas only handles files that appear while it runs. Set `scan_on_start = true` to organize the files already in the watched directories when it starts.
- **Cross-filesystem moves.** A destination on another drive is copied and then the original removed.

### Reloading the configuration

With `reload = true` (the default), rocas reloads `rocas.toml` shortly after you save it. It listens for file-system events on the config directory, so an idle rocas does not poll the file; with `poll = true` that directory is polled like the watched ones. Editors that save by replacing the file are handled, and so is creating `rocas.toml` after rocas has started, even when its directory does not exist yet: rocas then watches the nearest existing parent directory (not recursively) until the directory appears, and moves the watch down to it. New rules, destinations and the settings above apply without a restart. If the new file is invalid, rocas logs the error and keeps running with the previous configuration.

### Updates

The binaries on the Releases page include self-update: with `check_for_updates = true` rocas looks for a newer release on startup, and with `auto_update = true` it installs it. Builds from package managers, including the Nix package, are compiled without the `self-update` cargo feature; there `check_for_updates` and `auto_update` are ignored and your package manager does the updating. To build without it yourself: `cargo build --release --no-default-features`.

### Command line and environment

Every option can also be set with a command-line flag or an environment variable. Precedence, lowest to highest: config file, environment, command line.

- **Flags** are the key path in kebab-case: `--watcher.recursive`, `--watcher.max-depth 2`, `--misc.log-level debug`. On/off options take no value: `--watcher.recursive` turns one on, `--no-misc.check-for-updates` (or `--misc.check-for-updates=false`) turns one off.
- **Environment variables** are `ROCAS_` plus the key path uppercased with `_`: `ROCAS_WATCHER_RECURSIVE=true`, `ROCAS_MISC_LOG_LEVEL=debug`. Lists are comma-separated: `ROCAS_WATCHER_WATCH_PATHS=/a,/b`.
- `rocas --help` lists every option with its default and environment variable name.
- `rocas --version` (or `-V`) prints the version.

### Starting on boot

```sh
rocas --boot   # or: rocas -b
```

Toggles launching rocas when you log in (run it again to turn it off), then keeps running as usual. `--boot` is an action, not a setting: it is never read from the config file or the environment.

## Upgrading from 1.x

- **`rocas boot` is now `rocas --boot`** (or `-b`).
- **Existing files are never overwritten by default.** 1.x replaced a file of the same name in the destination. Now the incoming file is renamed (`name (1).ext`). Set `on_conflict = "overwrite"` in `[misc]` for the old behaviour, or `"skip"` to leave the incoming file alone.
- **`interval_millis` only applies with `poll = true`.** It used to have no effect on native watching. rocas now uses OS notifications and does not wake up while idle unless you enable `poll`.
- **Partial downloads are ignored.** Files matching the default `ignore` list are not moved. If you relied on moving one of those names, set `ignore` to your own list.
- **The config reloads on change.** Set `reload = false` to disable that.
- **rocas never moves its own config file.** A rule such as `*.toml` over a watch directory that contains `rocas.toml` (for example `~/.config`, watched recursively) skips that one file, however the path is spelled, while other files still match. Editing it triggers one reload.
- **`watch_path` is optional** and defaults to your Downloads directory; `log_file` defaults to `rocas.log` next to the config file.
- **Nix:** `programs.rocas.watcher.watchPath` defaults to `null` (your Downloads directory).
- **NixOS service:** the module writes the config to `/etc/rocas/rocas.toml` and runs rocas with `XDG_CONFIG_HOME=/etc`, so rocas reads it from there instead of a per-user config directory. The service has `StateDirectory=rocas`, and `services.rocas.misc.logFile` defaults to `/var/lib/rocas/rocas.log` rather than a file next to the config. Setting it to `null` gives rocas's own default (next to the config), which the service's read-only `/etc` does not allow.


## Contributing

Contributions are welcome! For major changes, please open an issue first to discuss what you have in mind. Bug fixes and improvements can go straight to a pull request.

### Development

The dev shell is [devenv](https://devenv.sh): run `devenv shell` (or let direnv load it), then `check` runs the same gate as CI: formatting, clippy, tests, docs, `cargo deny`, shellcheck and a spell check. The nix package and the NixOS and home-manager modules still come from `flake.nix`.

## Credits

- [@Aliwizzz](https://github.com/Aliwizzz) - Logo/icon and idea
- [@prdgn52627](https://discord.com/users/726913634516860971) — Name and banner

## License

MIT — see [LICENSE](LICENSE) for details.
