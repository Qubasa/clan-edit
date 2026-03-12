{
  meta.name = "PerMachineClan";
  meta.domain = "machines.local";

  inventory.machines = {
    alpha = { };
    beta = { };
  };

  inventory.instances = {
    monitoring = {
      roles = {
        client = {
          tags.all = { };
          settings.useSSL = true;
        };

        server.machines."alpha".settings = {
          grafana.enable = true;
          host = "alpha.machines.local";
        };
      };
    };

    sshd = {
      module = {
        name = "sshd";
        input = "clan-core";
      };
      roles.server.tags.all = { };
      roles.server.machines."alpha".settings = {
        certificate.searchDomains = [ "*.machines.local" ];
      };
      roles.server.machines."beta".settings = {
        certificate.searchDomains = [ "beta.machines.local" ];
      };
    };
  };
}
