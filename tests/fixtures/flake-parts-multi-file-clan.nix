# Flake-parts module that imports machine definitions from a separate file.
{ ... }:
{
  clan = {
    imports = [ ./flake-parts-multi-file-machines.nix ];

    meta.name = "FlakePartsMultiClan";

    inventory.machines.sara = {
      deploy.targetHost = "root@192.168.1.20";
    };
  };
}
