{lib, ...}:
{
  meta.name = lib.mkDefault "DefaultClan";

  inventory.machines.server = lib.mkForce {
    deploy.targetHost = "root@192.168.1.10";
  };

  inventory.instances.sshd = lib.mkDefault {
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
  };
}
