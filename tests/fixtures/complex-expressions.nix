{lib, ...}:
let
  # A helper that enriches instance configs with a default module input
  addDefaultInput = attrs: attrs // { module.input = "clan-core"; };
in
{
  meta.name = "ComplexClan";

  # Merge operator — cannot navigate into
  inventory.instances.merged = {
    module.name = "merged";
    module.input = "clan-core";
  } // {
    roles.server.tags.all = { };
  };

  # Function application — cannot navigate into
  inventory.instances.applied = addDefaultInput {
    module.name = "applied";
  };

  # mkDefault wrapping an attrset — can navigate into
  inventory.instances.defaulted = lib.mkDefault {
    module.name = "defaulted";
    module.input = "clan-core";
    roles.server.tags.all = { };
  };

  # mkForce wrapping a simple value
  inventory.instances.forced-name.module.name = lib.mkForce "forced";
  inventory.instances.forced-name.module.input = "clan-core";

  # Let-in expression — cannot navigate into
  inventory.instances.let-bound = let
    key = "ssh-ed25519 AAAA";
  in {
    module.name = "let-bound";
    module.input = "clan-core";
    roles.server.settings.key = key;
  };

  # Plain attrset — can navigate into
  inventory.instances.plain = {
    module.name = "plain";
    module.input = "clan-core";
    roles.default.tags.all = { };
  };
}
