{
  imports = [ ./clan-parts-machines.nix ];

  meta.name = "ClanPartsClan";

  inventory.instances = {
    sshd = {
      roles.server.tags.all = { };
    };
  };
}
