# clan-edit

A Rust CLI tool for programmatically editing `clan.nix` inventory files. Uses [rnix](https://github.com/nix-community/rnix-parser) for CST-preserving modifications -- comments, whitespace, and formatting are kept intact.

## How it works

clan-edit parses `.nix` files into a lossless concrete syntax tree (CST) using rnix/rowan, navigates to the target attribute path, and performs the edit directly on the source text using byte ranges from the AST.

After every write, clan-edit automatically runs `nix eval` against the enclosing flake to verify the change produces a valid configuration. If evaluation fails, the file is rolled back to its original content and the error is reported. Pass `--no-verify` to skip this check.

Attribute paths are dot-separated (e.g., `inventory.instances.sshd.roles.server.tags.all`) and work with both Nix syntax forms:

- Nested: `inventory = { machines = { server = { }; }; };`
- Dotted: `inventory.machines.server = { };`
- Mixed: `inventory.instances = { sshd.roles.server = { }; };`

When inserting new attributes, clan-edit finds the deepest existing ancestor attrset and adds the binding there, matching the indentation of surrounding code.

## Installation

```bash
# From the flake
nix build github:clan-lol/clan-edit

# Or in a dev shell
nix develop
cargo build
```

## Usage

clan-edit has three commands: `get`, `set`, and `delete`. All commands take `--file <path>` (defaults to `clan.nix`).

```bash
# Read a value
clan-edit get --path meta.name
# => "MyClan"

# Set a value (overwrites if exists, inserts if not)
clan-edit set --path meta.name --value '"NewName"'

# Delete an attribute
clan-edit delete --path inventory.instances.tor
```

### Global flags

```bash
--file <path>       # Path to the clan.nix file (default: clan.nix)
--no-verify         # Skip nix eval verification after writes
--flake <path>      # Flake directory for verification (default: auto-detect)
```

### Values

The `--value` argument to `set` takes raw Nix syntax. You're writing the literal text that will appear in the file:

```bash
# String (note the nested quotes)
clan-edit set --path meta.name --value '"hello"'

# Attribute set
clan-edit set --path inventory.machines.box --value '{ }'

# List
clan-edit set --path some.list --value '[ "a" "b" ]'

# Bool / Integer
clan-edit set --path some.flag --value 'true'
clan-edit set --path some.count --value '42'

# Multi-line attrset
clan-edit set --path inventory.machines.web --value '{
    deploy.targetHost = "root@10.0.0.1";
  }'
```

## Examples

### Managing machines

```bash
# Add a machine (empty config)
clan-edit set --path inventory.machines.webserver --value '{ }'

# Add a machine with deploy settings
clan-edit set --path inventory.machines.webserver --value '{
    deploy.targetHost = "root@192.168.1.10";
  }'

# Set a single field on an existing machine
clan-edit set --path inventory.machines.webserver.deploy.targetHost --value '"root@10.0.0.2"'

# Remove a machine
clan-edit delete --path inventory.machines.webserver
```

### Setting up an inventory service with roles and settings

This example sets up a full `sshd` service instance with a module reference, role assignments, tags, and per-role settings -- step by step.

```bash
# 1. Create the service instance with its module reference
clan-edit set --path inventory.instances.sshd --value '{
    module = {
      name = "sshd";
      input = "clan-core";
    };
  }'

# 2. Assign the "all" tag to the server role (all tagged machines become servers)
clan-edit set --path inventory.instances.sshd.roles.server.tags.all --value '{ }'

# 3. Add authorized SSH keys as role settings
clan-edit set --path inventory.instances.sshd.roles.server.settings.authorizedKeys.admin \
  --value '"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... admin@example.com"'

clan-edit set --path inventory.instances.sshd.roles.server.settings.authorizedKeys.deploy \
  --value '"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... deploy@ci"'

# 4. Assign the "all" tag to the client role too
clan-edit set --path inventory.instances.sshd.roles.client.tags.all --value '{ }'

# 5. Assign a specific machine to the controller role (instead of using tags)
clan-edit set --path inventory.instances.sshd.roles.controller.machines.gateway --value '{ }'
```

After these commands, the `clan.nix` file will contain:

```nix
{
  # ... existing config ...

  inventory.instances.sshd = {
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
    roles.server.settings = {
      authorizedKeys = {
        admin = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... admin@example.com";
        deploy = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... deploy@ci";
      };
    };
    roles.client.tags.all = { };
    roles.controller.machines.gateway = { };
  };
}
```

Each `set` invocation is individually verified via `nix eval` against clan-core. If any step introduces a type error or invalid structure, the change is rolled back and an error is printed.

### Editing values in place

```bash
# Read the current value
clan-edit get --path meta.name
# => "MyClan"

# Change it
clan-edit set --path meta.name --value '"UpdatedClan"'

# Update a nested setting inside an existing service
clan-edit set --path inventory.instances.sshd.roles.server.settings.authorizedKeys.newkey \
  --value '"ssh-ed25519 AAAA-new-key"'

# Delete a specific key
clan-edit delete --path inventory.instances.sshd.roles.server.settings.authorizedKeys.oldkey
```

### Special characters in attribute names

Nix attribute names that aren't simple identifiers need quoting. clan-edit handles this automatically -- when inserting a key that starts with a digit, contains spaces, or other special characters, it wraps the key in quotes in the output file.

```bash
# Dashes are valid Nix identifiers -- no quoting needed
clan-edit set --path inventory.machines.my-server --value '{ }'
# => my-server = { };

# Starts with a digit -- clan-edit auto-quotes it
clan-edit set --path inventory.machines.3rd-node --value '{ }'
# => "3rd-node" = { };

# Spaces in the name -- also auto-quoted
clan-edit set --path 'inventory.machines.my server' --value '{ }'
# => "my server" = { };

# Reading and editing existing quoted keys works the same way
clan-edit get --path 'inventory.machines.my server'
clan-edit set --path 'inventory.machines.webserver 2.deploy.targetHost' --value '"10.0.0.99"'
clan-edit delete --path 'inventory.machines.3rd-node'
```

**Limitation: dots in attribute names.** Since paths are split on `.`, attribute names that contain literal dots (e.g., `"my.machine"`) cannot be addressed. This is rarely an issue in practice -- clan inventory names don't use dots.

### `let ... in` bindings inside attribute values

When an attribute value uses a `let ... in` expression (e.g., to define shared variables within a service instance), clan-edit treats it as an opaque value:

- **Reading**: `get` returns the entire `let ... in { ... }` expression as-is.
- **Navigating into it**: `get`/`set` on sub-paths within the `let` body will fail, because clan-edit cannot look inside non-attrset expressions.
- **Replacing it**: `set` on the attribute itself replaces the entire `let ... in` expression with the new value.
- **Editing elsewhere**: Modifications to other parts of the file leave the `let ... in` expression completely intact.

```nix
# Example: sshd instance using a let binding for a shared key
inventory.instances.sshd = let
  commonKey = "ssh-ed25519 AAAA-shared-key";
in {
  module.name = "sshd";
  roles.server.settings.authorizedKeys.shared = commonKey;
};
```

```bash
# Reading the whole instance returns the let expression
clan-edit get --path inventory.instances.sshd
# => let commonKey = "ssh-ed25519 AAAA-shared-key"; in { ... }

# Cannot navigate into the let body -- this fails:
clan-edit get --path inventory.instances.sshd.module.name
# Error: attribute path not found

# Replacing the whole instance works (discards the let binding):
clan-edit set --path inventory.instances.sshd --value '{
    module.name = "sshd";
    roles.server.tags.all = { };
  }'

# Editing other parts of the file preserves the let expression:
clan-edit set --path inventory.machines.newbox --value '{ }'
# The sshd let...in is untouched
```

### Flake-parts projects

For projects using flake-parts with clan-core's flake module, attribute paths are prefixed with `clan.`:

```bash
clan-edit --file clan.nix set --path clan.meta.name --value '"MyFlakePartsClan"'
clan-edit --file clan.nix set --path clan.inventory.machines.server --value '{ }'
```

## Verification

By default, after every write clan-edit runs `nix eval` on the enclosing flake's `clan.inventory` output to check that the configuration still evaluates. It also evaluates the specific attribute path that was edited to catch type errors (e.g., setting a string where a submodule is expected).

If verification fails, the original file is restored automatically.

```bash
# This will fail and roll back -- "Custom_Name" is a string, not a submodule
clan-edit set --path inventory.machines.test --value '"Custom_Name"'
# Error: nix eval verification failed:
# error: A definition for option `inventory.machines.test' is not of type `submodule'.

# Skip verification when you know what you're doing
clan-edit --no-verify set --path meta.name --value '"Unverified"'

# Point to a specific flake directory
clan-edit --flake /path/to/my/flake set --path meta.name --value '"Hello"'
```

## Restrictions

**Syntax-only editing, evaluation-based verification.** clan-edit parses Nix syntax but does not evaluate it during editing. It cannot resolve variables, follow imports, or compute expressions. Verification is a separate post-write step using `nix eval`.

**Literal values only for `set`.** The `--value` must be a syntactically valid Nix expression that can be pasted directly into the source.

**Cannot modify inside expressions.** If a value at the target path is a function call, `let` binding, or other complex expression, `set` will replace the entire expression. It cannot navigate into or modify sub-parts of non-attrset values. See the [`let ... in` section above](#let--in-bindings-inside-attribute-values) for details.

**`let` bindings are preserved but opaque.** Top-level `let ... in { ... }` and `let ... in` values inside attributes are both supported -- edits elsewhere preserve them. However, clan-edit cannot navigate into `let` bodies. See above for examples.

**Lambda expressions supported.** Flake-parts module files (`{ ... }: { ... }`) are handled correctly.

**Dot-separated paths only.** Attribute keys are split on `.`, so names containing literal dots cannot be addressed. Keys with spaces, dashes, and digit prefixes work fine -- clan-edit auto-quotes them. See the [special characters section above](#special-characters-in-attribute-names).

**Single file.** Only one file is read and written per invocation.

## Testing

```bash
# Unit tests (pure Rust, no Nix needed)
cargo test

# Integration tests (requires Nix and clan-core, runs via nix)
nix run .#integration-tests

# Unit tests via nix flake check
nix flake check
```

The integration tests create temporary flakes that import edited `clan.nix` files via `clan-core.lib.clan`, then run `nix eval` to verify the edits produce valid inventory configurations. This catches issues that syntax-level tests cannot: wrong attribute names, type mismatches, missing required fields.

## Project structure

```
src/
  lib.rs          -- crate root
  ast.rs          -- core library: parse, navigate, get/set/delete
  main.rs         -- CLI entry point (clap), verification, rollback
tests/
  fixtures/       -- sample clan.nix files for testing
  integration/    -- nix eval integration tests
flake.nix         -- Nix package, dev shell, checks, integration test app
```
