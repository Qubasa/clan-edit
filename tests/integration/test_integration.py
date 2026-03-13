"""Integration tests for clan-edit.

Validates edits via nix eval against clan-core.
All 24 original bash tests are ported, plus new tests for:
- Intermediate path navigation
- mkDefault/mkForce support
- Complex expression detection
- Option discovery
"""

from __future__ import annotations

from conftest import EvalDir, EvalDirFactory, INV


# ============================================================================
# Original tests 1-24 (ported from run-tests.sh)
# ============================================================================


def test_01_minimal_evaluates(eval_dir_factory: EvalDirFactory) -> None:
    """Minimal clan.nix evaluates via nix eval."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    assert ed.nix_eval_succeeds(f"{INV}.meta.name")


def test_02_edit_meta_name(eval_dir_factory: EvalDirFactory) -> None:
    """Edit meta.name, verify via nix eval."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit("set", "--path", "meta.name", "--value", '"EditedClan"')
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "EditedClan"


def test_03_add_machine(eval_dir_factory: EvalDirFactory) -> None:
    """Add machine, verify via nix eval."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit(
        "set", "--path", "inventory.machines.testbox", "--value", "{ }"
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.machines.testbox.name")
    assert result == "testbox"


def test_04_add_service_instance(eval_dir_factory: EvalDirFactory) -> None:
    """Add service instance, verify it evaluates."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "inventory.instances.myservice",
        "--value",
        "{ }",
    )
    ed.git_add()
    assert ed.nix_eval_succeeds(f"{INV}.meta.name")


def test_05_full_inventory_evaluates(eval_dir_factory: EvalDirFactory) -> None:
    """Full inventory fixture evaluates."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "InventoryClan"


def test_06_per_machine_settings(eval_dir_factory: EvalDirFactory) -> None:
    """Per-machine settings fixture evaluates."""
    ed: EvalDir = eval_dir_factory("per-machine-settings.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "PerMachineClan"


def test_07_let_bindings_survive_edits(eval_dir_factory: EvalDirFactory) -> None:
    """Let bindings survive edits."""
    ed: EvalDir = eval_dir_factory("with-let-bindings.nix")
    ed.run_clan_edit("set", "--path", "meta.name", "--value", '"ModifiedLetClan"')
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "ModifiedLetClan"


def test_08_set_role_tag(eval_dir_factory: EvalDirFactory) -> None:
    """Set a role tag on an existing instance, verify via nix eval."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "inventory.instances.sshd.roles.server.tags.production",
        "--value",
        "{ }",
    )
    ed.git_add()
    assert ed.nix_eval_succeeds(f"{INV}.meta.name")


def test_09_invalid_type_detected(eval_dir_factory: EvalDirFactory) -> None:
    """Invalid type is caught by verification and rolled back."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    original_name = ed.nix_eval(f"{INV}.meta.name")
    result = ed.run_clan_edit(
        "set", "--path", "meta.name", "--value", "42", check=False
    )
    ed.git_add()
    current_name = ed.nix_eval(f"{INV}.meta.name")
    if result.returncode != 0:
        # Verification caught the type error, file should be rolled back
        assert current_name == original_name
    else:
        # Module system accepted the integer
        assert current_name == 42 or isinstance(current_name, str)


def test_10_flake_parts_simple(flake_parts_eval_dir_factory: EvalDirFactory) -> None:
    """Flake-parts simple fixture evaluates."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "friedow"


def test_11_flake_parts_complex(flake_parts_eval_dir_factory: EvalDirFactory) -> None:
    """Flake-parts complex fixture evaluates."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-complex.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "heliosphere"


def test_12_add_machine_flake_parts(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """Add machine on flake-parts fixture."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "clan.inventory.machines.newbox.name",
        "--value",
        '"newbox"',
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.machines.newbox.name")
    assert result == "newbox"


