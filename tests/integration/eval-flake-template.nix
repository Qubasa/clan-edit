# Template flake.nix for evaluating a clan.nix file against clan-core.
# CLAN_CORE_PATH and CLAN_NIX_PATH are substituted at test time.
{
  inputs = {
    clan-core.url = "CLAN_CORE_PATH";
    nixpkgs.follows = "clan-core/nixpkgs";
  };

  outputs =
    { self, clan-core, ... }:
    let
      clan = clan-core.lib.clan {
        inherit self;
        meta.name = "eval-test";
        meta.domain = "test.local";
      };
    in
    {
      clan = clan.config;
    };
}
