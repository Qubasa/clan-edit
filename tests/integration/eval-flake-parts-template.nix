# Template flake.nix for evaluating a clan.nix file as a flake-parts module.
# CLAN_CORE_PATH is substituted at test time.
# The clan.nix file is imported as a flake-parts module that sets `clan = { ... }`.
{
  inputs = {
    clan-core.url = "CLAN_CORE_PATH";
    nixpkgs.follows = "clan-core/nixpkgs";
    flake-parts.follows = "clan-core/flake-parts";
  };

  outputs =
    inputs@{
      flake-parts,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [
        inputs.clan-core.flakeModules.default
        ./clan.nix
      ];
    };
}