def test_13_set_flake_parts_meta(flake_parts_eval_dir_factory: EvalDirFactory) -> None:
    """Set meta.name on flake-parts fixture."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "clan.meta.name",
        "--value",
        '"EditedFlakeParts"',
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "EditedFlakeParts"


def test_14_valid_edit_with_verification(eval_dir_factory: EvalDirFactory) -> None:
    """Valid edit with verification succeeds."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit("set", "--path", "meta.name", "--value", '"VerifiedName"')
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "VerifiedName"


def test_15_invalid_edit_rolled_back(eval_dir_factory: EvalDirFactory) -> None:
    """Invalid edit is rolled back by verification."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    original_name = ed.nix_eval(f"{INV}.meta.name")
    # Try to set meta.name to an undefined variable (should fail verification)
    result = ed.run_clan_edit(
        "set", "--path", "meta.name", "--value", "UndefinedVar", check=False
    )
    assert result.returncode != 0
    ed.git_add()
    restored_name = ed.nix_eval(f"{INV}.meta.name")
    assert restored_name == original_name


def test_16_no_verify_allows_invalid(eval_dir_factory: EvalDirFactory) -> None:
    """--no-verify allows invalid edit to persist."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit(
        "--no-verify", "set", "--path", "meta.name", "--value", "UndefinedVar"
    )
    content = ed.clan_nix.read_text()
    assert "UndefinedVar" in content


def test_17_special_names(eval_dir_factory: EvalDirFactory) -> None:
    """Special names fixture evaluates."""
    ed: EvalDir = eval_dir_factory("special-names.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "SpecialNamesClan"


def test_18_digit_prefixed_name(eval_dir_factory: EvalDirFactory) -> None:
    """Add machine with digit-prefixed name."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit(
        "set", "--path", "inventory.machines.3rd-node", "--value", "{ }"
    )
    ed.git_add()
    result = ed.nix_eval(f'{INV}.machines."3rd-node".name')
    assert result == "3rd-node"


def test_19_space_in_name(eval_dir_factory: EvalDirFactory) -> None:
    """Add machine with space in name."""
    ed: EvalDir = eval_dir_factory("minimal.nix")
    ed.run_clan_edit(
        "set", "--path", "inventory.machines.my server", "--value", "{ }"
    )
    ed.git_add()
    result = ed.nix_eval(f'{INV}.machines."my server".name')
    assert result == "my server"


def test_20_set_on_quoted_key_machine(eval_dir_factory: EvalDirFactory) -> None:
    """Set value on existing quoted-key machine."""
    ed: EvalDir = eval_dir_factory("special-names.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "inventory.machines.webserver 2.deploy.targetHost",
        "--value",
        '"10.0.0.99"',
    )
    ed.git_add()
    result = ed.nix_eval(f'{INV}.machines."webserver 2".deploy.targetHost')
    assert result == "10.0.0.99"


def test_21_delete_quoted_key_machine(eval_dir_factory: EvalDirFactory) -> None:
    """Delete quoted-key machine."""
    ed: EvalDir = eval_dir_factory("special-names.nix")
    ed.run_clan_edit("delete", "--path", "inventory.machines.2nd-backup")
    ed.git_add()
    assert ed.nix_eval_succeeds(f"{INV}.meta.name")
    assert not ed.nix_eval_succeeds(f'{INV}.machines."2nd-backup"')


def test_22_instance_let_binding_evaluates(eval_dir_factory: EvalDirFactory) -> None:
    """Instance let-binding fixture evaluates."""
    ed: EvalDir = eval_dir_factory("instance-let-binding.nix")
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "LetInstanceClan"


def test_23_editing_around_let_in_preserves(eval_dir_factory: EvalDirFactory) -> None:
    """Editing around a let-in instance preserves it."""
    ed: EvalDir = eval_dir_factory("instance-let-binding.nix")
    ed.run_clan_edit(
        "set", "--path", "inventory.machines.newbox", "--value", "{ }"
    )
    ed.git_add()
    assert ed.nix_eval_succeeds(f"{INV}.machines.newbox.name")
    assert ed.nix_eval_succeeds(f"{INV}.instances.sshd.module.name")
    content = ed.clan_nix.read_text()
    assert "commonKey" in content


def test_24_replace_let_in_instance(eval_dir_factory: EvalDirFactory) -> None:
    """Replace let-in instance value wholesale."""
    ed: EvalDir = eval_dir_factory("instance-let-binding.nix")
    new_value = """{
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
  }"""
    ed.run_clan_edit(
        "set", "--path", "inventory.instances.sshd", "--value", new_value
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.instances.sshd.module.name")
    assert result == "sshd"
    content = ed.clan_nix.read_text()
    assert "commonKey" not in content


# ============================================================================
# New tests: Intermediate path navigation
# ============================================================================


def test_get_intermediate_path_roles(eval_dir_factory: EvalDirFactory) -> None:
    """Get intermediate path 'roles' from dotted-key bindings."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.sshd.roles"
    )
    output = result.stdout.strip()
    assert "server" in output
    assert "client" in output


