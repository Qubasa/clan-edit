# Flake-parts module that imports inventory from a separate file.
{ ... }:
{
  clan = {
    meta.name = "SeparateFileClan";

    inventory = import ./inventory-settings.nix;
  };
}
