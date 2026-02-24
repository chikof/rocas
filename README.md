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

| Option                   | Default       | Description                               |
| ------------------------ | ------------- | ----------------------------------------- |
| `watcher.watchPath`      | `~/Downloads` | Directory to watch                        |
| `watcher.recursive`      | `true`        | Watch subdirectories                      |
| `watcher.intervalMillis` | `1000`        | Polling interval (ms)                     |
| `watcher.maxDepth`       | `null`        | Max recursion depth                       |
| `misc.logLevel`          | `"info"`      | `trace` `debug` `info` `warn` `error`     |
| `misc.checkForUpdates`   | `true`        | Check for updates on startup              |
| `rules`                  | `[]`          | List of `{ patterns, destination }` rules |

> `misc.autoUpdate` is accepted for config compatibility but has no effect in Nix-managed installs — update rocas via `nix flake update` instead.

## Configuration

Rocas looks for its config file in the following locations depending on your OS:

- **Linux:** `~/.config/rocas/rocas.toml`
- **macOS:** `~/Library/Application Support/rocas/rocas.toml`
- **Windows:** `%APPDATA%\rocas\rocas.toml`
- **ANY:** `./rocas.toml` (current working directory)

```toml
[watcher]
watch_path = "/home/chiko/Downloads"  # directory to watch
recursive = true                      # watch subdirectories
interval_millis = 1000                # polling interval in milliseconds
max_depth = 0                         # max recursion depth (0 for unlimited)

[misc]
log_level = "info"                    # trace | debug | info | warn | error
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

- [@prdgn52627](https://discord.com/users/726913634516860971) — Name and logo

## License

MIT — see [LICENSE](LICENSE) for details.