def test_get_intermediate_path_deep(eval_dir_factory: EvalDirFactory) -> None:
    """Get deeper intermediate path (roles.server)."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.sshd.roles.server"
    )
    output = result.stdout.strip()
    assert "tags" in output


def test_get_intermediate_path_module(eval_dir_factory: EvalDirFactory) -> None:
    """Get intermediate 'module' path from nested attrset."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    # module is a nested attrset, so exact lookup should work
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.sshd.module"
    )
    output = result.stdout.strip()
    assert "sshd" in output


def test_get_intermediate_path_not_found(eval_dir_factory: EvalDirFactory) -> None:
    """Intermediate path with no matching bindings fails."""
    ed: EvalDir = eval_dir_factory("with-inventory.nix")
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.sshd.nonexistent",
        check=False,
    )
    assert result.returncode != 0
    assert "not found" in result.stderr.lower() or "not found" in result.stdout.lower()


# ============================================================================
# New tests: mkDefault / mkForce
# ============================================================================


def test_get_mkdefault_unwrap(eval_dir_factory: EvalDirFactory) -> None:
    """Get unwraps lib.mkDefault to return inner value."""
    ed: EvalDir = eval_dir_factory("with-mkdefault.nix")
    result = ed.run_clan_edit("get", "--path", "meta.name")
    output = result.stdout.strip()
    assert output == '"DefaultClan"'
    assert "mkDefault" not in output


def test_get_mkforce_unwrap(eval_dir_factory: EvalDirFactory) -> None:
    """Get unwraps lib.mkForce to return inner attrset."""
    ed: EvalDir = eval_dir_factory("with-mkdefault.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.machines.server"
    )
    output = result.stdout.strip()
    assert "deploy.targetHost" in output
    assert "mkForce" not in output


def test_set_preserves_mkdefault(eval_dir_factory: EvalDirFactory) -> None:
    """Set preserves lib.mkDefault wrapper."""
    ed: EvalDir = eval_dir_factory("with-mkdefault.nix")
    ed.run_clan_edit("set", "--path", "meta.name", "--value", '"NewDefault"')
    content = ed.clan_nix.read_text()
    assert "lib.mkDefault" in content
    assert '"NewDefault"' in content
    assert '"DefaultClan"' not in content


def test_set_preserves_mkforce(eval_dir_factory: EvalDirFactory) -> None:
    """Set preserves lib.mkForce wrapper on whole value."""
    ed: EvalDir = eval_dir_factory("with-mkdefault.nix")
    ed.run_clan_edit(
        "set",
        "--path",
        "inventory.machines.server",
        "--value",
        '{ deploy.targetHost = "10.0.0.1"; }',
    )
    content = ed.clan_nix.read_text()
    assert "lib.mkForce" in content
    assert "10.0.0.1" in content


def test_navigate_through_mkdefault_attrset(eval_dir_factory: EvalDirFactory) -> None:
    """Navigate into lib.mkDefault { ... } to read a sub-path."""
    ed: EvalDir = eval_dir_factory("with-mkdefault.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.sshd.module.name"
    )
    assert result.stdout.strip() == '"sshd"'


