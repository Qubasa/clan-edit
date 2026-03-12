# Simplified from clanNixExamples/perstarkse.nix
# A flake-parts module using flake.clan = { ... } pattern with complex inventory.
{ ... }:
{
  flake.clan = {
    meta.name = "heliosphere";

    inventory = {
      machines = {
        sedna = {
          deploy.buildHost = "root@charon.lan";
          tags = [
            "server"
          ];
        };
        io = {
          deploy.buildHost = "root@charon.lan";
          tags = [
            "server"
          ];
        };
        makemake = {
          deploy.buildHost = "root@charon.lan";
          tags = [
            "server"
          ];
        };
        charon = {
          tags = [
            "client"
          ];
        };
        ariel = {
          deploy.buildHost = "root@charon.lan";
          tags = [
            "client"
          ];
        };
      };

      instances = {
        zerotier = {
          roles = {
            controller.machines.io = {};
            peer.tags.all = {};
          };
        };
        clan-cache = {
          module = {
            name = "trusted-nix-caches";
            input = "clan-core";
          };
          roles.default.tags.all = {};
        };
        sshd-basic = {
          module = {
            name = "sshd";
            input = "clan-core";
          };
          roles = {
            server = {
              tags.all = {};
              settings = {
                authorizedKeys = {
                  "p" = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAII6uq8nXD+QBMhXqRNywwCa/dl2VVvG/2nvkw9HEPFzn";
                };
              };
            };
            client.tags.all = {};
          };
        };
        user-p = {
          module = {
            name = "users";
            input = "clan-core";
          };
          roles.default = {
            tags.all = {};
            settings = {
              user = "p";
              prompt = true;
            };
          };
        };
        emergency-access = {
          module = {
            name = "emergency-access";
            input = "clan-core";
          };
          roles.default.tags.nixos = {};
        };
      };
    };
  };
}
