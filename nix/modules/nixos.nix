# NixOS module for rocas — system-wide install with a systemd system service.
# Usage in a NixOS configuration:
#
#   services.rocas = {
#     enable = true;
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
  cfg = config.services.rocas;

  rocasPkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;

  tomlFormat = pkgs.formats.toml { };
  configFile = tomlFormat.generate "rocas.toml" (
    lib.filterAttrsRecursive (_: v: v != null) {
      watcher = {
        watch_path = cfg.watcher.watchPath;
        watch_paths = cfg.watcher.watchPaths;
        recursive = cfg.watcher.recursive;
        interval_millis = cfg.watcher.intervalMillis;
        max_depth = cfg.watcher.maxDepth;
        debounce_ms = cfg.watcher.debounceMs;
        rename_timeout_ms = cfg.watcher.renameTimeoutMs;
      };
      misc = {
        check_for_updates = cfg.misc.checkForUpdates;
        auto_update = cfg.misc.autoUpdate;
        log_level = cfg.misc.logLevel;
        log_file = cfg.misc.logFile;
        log_max_size_mb = cfg.misc.logMaxSizeMb;
        log_keep_files = cfg.misc.logKeepFiles;
      };
      rules = map (r: {
        patterns = r.patterns;
        destination = r.destination;
      }) cfg.rules;
    }
  );
in
{
  options.services.rocas = {
    enable = lib.mkEnableOption "rocas file organizer";

    package = lib.mkOption {
      type = lib.types.package;
      default = rocasPkg;
      description = "The rocas package to use.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "rocas";
      description = "User account under which rocas runs.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "rocas";
      description = "Group under which rocas runs.";
    };

    watcher = {
      watchPath = lib.mkOption {
        type = lib.types.str;
        default = "/home/${cfg.user}/Downloads";
        description = "Directory to watch.";
      };
      recursive = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Watch subdirectories recursively.";
      };
      intervalMillis = lib.mkOption {
        type = lib.types.int;
        default = 1000;
        description = "Polling interval in milliseconds.";
      };
      maxDepth = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "Maximum recursion depth (null = unlimited).";
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
    };

    misc = {
      checkForUpdates = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Check for updates on startup.";
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
          Path to the log file. null disables file logging (stderr only).
          Example: "/var/log/rocas/rocas.log"
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
    };

    rules = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            patterns = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              description = "Glob patterns to match.";
            };
            destination = lib.mkOption {
              type = lib.types.str;
              description = "Destination directory.";
            };
          };
        }
      );
      default = [ ];
      description = "File matching rules.";
      example = lib.literalExpression ''
        [
          { patterns = [ "*.pdf" ]; destination = "/home/alice/Documents"; }
          { patterns = [ "*.jpg" "*.png" ]; destination = "/home/alice/Pictures"; }
        ]
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = lib.mkIf (cfg.user == "rocas") {
      isSystemUser = true;
      group = cfg.group;
      description = "rocas service user";
    };
    users.groups.${cfg.group} = lib.mkIf (cfg.group == "rocas") { };

    systemd.services.rocas = {
      description = "rocas file organizer";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/rocas --config ${configFile}";
        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = "5s";
        # Hardening
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        ReadWritePaths =
          cfg.watcher.watchPaths ++ [ cfg.watcher.watchPath ] ++ (map (r: r.destination) cfg.rules);
        PrivateTmp = true;
      };
    };
  };
}
