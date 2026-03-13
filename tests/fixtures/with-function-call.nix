let
  # A helper that adds default roles configuration to an instance
  someFunc = attrs: attrs // {
    module.input = "clan-core";
    roles.default.tags.all = { };
  };
in
{
  meta.name = "FuncClan";

  inventory.instances.transformed = someFunc {
    module.name = "transformed";
  };

  inventory.instances.simple = {
    module.name = "simple";
    module.input = "clan-core";
    roles.default.tags.all = { };
  };
}
