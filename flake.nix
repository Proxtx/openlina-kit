{
  description = "openlina-kit: modding toolkit for Mosa Lina";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "clippy" "rustfmt" ];
        targets = [ "wasm32-wasip1" ];
      };
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          rust
          pkgs.imagemagick
          pkgs.gifsicle
        ];
        # Keep build outputs separate from a rustup toolchain's `target/`.
        CARGO_TARGET_DIR = "target/nix";
      };
    };
}