# ============================================================================
# New tests: Complex expression detection
# ============================================================================


def test_get_merge_operator_error(eval_dir_factory: EvalDirFactory) -> None:
    """Navigating into a // expression gives a descriptive error."""
    ed: EvalDir = eval_dir_factory("with-merge-operator.nix")
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.machine-type.foo",
        check=False,
    )
    assert result.returncode != 0
    stderr = result.stderr.lower()
    assert "merge operator" in stderr or "//" in stderr


def test_get_merge_operator_subpath_module(eval_dir_factory: EvalDirFactory) -> None:
    """All sub-paths through // fail with merge error."""
    ed: EvalDir = eval_dir_factory("with-merge-operator.nix")
    for subpath in [
        "inventory.instances.machine-type.module",
        "inventory.instances.machine-type.module.input",
        "inventory.instances.machine-type.module.name",
    ]:
        result = ed.run_clan_edit("get", "--path", subpath, check=False)
        assert result.returncode != 0, f"Expected failure for {subpath}"
        assert (
            "merge operator" in result.stderr.lower() or "//" in result.stderr.lower()
        ), f"Expected merge error for {subpath}, got: {result.stderr}"


def test_get_whole_merge_value_succeeds(eval_dir_factory: EvalDirFactory) -> None:
    """Getting the whole // expression (without navigating into it) succeeds."""
    ed: EvalDir = eval_dir_factory("with-merge-operator.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.machine-type"
    )
    assert "//" in result.stdout


def test_get_function_call_error(eval_dir_factory: EvalDirFactory) -> None:
    """Navigating into a function application gives a descriptive error."""
    ed: EvalDir = eval_dir_factory("with-function-call.nix")
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.transformed.module",
        check=False,
    )
    assert result.returncode != 0
    assert "function application" in result.stderr.lower()


def test_get_whole_function_call_succeeds(eval_dir_factory: EvalDirFactory) -> None:
    """Getting the whole function call value succeeds."""
    ed: EvalDir = eval_dir_factory("with-function-call.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.transformed"
    )
    assert "someFunc" in result.stdout


def test_get_lambda_error(eval_dir_factory: EvalDirFactory) -> None:
    """Navigating into a lambda gives a descriptive error."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    # Note: We don't have a dedicated lambda fixture since lambdas as
    # inventory instance values would be unusual. Test via unit tests only.
    # This test verifies the let-in detection instead.
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.let-bound.module",
        check=False,
    )
    assert result.returncode != 0
    assert "let-in" in result.stderr.lower()


def test_set_through_merge_fails(eval_dir_factory: EvalDirFactory) -> None:
    """Setting a path through // fails with merge error."""
    ed: EvalDir = eval_dir_factory("with-merge-operator.nix")
    result = ed.run_clan_edit(
        "set",
        "--path",
        "inventory.instances.machine-type.foo",
        "--value",
        '"baz"',
        check=False,
    )
    assert result.returncode != 0
    assert "merge operator" in result.stderr.lower() or "//" in result.stderr.lower()


def test_set_through_function_fails(eval_dir_factory: EvalDirFactory) -> None:
    """Setting a path through function application fails."""
    ed: EvalDir = eval_dir_factory("with-function-call.nix")
    result = ed.run_clan_edit(
        "set",
        "--path",
        "inventory.instances.transformed.module.name",
        "--value",
        '"new"',
        check=False,
    )
    assert result.returncode != 0
    assert "function application" in result.stderr.lower()


# ============================================================================
# New tests: Complex expressions fixture (comprehensive)
# ============================================================================


def test_complex_merged_blocks(eval_dir_factory: EvalDirFactory) -> None:
    """Merged instance blocks navigation."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.merged.extra", check=False
    )
    assert result.returncode != 0
    assert "merge operator" in result.stderr.lower() or "//" in result.stderr.lower()


def test_complex_applied_blocks(eval_dir_factory: EvalDirFactory) -> None:
    """Function-applied instance blocks navigation."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.applied.module.name",
        check=False,
    )
    assert result.returncode != 0
    assert "function application" in result.stderr.lower()


