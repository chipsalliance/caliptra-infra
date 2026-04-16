{
  description = "Nix flake for caliptra-bitstream-downloader";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default;

        caliptra-bitstream-downloader =
          with pkgs;
          rustPlatform.buildRustPackage {
            pname = "caliptra-bitstream-downloader";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [
            ];
            buildInputs = [
            ];
          };

      in
      {
        packages.default = caliptra-bitstream-downloader;

        apps.default = flake-utils.lib.mkApp {
          drv = caliptra-bitstream-downloader;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
          ];
        };
      }
    );
}
