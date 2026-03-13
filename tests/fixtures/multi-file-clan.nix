{
  imports = [ ./multi-file-machines.nix ];

  meta.name = "MultiFileClan";

  inventory.machines.sara = {
    deploy.targetHost = "root@192.168.1.20";
  };
}
