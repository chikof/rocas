# Changelog

All notable changes to this project will be documented in this file.

## [2.0.0] - 2026-11-03

### Bug Fixes
- **test:** `DirWatcher` requires a `WatcherConfig` reference

- **deps:** Update rust crate self_update to 0.43.0 (#9)

- **deps:** Update forgeconf digest to b4b2e01 (#7)

- `self_update` version 0.43 is broken, downgraded to 0.42 in the meantime

- **deps:** Update rust crate self_update to 0.44.0 (#13)

- Just handle the boot subcommand

- Keep --boot out of the config layers

- Write log lines unbuffered and rotate next to the log file

- Print the banner in one write and ignore stderr errors

- Never organise rocas's own config file

- Watch for the config directory when it does not exist yet

- Remove the source only once the destination is durable

- Match globs without exponential backtracking

- Run the update check after watching has started

- Build cleanly on Windows

- Install automatic updates without a confirmation prompt


### CI
- Run the check gate and MSRV build

- Lint without default features and build the release binary

- Lint the Windows and macOS builds

- Run unit tests and a release build on Windows and macOS

- Build portable release binaries, including Apple Silicon


### Chores
- **deps:** Update rust crate libc to v0.2.183 (#8)

- Update `self_update` i just had to specify the reqwest feature

- **deps:** Update softprops/action-gh-release action to v3 (#14)

- **deps:** Update rust crate libc to v0.2.186 (#12)

- **deps:** Update rust crate rustc-hash to v2.1.2 (#11)

- **deps:** Update lockfile to clear security advisories


### Documentation
- Document the 2.0 options, conflict handling, reload and upgrading from 1.x

- Note the NixOS module changes and the own config file rule

- Describe the setup wizard


### Features
- Add cli flags (clap) for configuration

- Config help arg

- Load configuration with the forgeconf derive API ⚠️ BREAKING

- Make self-update an optional feature

- Pre-parse --boot and --version with the config's flag shapes

- Never overwrite when moving into a destination

- Drive every file through one non-blocking event loop ⚠️ BREAKING

- **nix:** Options for polling, settling, ignore, scan on start, conflicts and reload

- Reload the configuration on file events instead of polling

- Turn autostart on without toggling it

- Add a first-run setup wizard


### Miscellaneous
- Develop with devenv and one check gate

- **nix:** Take name and version from rocas/Cargo.toml and build without self-update

- Depend on forgeconf 1.0.0 from crates.io

- List every commit in the changelog, not only rocas/ ones

- **deps:** Update dirs to 7 and self_update to 1.3


### Testing
- Run the binary against temporary directories

- Cover the poll backend end to end

- Move a file across filesystems end to end

- Compare wizard paths the way each platform prints them


## [1.4.0] - 2026-03-25

### Bug Fixes
- Files used to get corrupted if they were being downloaded + few optimizations


### Chores
- **deps:** Update github artifact actions (#5)

- Release


### Features
- Auto restart rocas when after downloading update


## [1.3.2] - 2026-03-02

### Bug Fixes
- **deps:** Update forgeconf digest to a5f2c91 (#6)


### Chores
- **deps:** Update actions/cache action to v5 (#2)

- **deps:** Update actions/checkout action to v6 (#3)

- Remove unused changelog md

- Remove redundant /SUBSYSTEM:WINDOWS linker flag from .cargo/config.toml

- Release


## [1.3.1] - 2026-03-01

### Bug Fixes
- Initialise logger only once to avoid SetLoggerError


### Chores
- Release


### Features
- Print ASCII art banner with categorised config on startup


## [1.3.0] - 2026-03-01

### Chores
- Push changelog

- Release


### Features
- Add custom file logger with rotation

- Hide console window on Windows


## [1.2.3] - 2026-03-01

### Bug Fixes
- **release:** Fix cargo-release config and generate changelog


### Chores
- Credit aliwi

- Release


### Features
- Add watch_paths, debounce_ms, rename_timeout_ms to nix modules

- Embed Windows icon into binary via winres


### Miscellaneous
- Add renovate.json

- Merge pull request #1 from chikof/renovate/configure

chore: Configure Renovate


## [1.2.2] - 2026-03-01

### Bug Fixes
- Add missing data to the build

- **nix:** Remove extra =

- **nix:** It's pname not name

- Update Cargo.lock


### Chores
- Add cargo-release + git-cliff release pipeline

- Forgot to add the cargo-release package

- Exclude watcher crate from cargo-release

- Add git-cliff package


### Features
- Multi-dir watching, expose debounce knobs, fix windows bugs


## [1.2.1] - 2026-02-24

### Chores
- Add credits

- Refine readme

- Oopsie daisy

- Forgot to list the `auto_update` setting


### Features
- Add log_level config

- Nix modules

- Replace simple watcher for a more complete one using notify

- **release:** Prepare new version


## [1.1.1] - 2026-02-23

### Bug Fixes
- **ci:** It seems that macos removed upx?


### Features
- Functional autostart & auto update improvements

- **release:** Prepare new version


## [1.0.1] - 2026-02-20

### Chores
- Forgot to push the license


### Features
- Binary optimizations

- Prepare new release


## [1.0.0] - 2026-02-20

### Features
- First commit



