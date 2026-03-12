{
  meta.name = "LetInstanceClan";

  inventory.machines.workstation = { };

  inventory.instances.sshd = let
    commonKey = "ssh-ed25519 AAAA-shared-key";
  in {
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
    roles.server.settings.authorizedKeys.shared = commonKey;
  };
}
