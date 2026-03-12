{
  meta.name = "InventoryClan";
  meta.domain = "example.com";

  inventory.machines = {
    webserver = {
      deploy.targetHost = "root@192.168.1.10";
    };
    dbserver = { };
  };

  inventory.instances = {
    sshd = {
      module = {
        name = "sshd";
        input = "clan-core";
      };
      roles.server.tags.all = { };
      roles.server.settings = {
        authorizedKeys = {
          "admin" = "ssh-rsa AAAA...";
        };
      };
      roles.client.tags.all = { };
    };

    zerotier = {
      roles.controller.machines.webserver = { };
      roles.peer.tags.all = { };
    };
  };
}
