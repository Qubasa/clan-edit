{
  meta.name = "SpecialNamesClan";

  inventory.machines = {
    "webserver 2" = {
      deploy.targetHost = "10.0.0.2";
    };
    "2nd-backup" = { };
    normal-server = { };
  };

  inventory.instances = {
    sshd = {
      module.name = "sshd";
      roles.server.machines."webserver 2" = { };
      roles.server.machines."2nd-backup" = { };
    };
  };
}
