{
  meta.name = "LetSharedClan";

  inventory.machines.server = { };

  inventory.instances.sshd = let
    commonKey = "ssh-ed25519 AAAA-shared-key";
  in {
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
    roles.server.settings.authorizedKeys.admin = commonKey;
    roles.server.settings.authorizedKeys.deploy = commonKey;
  };
}