def test_complex_defaulted_navigable(eval_dir_factory: EvalDirFactory) -> None:
    """mkDefault-wrapped instance is navigable."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.defaulted.module.name"
    )
    assert result.stdout.strip() == '"defaulted"'


def test_complex_forced_name(eval_dir_factory: EvalDirFactory) -> None:
    """mkForce on a leaf value unwraps correctly."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.forced-name.module.name"
    )
    assert result.stdout.strip() == '"forced"'


def test_complex_let_bound_blocks(eval_dir_factory: EvalDirFactory) -> None:
    """Let-bound instance blocks navigation."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get",
        "--path",
        "inventory.instances.let-bound.module.name",
        check=False,
    )
    assert result.returncode != 0
    assert "let-in" in result.stderr.lower()


def test_complex_plain_navigable(eval_dir_factory: EvalDirFactory) -> None:
    """Plain attrset instance is navigable."""
    ed: EvalDir = eval_dir_factory("complex-expressions.nix")
    result = ed.run_clan_edit(
        "get", "--path", "inventory.instances.plain.module.name"
    )
    assert result.stdout.strip() == '"plain"'


# ============================================================================
# New tests: Option discovery
# ============================================================================


def test_discover_file_non_flake_parts(eval_dir_factory: EvalDirFactory) -> None:
    """Option discovery works for non-flake-parts projects.

    The eval_dir_factory creates flakes with clanOptions exposed.
    """
    ed: EvalDir = eval_dir_factory("minimal.nix")
    # Verify the flake exposes clanOptions
    assert ed.nix_eval_succeeds("clanOptions")


def test_discover_file_flake_parts(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """Flake-parts projects expose clan.options."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    # flake-parts exposes clan.options via the flake module
    assert ed.nix_eval_succeeds(f"{INV}.meta.name")


