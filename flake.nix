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
            cargoHash = "sha256-VqtzS9hRnYNS+BXECDwU5ok50mEv6pgx5cLXROBlhb8=";
          };
          default = clan-edit;
        }
      );

      checks = forEachSupportedSystem (
        { pkgs }:
        {
          # Unit tests (cargo test, runs in sandbox)
          unit-tests = self.packages.${pkgs.stdenv.hostPlatform.system}.clan-edit.overrideAttrs {
            pname = "clan-edit-unit-tests";
            doCheck = true;
          };
        }
      );

      # Integration tests need nix eval (the daemon), so they can't run in the
      # nix sandbox.  Run via: nix run .#integration-tests
      apps = forEachSupportedSystem (
        { pkgs }:
        let
          system = pkgs.stdenv.hostPlatform.system;
          script = pkgs.writeShellApplication {
            name = "clan-edit-integration-tests";
            runtimeInputs = [
              self.packages.${system}.default
              pkgs.nix
              pkgs.git
            ];
            text = ''
              export CLAN_CORE_PATH="${inputs.clan-core}"
              export FIXTURES_DIR="${./tests/fixtures}"
              bash ${./tests/integration/run-tests.sh}
            '';
          };
        in
        {
          integration-tests = {
            type = "app";
            program = "${script}/bin/clan-edit-integration-tests";
          };
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
