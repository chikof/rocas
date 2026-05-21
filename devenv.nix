{
  pkgs,
  inputs,
  ...
}: let
  # Kept off PATH so it never shadows the nightly toolchain; the msrv check uses it.
  msrvToolchain = (inputs.rust-overlay.lib.mkRustBin {} pkgs).stable."1.88.0".minimal;
in {
  languages.rust = {
    enable = true;
    # rustfmt.toml uses nightly-only options and CI formats on nightly.
    channel = "nightly";
    components = ["rustc" "cargo" "clippy" "rustfmt" "rust-src" "rust-analyzer"];
  };

  packages = [
    pkgs.git
    pkgs.openssl
    pkgs.pkg-config
    pkgs.cargo-release
    pkgs.git-cliff
    pkgs.imagemagick
    pkgs.shellcheck
    pkgs.typos
    pkgs.cargo-deny
    pkgs.cargo-nextest
    pkgs.cargo-edit
    pkgs.bacon
    pkgs.actionlint
  ];

  env = {
    LAZYVIM_RUST_DIAGNOSTICS = "bacon-ls";
    ROCAS_MSRV_TOOLCHAIN = "${msrvToolchain}";
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
    LD_LIBRARY_PATH = "${pkgs.openssl.out}/lib";
  };

  # scripts/check.sh is the single gate.
  scripts.check.exec = ''
    exec "$DEVENV_ROOT/scripts/check.sh"
  '';

  enterShell = ''
    echo "rocas: run 'check' (gate)"
  '';
}
