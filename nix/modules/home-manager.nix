# Home Manager module for rocas — per-user install with a systemd user service
# for autostart. This replaces .desktop autostart entries or launchd plists.
#
# Usage in a Home Manager configuration:
#
#   programs.rocas = {
#     enable    = true;
#     autostart = true;
#     watcher.watchPath = "/home/alice/Downloads";
#     rules = [
#       { patterns = [ "*.pdf" ]; destination = "/home/alice/Documents"; }
#     ];
#   };
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.rocas;

  rocasPkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;

  tomlFormat = pkgs.formats.toml { };
  configFile = tomlFormat.generate "rocas.toml" (
    lib.filterAttrsRecursive (_: v: v != null) {
      watcher = {
        watch_path = cfg.watcher.watchPath;
        watch_paths = cfg.watcher.watchPaths;
        recursive = cfg.watcher.recursive;
        max_depth = cfg.watcher.maxDepth;
        poll = cfg.watcher.poll;
        interval_millis = cfg.watcher.intervalMillis;
        debounce_ms = cfg.watcher.debounceMs;
        rename_timeout_ms = cfg.watcher.renameTimeoutMs;
        settle_ms = cfg.watcher.settleMs;
        max_wait_secs = cfg.watcher.maxWaitSecs;
        ignore = cfg.watcher.ignore;
        scan_on_start = cfg.watcher.scanOnStart;
      };
      misc = {
        check_for_updates = cfg.misc.checkForUpdates;
        auto_update = cfg.misc.autoUpdate;
        log_level = cfg.misc.logLevel;
        log_file = cfg.misc.logFile;
        log_max_size_mb = cfg.misc.logMaxSizeMb;
        log_keep_files = cfg.misc.logKeepFiles;
        on_conflict = cfg.misc.onConflict;
        reload = cfg.misc.reload;
      };
      rules = map (r: {
        patterns = r.patterns;
        destination = r.destination;
      }) cfg.rules;
    }
  );
in
{
  options.programs.rocas = {
    enable = lib.mkEnableOption "rocas file organizer";

    package = lib.mkOption {
      type = lib.types.package;
      default = rocasPkg;
      description = "The rocas package to use.";
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Start rocas automatically on login via a systemd user service.
        This is the Nix-native replacement for .desktop autostart entries.
      '';
    };

    watcher = {
      watchPath = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Directory to watch. null watches your Downloads directory.";
      };
      watchPaths = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = ''
          Additional directories to watch simultaneously. When non-empty,
          takes precedence over watchPath. All paths share the same
          recursive, maxDepth, and timing settings.
        '';
      };
      recursive = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Watch subdirectories recursively.";
      };
      maxDepth = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "Maximum recursion depth (null = unlimited).";
      };
      poll = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Poll for changes instead of using OS notifications. Turn this on
          for network drives, where the OS does not report changes.
        '';
      };
      intervalMillis = lib.mkOption {
        type = lib.types.int;
        default = 1000;
        description = "Polling interval in milliseconds. Only used when poll is true.";
      };
      debounceMs = lib.mkOption {
        type = lib.types.int;
        default = 50;
        description = ''
          Events within this window (ms) for the same path are collapsed
          into one. Increase on slow network drives or when batch copy
          tools fire many rapid events.
        '';
      };
      renameTimeoutMs = lib.mkOption {
        type = lib.types.int;
        default = 50;
        description = ''
          How long to wait (ms) for a rename "To" counterpart before
          treating the "From" as a plain delete.
        '';
      };
      settleMs = lib.mkOption {
        type = lib.types.int;
        default = 1000;
        description = ''
          Move a file once its size and modification time have been
          unchanged for this long (ms).
        '';
      };
      maxWaitSecs = lib.mkOption {
        type = lib.types.int;
        default = 300;
        description = ''
          Move a file anyway after waiting this long (s) for it to settle.
          A warning is logged when this limit is hit.
        '';
      };
      ignore = lib.mkOption {
        type = lib.types.nullOr (lib.types.listOf lib.types.str);
        default = null;
        example = [
          "*.part"
          "*.crdownload"
        ];
        description = ''
          Filename globs that are never moved. null keeps rocas's built-in
          list (`*.part`, `*.crdownload`, `*.download`, `*.partial`,
          `*.tmp`, `*.opdownload`, `.~lock.*`); a list replaces it.
        '';
      };
      scanOnStart = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Organize files already present in the watched directories at startup.";
      };
    };

    misc = {
      checkForUpdates = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "No-op in Nix-managed installs — update via your flake instead.";
      };
      autoUpdate = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "No-op in Nix-managed installs — update via your flake instead.";
      };
      logLevel = lib.mkOption {
        type = lib.types.enum [
          "trace"
          "debug"
          "info"
          "warn"
          "error"
        ];
        default = "info";
        description = "Log verbosity.";
      };
      logFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          Path to the log file. null uses rocas's default,
          ~/.config/rocas/rocas.log on Linux.
          Example: "${config.home.homeDirectory}/.local/share/rocas/rocas.log"
        '';
      };
      logMaxSizeMb = lib.mkOption {
        type = lib.types.int;
        default = 10;
        description = "Rotate the log file when it exceeds this size in megabytes. 0 disables rotation.";
      };
      logKeepFiles = lib.mkOption {
        type = lib.types.int;
        default = 3;
        description = "Number of rotated log files to keep alongside the active log.";
      };
      onConflict = lib.mkOption {
        type = lib.types.enum [
          "rename"
          "skip"
          "overwrite"
        ];
        default = "rename";
        description = ''
          What to do when the destination already has a file with that
          name: `rename` keeps both ("name (1).ext"), `skip` leaves the
          incoming file where it is, `overwrite` replaces the existing file.
        '';
      };
      reload = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Reload rocas.toml when it changes.";
      };
    };

    rules = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            patterns = lib.mkOption { type = lib.types.listOf lib.types.str; };
            destination = lib.mkOption { type = lib.types.str; };
          };
        }
      );
      default = [ ];
      description = "File matching rules.";
      example = lib.literalExpression ''
        [
          { patterns = [ "*.pdf" ]; destination = "~/Documents"; }
          { patterns = [ "*.jpg" "*.png" ]; destination = "~/Pictures"; }
        ]
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    # Writes to ~/.config/rocas/rocas.toml — rocas reads this by default.
    xdg.configFile."rocas/rocas.toml".source = configFile;

    # Systemd user service — starts on graphical login, restarts on failure.
    # To manage manually: systemctl --user {start,stop,status} rocas
    systemd.user.services.rocas = lib.mkIf cfg.autostart {
      Unit = {
        Description = "rocas file organizer";
        After = [ "graphical-session.target" ];
        PartOf = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/rocas";
        Restart = "on-failure";
        RestartSec = "5s";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
