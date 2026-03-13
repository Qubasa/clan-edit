{
  machines.server1 = {
    tags = [ "server" ];
  };

  instances.sshd = {
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
  };
}
