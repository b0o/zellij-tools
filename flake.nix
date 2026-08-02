{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    crane,
    flake-utils,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachSystem ["x86_64-linux" "aarch64-linux" "aarch64-darwin"] (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };
        inherit (pkgs) lib;

        buildTarget = "wasm32-wasip1";
        cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
        cliCargoToml = fromTOML (builtins.readFile ./cli/Cargo.toml);
        pluginPackage = cargoToml.package.name;
        cliPackage = cliCargoToml.package.name;

        rustToolchain = p: p.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        rust = rustToolchain pkgs;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        src = craneLib.cleanCargoSource ./.;

        nativeBuildInputs = [pkgs.pkg-config];
        buildInputs = [pkgs.openssl];

        commonArgs = {
          inherit src nativeBuildInputs buildInputs;
          strictDeps = true;
        };

        pluginArgs =
          commonArgs
          // {
            pname = pluginPackage;
            version = cargoToml.package.version;
            cargoExtraArgs = "--locked -p ${pluginPackage} --target ${buildTarget}";

            # Tests can't run on the host: zellij_tile declares WASI FFI symbols
            # that only exist in the Zellij WASI runtime.
            doCheck = false;
          };

        pluginCargoArtifacts = craneLib.buildDepsOnly (pluginArgs
          // {
            pname = "${pluginPackage}-deps";
          });

        plugin = craneLib.buildPackage (pluginArgs
          // {
            cargoArtifacts = pluginCargoArtifacts;

            installPhase = ''
              runHook preInstall
              mkdir -p $out/share/zellij/plugins
              cp target/${buildTarget}/release/${pluginPackage}.wasm \
                $out/share/zellij/plugins/${pluginPackage}.wasm
              runHook postInstall
            '';
          });

        cliArgs =
          commonArgs
          // {
            pname = cliPackage;
            version = cliCargoToml.package.version;
            cargoExtraArgs = "--locked -p ${cliPackage}";
          };

        cliCargoArtifacts = craneLib.buildDepsOnly (cliArgs
          // {
            pname = "${cliPackage}-deps";
          });

        cli = craneLib.buildPackage (cliArgs
          // {
            cargoArtifacts = cliCargoArtifacts;
          });

        pluginClippy = craneLib.cargoClippy (pluginArgs
          // {
            cargoArtifacts = pluginCargoArtifacts;
            cargoClippyExtraArgs = "-- -D warnings";
          });

        cliClippy = craneLib.cargoClippy (cliArgs
          // {
            cargoArtifacts = cliCargoArtifacts;
            cargoClippyExtraArgs = "-- -D warnings";
          });

        packages = {
          default = plugin;
          inherit cli;
        };

        checks = {
          inherit plugin cli pluginClippy cliClippy;
        };

        appRuntimeInputs = [
          rust
          pkgs.dprint
          pkgs.nix
          pkgs.openssl
          pkgs.pkg-config
        ];

        mkCargoApp = name: spec: let
          inherit (spec) command;
          scriptName = "zellij-tools-${name}";
          openssl = pkgs.openssl.dev;
          libPath = lib.makeLibraryPath [openssl];
          script = pkgs.writeShellApplication {
            name = scriptName;
            runtimeInputs = appRuntimeInputs;
            text = ''
              export ZELLIJ_TOOLS_TARGET="${buildTarget}"
              export ZELLIJ_TOOLS_CLI_PACKAGE="${cliPackage}"
              export PKG_CONFIG_PATH="${openssl}/lib/pkgconfig:''${PKG_CONFIG_PATH:-}"
              export LD_LIBRARY_PATH="${libPath}:''${LD_LIBRARY_PATH:-}"
              ${command}
              # pruner-keep
            '';
          };
        in {
          type = "app";
          program = "${script}/bin/${scriptName}";
          meta.description = spec.description;
        };

        apps = rec {
          default = run;

          build-plugin = {
            description = "Build the plugin with Cargo";
            command =
              # sh
              ''
                cargo build --target "$ZELLIJ_TOOLS_TARGET"
              '';
          };

          build-cli = {
            description = "Build the CLI with Cargo";
            command =
              # sh
              ''
                cargo build -p "$ZELLIJ_TOOLS_CLI_PACKAGE"
              '';
          };

          build = {
            description = "Build the plugin and CLI with Cargo";
            command = build-plugin.command + build-cli.command;
          };

          build-plugin-release = {
            description = "Build the plugin in release mode with Cargo";
            command =
              # sh
              ''
                cargo build --release --target "$ZELLIJ_TOOLS_TARGET"
              '';
          };

          build-cli-release = {
            description = "Build the CLI in release mode with Cargo";
            command =
              # sh
              ''
                cargo build --release -p "$ZELLIJ_TOOLS_CLI_PACKAGE"
              '';
          };

          build-release = {
            description = "Build the plugin and CLI in release mode with Cargo";
            command = build-plugin-release.command + build-cli-release.command;
          };

          check = {
            description = "Run dprint and clippy checks";
            command =
              # sh
              ''
                dprint check
                cargo clippy --target "$ZELLIJ_TOOLS_TARGET" -- -D warnings
                cargo clippy -p "$ZELLIJ_TOOLS_CLI_PACKAGE" -- -D warnings
              '';
          };

          fmt = {
            description = "Format sources with dprint and rustfmt";
            command =
              # sh
              ''
                dprint fmt
                cargo fmt
              '';
          };

          test = {
            description = "Run library tests";
            command =
              # sh
              ''
                cargo test --lib
              '';
          };

          ci = {
            description = "Run nix flake check";
            command =
              # sh
              ''
                nix flake check
              '';
          };

          run = {
            description = "Run the CLI with Cargo";
            command =
              # sh
              ''
                cargo run -p "$ZELLIJ_TOOLS_CLI_PACKAGE" -- "$@"
              '';
          };

          run-release = {
            description = "Run the CLI in release mode with Cargo";
            command =
              # sh
              ''
                cargo run --release -p "$ZELLIJ_TOOLS_CLI_PACKAGE" -- "$@"
              '';
          };
        };
      in {
        inherit packages checks;
        apps = lib.mapAttrs mkCargoApp apps;

        devShells.default = craneLib.devShell {
          name = "zellij-tools";
          inherit checks;

          ZELLIJ_TOOLS_TARGET = buildTarget;
          ZELLIJ_TOOLS_CLI_PACKAGE = cliPackage;
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          LD_LIBRARY_PATH = lib.makeLibraryPath [pkgs.openssl];

          packages = [
            pkgs.curl
            pkgs.dprint
            pkgs.pkg-config
            pkgs.openssl
          ];
        };

        nixosModules.default = import ./. {inherit (packages) default;};
      }
    );
}