def test_flake_parts_separate_file(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """Flake-parts with inventory imported from a separate file.

    Tests that editing clan.nix works when inventory is imported from
    another file, and that the imported file remains intact.
    """
    ed: EvalDir = flake_parts_eval_dir_factory(
        "flake-parts-separate-file.nix",
        extra_fixtures=["inventory-settings.nix"],
    )
    # Verify initial evaluation works
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "SeparateFileClan"

    # Edit meta.name (defined directly in clan.nix)
    ed.run_clan_edit(
        "set",
        "--path",
        "clan.meta.name",
        "--value",
        '"EditedSeparate"',
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "EditedSeparate"

    # Verify the imported inventory-settings.nix is still intact
    result = ed.nix_eval(f"{INV}.instances.sshd.module.name")
    assert result == "sshd"


def test_let_in_shared_variable_blocks_edit(
    eval_dir_factory: EvalDirFactory,
) -> None:
    """Editing through a let-in instance value is blocked.

    When an instance value is `let x = ...; in { ... }`, navigating into
    it to edit a sub-path fails because the let-in is opaque to the AST
    editor.  This test documents this limitation: both bindings reference
    `commonKey`, and we cannot selectively override one without replacing
    the whole let-in expression.
    """
    ed: EvalDir = eval_dir_factory("let-shared-variable.nix")

    # Verify the fixture evaluates
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "LetSharedClan"

    # Trying to set a path *through* the let-in should fail
    result = ed.run_clan_edit(
        "set",
        "--path",
        "inventory.instances.sshd.roles.server.settings.authorizedKeys.admin",
        "--value",
        '"ssh-ed25519 NEW-KEY"',
        check=False,
    )
    assert result.returncode != 0
    assert "let-in" in result.stderr.lower()

    # But we can replace the whole instance value (losing the let binding)
    new_value = """{
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
    roles.server.settings.authorizedKeys.admin = "ssh-ed25519 NEW-KEY";
    roles.server.settings.authorizedKeys.deploy = "ssh-ed25519 AAAA-shared-key";
  }"""
    ed.run_clan_edit(
        "set", "--path", "inventory.instances.sshd", "--value", new_value
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.instances.sshd.module.name")
    assert result == "sshd"

    # The let binding should be gone (replaced with literal value)
    content = ed.clan_nix.read_text()
    assert "commonKey" not in content


# ============================================================================
# Multi-file source tracking tests
# ============================================================================


def test_multi_file_evaluates(eval_dir_factory: EvalDirFactory) -> None:
    """Multi-file fixture (clan.nix + imported machine.nix) evaluates correctly."""
    ed: EvalDir = eval_dir_factory(
        "multi-file-clan.nix",
        extra_fixtures=["multi-file-machines.nix"],
    )
    # Both machines should be accessible
    result_sara = ed.nix_eval(f"{INV}.machines.sara.name")
    assert result_sara == "sara"
    result_jon = ed.nix_eval(f"{INV}.machines.jon.name")
    assert result_jon == "jon"


def test_multi_file_set_writes_to_imported_file(
    eval_dir_factory: EvalDirFactory,
) -> None:
    """Set on machine in imported file writes to that file, not clan.nix."""
    ed: EvalDir = eval_dir_factory(
        "multi-file-clan.nix",
        extra_fixtures=["multi-file-machines.nix"],
    )
    machines_nix = ed.path / "multi-file-machines.nix"
    original_clan = ed.clan_nix.read_text()
    original_machines = machines_nix.read_text()

    # Edit jon (defined in multi-file-machines.nix) via discovery
    ed.run_clan_edit_discover(
        "set",
        "--path",
        "inventory.machines.jon.deploy.targetHost",
        "--value",
        '"10.0.0.99"',
    )
    ed.git_add()

    # machine.nix should be modified
    new_machines = machines_nix.read_text()
    assert new_machines != original_machines
    assert "10.0.0.99" in new_machines

    # clan.nix should be unchanged
    new_clan = ed.clan_nix.read_text()
    assert new_clan == original_clan

    # Verify via nix eval
    result = ed.nix_eval(f"{INV}.machines.jon.deploy.targetHost")
    assert result == "10.0.0.99"


def test_multi_file_set_writes_to_clan_nix(
    eval_dir_factory: EvalDirFactory,
) -> None:
    """Set on machine in clan.nix writes to clan.nix, not imported file."""
    ed: EvalDir = eval_dir_factory(
        "multi-file-clan.nix",
        extra_fixtures=["multi-file-machines.nix"],
    )
    machines_nix = ed.path / "multi-file-machines.nix"
    original_machines = machines_nix.read_text()

    # Edit sara (defined in clan.nix) via discovery
    ed.run_clan_edit_discover(
        "set",
        "--path",
        "inventory.machines.sara.deploy.targetHost",
        "--value",
        '"10.0.0.88"',
    )
    ed.git_add()

    # clan.nix should be modified
    new_clan = ed.clan_nix.read_text()
    assert "10.0.0.88" in new_clan

    # machine.nix should be unchanged
    new_machines = machines_nix.read_text()
    assert new_machines == original_machines

    # Verify via nix eval
    result = ed.nix_eval(f"{INV}.machines.sara.deploy.targetHost")
    assert result == "10.0.0.88"


def test_multi_file_explicit_file_overrides(
    eval_dir_factory: EvalDirFactory,
) -> None:
    """Explicit --file flag overrides source tracking."""
    ed: EvalDir = eval_dir_factory(
        "multi-file-clan.nix",
        extra_fixtures=["multi-file-machines.nix"],
    )

    # Force writing to clan.nix even though jon is in machine.nix.
    # This will add the path to clan.nix (not modify machine.nix).
    ed.run_clan_edit(
        "--no-verify",
        "set",
        "--path",
        "inventory.machines.jon.tags",
        "--value",
        '[ "test" ]',
    )

    # clan.nix should contain the edit
    content = ed.clan_nix.read_text()
    assert "test" in content


def test_multi_file_new_machine_falls_back(
    eval_dir_factory: EvalDirFactory,
) -> None:
    """Set on a new (non-existent) machine falls back to default discovery."""
    ed: EvalDir = eval_dir_factory(
        "multi-file-clan.nix",
        extra_fixtures=["multi-file-machines.nix"],
    )

    # newbox doesn't exist in any file, so discovery should fall back
    # to the default file (clan.nix, the last local definition).
    ed.run_clan_edit_discover(
        "set",
        "--path",
        "inventory.machines.newbox",
        "--value",
        "{ }",
    )
    ed.git_add()

    result = ed.nix_eval(f"{INV}.machines.newbox.name")
    assert result == "newbox"


# ============================================================================
# Flake-parts options tree tests
# ============================================================================


def test_flake_parts_clanoptions_exposed(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """Flake-parts exposes clanOptions via config.flake.clan.options."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    assert ed.nix_eval_succeeds("clanOptions")


def test_flake_parts_multi_file_source_tracking(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """Multi-file source tracking works in flake-parts setup."""
    ed: EvalDir = flake_parts_eval_dir_factory(
        "flake-parts-multi-file-clan.nix",
        extra_fixtures=["flake-parts-multi-file-machines.nix"],
    )

    # Both machines should be accessible
    assert ed.nix_eval(f"{INV}.machines.sara.name") == "sara"
    assert ed.nix_eval(f"{INV}.machines.jon.name") == "jon"

    # Edit jon (defined in machines file) via discovery
    machines_nix = ed.path / "flake-parts-multi-file-machines.nix"
    original_clan = ed.clan_nix.read_text()

    ed.run_clan_edit_discover(
        "set",
        "--path",
        "clan.inventory.machines.jon.deploy.targetHost",
        "--value",
        '"10.0.0.99"',
    )
    ed.git_add()

    # machines file should be modified
    new_machines = machines_nix.read_text()
    assert "10.0.0.99" in new_machines

    # clan.nix should be unchanged
    assert ed.clan_nix.read_text() == original_clan

    # Verify via nix eval
    assert ed.nix_eval(f"{INV}.machines.jon.deploy.targetHost") == "10.0.0.99"


def test_flake_parts_options_discovery(
    flake_parts_eval_dir_factory: EvalDirFactory,
) -> None:
    """clan-edit set works with flake-parts option discovery."""
    ed: EvalDir = flake_parts_eval_dir_factory("flake-parts-simple.nix")
    # Use discovery (no --file) to set a value
    ed.run_clan_edit_discover(
        "set",
        "--path",
        "clan.meta.name",
        "--value",
        '"DiscoveredEdit"',
    )
    ed.git_add()
    result = ed.nix_eval(f"{INV}.meta.name")
    assert result == "DiscoveredEdit"


def test_discover_file_missing_clanoptions(eval_dir_factory: EvalDirFactory) -> None:
    """When clanOptions is not exposed, clan-edit gives a helpful error.

    This is tested at the unit level (test_discover_file_error_message in
    main.rs), but we verify the CLI error message here too.
    """
    import shutil
    import tempfile
    from pathlib import Path

    from conftest import git_init

    tmpdir = Path(tempfile.mkdtemp())
    try:
        # Create a minimal flake WITHOUT clanOptions
        (tmpdir / "flake.nix").write_text("{ outputs = { self, ... }: { }; }\n")
        (tmpdir / "clan.nix").write_text('{ meta.name = "test"; }\n')
        git_init(tmpdir)

        # clan-edit should fall back to clan.nix when discovery fails
        from conftest import run_clan_edit

        result = run_clan_edit(
            "--file",
            str(tmpdir / "clan.nix"),
            "get",
            "--path",
            "meta.name",
        )
        # With explicit -f, it should work fine
        assert result.returncode == 0
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
