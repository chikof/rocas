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
        recursive = cfg.watcher.recursive;
        interval_millis = cfg.watcher.intervalMillis;
        max_depth = cfg.watcher.maxDepth;
      };
      misc = {
        check_for_updates = cfg.misc.checkForUpdates;
        auto_update = cfg.misc.autoUpdate;
        log_level = cfg.misc.logLevel;
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
        type = lib.types.str;
        default = config.home.homeDirectory + "/Downloads";
        description = "Directory to watch.";
      };
      recursive = lib.mkOption {
        type = lib.types.bool;
        default = true;
      };
      intervalMillis = lib.mkOption {
        type = lib.types.int;
        default = 1000;
      };
      maxDepth = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
      };
    };

    misc = {
      checkForUpdates = lib.mkOption {
        type = lib.types.bool;
        default = true;
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
