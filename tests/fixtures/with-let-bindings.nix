let
  username = "testuser";
  domain = "let-test.local";
in
{
  meta.name = "LetBindingClan";
  meta.domain = domain;

  inventory.machines = {
    workstation = { };
  };

  inventory.instances = {
    user-config = {
      module = {
        name = "users";
        input = "clan-core";
      };
      roles.default.machines.workstation = { };
      roles.default.settings = {
        user = username;
        groups = [
          "wheel"
          "networkmanager"
        ];
      };
    };
  };
}
