# Builds the rocas binary using crane + fenix.
# Called from flake.nix with: craneLib (pkgs is passed too, unused)
{ craneLib, ... }:

let
  cargoMeta = craneLib.crateNameFromCargoToml { cargoToml = ../rocas/Cargo.toml; };

  commonArgs = {
    inherit (cargoMeta) pname version;
    src = craneLib.cleanCargoSource ../.;
    strictDeps = true;
    # Self-update is off: nix manages updates, and it is most of the binary.
    cargoExtraArgs = "--locked -p rocas --no-default-features";
  };
in
craneLib.buildPackage commonArgs
