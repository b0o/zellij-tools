{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        inherit (pkgs) mkShell;

        rust = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        buildTarget = "wasm32-wasip1";

        rustPlatform = pkgs.makeRustPlatform {
          rustc = rust;
          cargo = rust;
        };

        packages.default = rustPlatform.buildRustPackage rec {
          name = "zellij-tools";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          buildPhase = ''
            cargo build --release --target=${buildTarget}
          '';

          installPhase = ''
            mkdir -p $out/lib/zellij/plugins
            cp target/${buildTarget}/release/${name}.wasm $out/lib/zellij/plugins
          '';
        };
      in {
        inherit packages;

        devShells.default = mkShell {
          name = "zellij-tools";

          buildInputs = [
            rust
            pkgs.just
            pkgs.curl
            pkgs.pkg-config
            pkgs.openssl
          ];
        };

        nixosModules.default = import ./. {inherit (packages) default;};
      }
    );
}
