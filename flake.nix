{
  description = "clan-edit: CLI tool for editing clan.nix inventory files";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0.1"; # unstable Nixpkgs
    fenix = {
      url = "https://flakehub.com/f/nix-community/fenix/0.1";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    clan-core = {
      url = "git+https://git.clan.lol/clan/clan-core";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, ... }@inputs:

    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSupportedSystem =
        f:
        inputs.nixpkgs.lib.genAttrs supportedSystems (
          system:
          f {
            pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [
                inputs.self.overlays.default
              ];
            };
          }
        );
    in
    {
      overlays.default = final: prev: {
        rustToolchain =
          with inputs.fenix.packages.${prev.stdenv.hostPlatform.system};
          combine (
            with stable;
            [
              clippy
              rustc
              cargo
              rustfmt
              rust-src
            ]
          );
      };

      packages = forEachSupportedSystem (
        { pkgs }:
        rec {
          clan-edit = pkgs.rustPlatform.buildRustPackage {
            pname = "clan-edit";
            version = "0.1.0";
            src = ./.;
            cargoHash = "sha256-40ru+/Wx+Jjc7VHlcDwzFUvEq46/jf3vwbwznzH50i0=";
          };
          default = clan-edit;
        }
      );

      checks = forEachSupportedSystem (
        { pkgs }:
        let
          system = pkgs.stdenv.hostPlatform.system;
          pythonEnv = pkgs.python3.withPackages (ps: [ ps.pytest ]);

          # Collect all resolved flake input source trees (for closureInfo)
          collectFlakeInputs =
            depth: flakeInput:
            if depth <= 0 then
              [ ]
            else
              let
                direct = builtins.removeAttrs (flakeInput.inputs or { }) [ "self" ];
                paths = builtins.attrValues direct;
              in
              paths ++ builtins.concatMap (collectFlakeInputs (depth - 1)) paths;

          # Generate --override-input flags so nix eval can resolve all
          # inputs without network access inside the sandbox.
          clanCoreInputs = builtins.removeAttrs (inputs.clan-core.inputs or { }) [ "self" ];
          overrideFlags = builtins.concatStringsSep " " (
            [ "--override-input clan-core path:${inputs.clan-core}" ]
            ++ builtins.attrValues (
              builtins.mapAttrs (
                name: input: "--override-input clan-core/${name} path:${input}"
              ) clanCoreInputs
            )
          );

          # Wrapper that injects --override-input flags into every nix
          # command so both conftest.py and clan-edit's internal nix eval
          # resolve inputs from the pre-populated local store.
          nixWrapper = pkgs.writeShellScriptBin "nix" ''
            exec ${pkgs.nix}/bin/nix "$@" ${overrideFlags}
          '';
        in
        {
          # Unit tests (cargo test, runs in sandbox)
          unit-tests = self.packages.${system}.clan-edit.overrideAttrs {
            pname = "clan-edit-unit-tests";
            doCheck = true;
          };

          # Integration tests (nix eval via isolated store, runs in sandbox)
          integration-tests = pkgs.runCommand "clan-edit-integration-tests" {
            nativeBuildInputs = [
              nixWrapper # must come before pkgs.nix to shadow it
              self.packages.${system}.default
              pkgs.nix
              pkgs.git
              pythonEnv
            ];
            closureInfo = pkgs.closureInfo {
              rootPaths = [ inputs.clan-core ] ++ collectFlakeInputs 3 inputs.clan-core;
            };
          } ''
            set -euo pipefail

            export HOME=$TMPDIR
            export NIX_STATE_DIR=$TMPDIR/nix
            export NIX_CONF_DIR=$TMPDIR/etc
            CLAN_TEST_STORE=$TMPDIR/store

            mkdir -p "$CLAN_TEST_STORE/nix/store"
            mkdir -p "$CLAN_TEST_STORE/nix/var/nix/gcroots"
            mkdir -p "$NIX_CONF_DIR"

            echo "experimental-features = nix-command flakes" > "$NIX_CONF_DIR/nix.conf"
            echo "store = local?root=$CLAN_TEST_STORE" >> "$NIX_CONF_DIR/nix.conf"

            # Pre-populate the local store with all source trees needed by nix eval
            ${pkgs.findutils}/bin/xargs -r -P"$(nproc)" \
              ${pkgs.coreutils}/bin/cp --recursive --no-dereference --reflink=auto \
              --target-directory "$CLAN_TEST_STORE/nix/store" < "$closureInfo/store-paths"
            ${pkgs.nix}/bin/nix-store --load-db --store "$CLAN_TEST_STORE" < "$closureInfo/registration"

            export CLAN_CORE_PATH="${inputs.clan-core}"
            export FIXTURES_DIR="${./tests/fixtures}"

            ${pythonEnv}/bin/python -m pytest ${./tests/integration} -v

            mkdir -p $out
            touch $out/success
          '';
        }
      );

      devShells = forEachSupportedSystem (
        { pkgs }:
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustToolchain
              openssl
              pkg-config
              (python3.withPackages (ps: [ ps.pytest ]))
              mypy
              cargo-deny
              cargo-edit
              cargo-watch
              rust-analyzer
            ];

            env = {
              # Required by rust-analyzer
              RUST_SRC_PATH = "${pkgs.rustToolchain}/lib/rustlib/src/rust/library";
              # For integration tests
              CLAN_CORE_PATH = "${inputs.clan-core}";
            };
          };
        }
      );
    };
}
