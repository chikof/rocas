# Builds the rocas binary using crane + fenix.
# Called from flake.nix with: pkgs, craneLib
{ pkgs, craneLib }:

let
  cargoMeta = craneLib.crateNameFromCargoToml = { cargoToml = ../Cargo.toml; };

  commonArgs = {
    inherit (cargoMeta) name version;
    src = craneLib.cleanCargoSource ../.;
    strictDeps = true;
    buildInputs = with pkgs; [ openssl ];
    nativeBuildInputs = with pkgs; [ pkg-config ];
  };
in
craneLib.buildPackage commonArgs
