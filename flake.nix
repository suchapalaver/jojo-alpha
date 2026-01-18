{
  description = "DeFi Trading Agent - AI-powered trading with BAML runtime";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            # Rust toolchain
            rustToolchain

            # Build essentials
            pkg-config
            cmake
            clang

            # For bindgen (quickjs-sys)
            llvmPackages.libclang
          ];

          buildInputs = with pkgs; [
            # Required for native dependencies
            openssl
            openssl.dev
          ];

          shellHook = ''
            echo "DeFi Trading Agent dev environment"
            echo "  rustc: $(rustc --version)"
            echo "  cargo: $(cargo --version)"
          '';

          RUST_BACKTRACE = 1;
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        };
      });
}
