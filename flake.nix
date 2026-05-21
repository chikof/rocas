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
            stable.cargo
          ];

        craneLib = crane.overrideToolchain toolchain;
        rocas = import ./nix/package.nix { inherit pkgs craneLib; };
      in
      {
        packages.default = rocas;
        apps.default = flake-utils.lib.mkApp { drv = rocas; };
        checks.default = rocas;
      }
    )
    // {
      nixosModules.default = import ./nix/modules/nixos.nix { inherit self; };
      homeManagerModules.default = import ./nix/modules/home-manager.nix { inherit self; };
    };
}
