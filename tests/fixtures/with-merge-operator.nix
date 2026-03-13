{
  meta.name = "MergeClan";

  inventory.instances.machine-type = {
    module.input = "self";
    module.name = "@pinpox/machine-type";
    roles.desktop.tags.desktop = { };
    roles.server.tags.server = { };
    roles.mobile.tags.mobile = { };
  } // {
    foo = "bar";
  };
}
