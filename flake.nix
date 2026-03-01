{
  description = "rocas - file watcher and organizer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    flake-utils.url = "github:numtide/flake-utils";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      ...
    }@inputs:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        crane = inputs.crane.mkLib pkgs;

        toolchain =
          with fenix.packages.${system};
          combine [
            stable.rustc
            stable.rust-src
            stable.cargo
            complete.rustfmt
            stable.clippy
            stable.rust-analyzer
          ];

        craneLib = crane.overrideToolchain toolchain;
        rocas = import ./nix/package.nix { inherit pkgs craneLib; };
      in
      {
        packages.default = rocas;
        apps.default = flake-utils.lib.mkApp { drv = rocas; };
        checks.default = rocas;

        devShells.default = craneLib.devShell {
          packages = with pkgs; [
            toolchain
            openssl
            pkg-config
            cargo-release
          ];
          env = {
            LAZYVIM_RUST_DIAGNOSTICS = "bacon-ls";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
            LD_LIBRARY_PATH = "${pkgs.openssl.out}/lib";
          };
        };
      }
    )
    // {
      nixosModules.default = import ./nix/modules/nixos.nix { inherit self; };
      homeManagerModules.default = import ./nix/modules/home-manager.nix { inherit self; };
    };
}
