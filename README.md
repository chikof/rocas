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
- Recursive watching with configurable depth
- Cross-platform (Windows, macOS, Linux)

## Installation

### Standalone

Download the appropriate binary for your OS from the [Releases](https://github.com/chikof/rocas/releases) page, extract it, and run the executable.

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

rocas will run as a system service under a dedicated `rocas` user. You can override this with `services.rocas.user` and `services.rocas.group`.

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

| Option                    | Default       | Description                                                                   |
| ------------------------- | ------------- | ----------------------------------------------------------------------------- |
| `watcher.watchPath`       | `~/Downloads` | Primary directory to watch                                                    |
| `watcher.watchPaths`      | `[]`          | Watch multiple directories simultaneously (takes precedence over `watchPath`) |
| `watcher.recursive`       | `true`        | Watch subdirectories                                                          |
| `watcher.intervalMillis`  | `1000`        | Polling interval (ms)                                                         |
| `watcher.maxDepth`        | `null`        | Max recursion depth (`null` = unlimited)                                      |
| `watcher.debounceMs`      | `50`          | Collapse rapid events for the same path within this window (ms)               |
| `watcher.renameTimeoutMs` | `50`          | Wait this long for a rename pair before treating the source as a delete       |
| `misc.logLevel`           | `"info"`      | `trace` `debug` `info` `warn` `error`                                         |
| `misc.logFile`            | `null`        | Path to a log file; omit to log to stderr only                                |
| `misc.logMaxSizeMb`       | `10`          | Rotate the log file when it exceeds this size (MB); `0` disables rotation     |
| `misc.logKeepFiles`       | `3`           | Number of rotated log files to keep                                           |
| `misc.checkForUpdates`    | `true`        | Check for updates on startup                                                  |
| `rules`                   | `[]`          | List of `{ patterns, destination }` rules                                     |

> `misc.autoUpdate` is accepted for config compatibility but has no effect in Nix-managed installs — update rocas via `nix flake update` instead.

## Configuration

Rocas looks for its config file in the following locations depending on your OS:

- **Linux:** `~/.config/rocas/rocas.toml`
- **macOS:** `~/Library/Application Support/rocas/rocas.toml`
- **Windows:** `%APPDATA%\rocas\rocas.toml`
- **ANY:** `./rocas.toml` (current working directory)

```toml
[watcher]
watch_path = "/home/chiko/Downloads"  # directory to watch (single)
# watch_paths = ["/home/chiko/Downloads", "/home/chiko/Desktop"]  # watch multiple dirs simultaneously
recursive = true                      # watch subdirectories
interval_millis = 1000                # polling interval in milliseconds
max_depth = 1                         # max recursion depth: 0 = root only, 1 = root + one level, omit for unlimited
# debounce_ms = 50                    # collapse events within this window (ms); increase for slow/network drives
# rename_timeout_ms = 50              # wait this long for a rename pair before treating From as a delete (ms)

[misc]
log_level = "info"                    # trace | debug | info | warn | error
# log_file = "/var/log/rocas/rocas.log"  # omit to log to stderr only
# log_max_size_mb = 10               # rotate when file exceeds this size (MB); 0 = no rotation
# log_keep_files = 3                 # number of rotated files to keep
check_for_updates = true              # check for updates on startup
auto_update = false                   # auto update is ignored in Nix-managed installs

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

## Contributing

Contributions are welcome! For major changes, please open an issue first to discuss what you have in mind. Bug fixes and improvements can go straight to a pull request.

## Credits

- [@Aliwizzz](https://github.com/Aliwizzz) - Windows icon and idea
- [@prdgn52627](https://discord.com/users/726913634516860971) — Name and logo

## License

MIT — see [LICENSE](LICENSE) for details.
