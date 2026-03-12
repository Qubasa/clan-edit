# Simplified from clanNixExamples/friedow.nix
# A flake-parts module that sets clan config directly.
{ ... }:
{
  clan = {
    meta.name = "friedow";

    inventory.instances = {
      sshd = {
        module = {
          name = "sshd";
          input = "clan";
        };
        roles = {
          client.tags = [ "all" ];
          server.tags = [ "all" ];
        };
      };
    };
  };
}
