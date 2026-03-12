#!/usr/bin/env bash
# Integration tests for clan-edit: validates edits via nix eval against clan-core
set -euo pipefail

# CLAN_CORE_PATH must be set (to clan-core flake store path or local checkout)
: "${CLAN_CORE_PATH:?CLAN_CORE_PATH must be set to the clan-core flake path}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/../.."
FIXTURES_DIR="${FIXTURES_DIR:-${SCRIPT_DIR}/../fixtures}"

# Use clan-edit from PATH, or fall back to cargo run
if command -v clan-edit > /dev/null 2>&1; then
  CLAN_EDIT="clan-edit"
else
  CLAN_EDIT="cargo run --quiet --manifest-path ${PROJECT_DIR}/Cargo.toml --"
fi

pass=0
fail=0

test_pass() {
  echo "  PASS: $1"
  pass=$((pass + 1))
}

test_fail() {
  echo "  FAIL: $1"
  echo "    $2"
  fail=$((fail + 1))
}

# Helper: create a temporary directory with a wrapping flake that imports clan.nix
setup_eval_dir() {
  local clan_nix_file="$1"
  local tmpdir
  tmpdir="$(mktemp -d)"

  # Copy the clan.nix to the temp dir (ensure writable for editing)
  cp "$clan_nix_file" "$tmpdir/clan.nix"
  chmod u+w "$tmpdir/clan.nix"

  # Create a flake.nix that uses clan-core.lib.clan and imports the clan.nix
  cat > "$tmpdir/flake.nix" << FLAKE_EOF
{
  inputs = {
    clan-core.url = "path:${CLAN_CORE_PATH}";
    nixpkgs.follows = "clan-core/nixpkgs";
  };

  outputs = { self, clan-core, ... }:
    let
      clanConfig = import ./clan.nix;
      clan = clan-core.lib.clan ({
        inherit self;
      } // clanConfig);
    in
    {
      clan = clan.config;
    };
}
FLAKE_EOF

  # Initialize git repo (required for flakes)
  git -C "$tmpdir" init -q
  git -C "$tmpdir" add .

  echo "$tmpdir"
}

# Helper: evaluate a specific attribute in the wrapping flake (must be JSON-serializable)
nix_eval() {
  local flake_dir="$1"
  local attr="$2"
  nix eval "path:${flake_dir}#${attr}" --json --no-warn-dirty 2>/dev/null
}

# Helper: check that a specific attribute evaluates successfully
nix_eval_succeeds() {
  local flake_dir="$1"
  local attr="$2"
  nix eval "path:${flake_dir}#${attr}" --json --no-warn-dirty > /dev/null 2>/dev/null
}

INV="clan.inventory"

# Helper: create a temporary directory with a flake-parts wrapping flake
# The clan.nix file is imported as a flake-parts module that sets `clan = { ... }`
setup_flake_parts_eval_dir() {
  local clan_nix_file="$1"
  local tmpdir
  tmpdir="$(mktemp -d)"

  # Copy the clan.nix to the temp dir (ensure writable for editing)
  cp "$clan_nix_file" "$tmpdir/clan.nix"
  chmod u+w "$tmpdir/clan.nix"

  # Create a flake.nix that uses flake-parts with clan-core's flakeModule
  cat > "$tmpdir/flake.nix" << FLAKE_EOF
{
  inputs = {
    clan-core.url = "path:${CLAN_CORE_PATH}";
    nixpkgs.follows = "clan-core/nixpkgs";
    flake-parts.follows = "clan-core/flake-parts";
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" ];
      imports = [
        inputs.clan-core.flakeModules.default
        ./clan.nix
      ];
    };
}
FLAKE_EOF

  # Initialize git repo (required for flakes)
  git -C "$tmpdir" init -q
  git -C "$tmpdir" add .

  echo "$tmpdir"
}

###############################################################################
# Test 1: minimal clan.nix evaluates successfully
###############################################################################
echo "Test 1: Minimal clan.nix evaluates via nix eval"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  test_pass "minimal clan.nix evaluates"
else
  test_fail "minimal clan.nix evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 2: edit meta.name, verify value changes
###############################################################################
echo "Test 2: Edit meta.name, verify via nix eval"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path meta.name --value '"EditedClan"'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
if [ "$result" = '"EditedClan"' ]; then
  test_pass "meta.name updated to EditedClan"
else
  test_fail "meta.name updated" "expected '\"EditedClan\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 3: add a machine, verify it appears
###############################################################################
echo "Test 3: Add machine, verify via nix eval"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path "inventory.machines.testbox" --value '{ }'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.machines.testbox.name")" || true
if [ "$result" = '"testbox"' ]; then
  test_pass "machine testbox added"
else
  test_fail "machine testbox added" "expected '\"testbox\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 4: add a service instance, verify it appears
###############################################################################
echo "Test 4: Add service instance, verify via nix eval"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path "inventory.instances.myservice" --value '{ }'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  test_pass "instance myservice added and evaluates"
else
  test_fail "instance myservice added" "nix eval failed after adding instance"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 5: with-inventory fixture evaluates
###############################################################################
echo "Test 5: Full inventory fixture evaluates"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/with-inventory.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"InventoryClan"' ]; then
    test_pass "with-inventory fixture evaluates"
  else
    test_fail "with-inventory fixture evaluates" "meta.name expected '\"InventoryClan\"', got '$result'"
  fi
else
  test_fail "with-inventory fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 6: per-machine settings fixture evaluates
###############################################################################
echo "Test 6: Per-machine settings fixture evaluates"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/per-machine-settings.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"PerMachineClan"' ]; then
    test_pass "per-machine-settings fixture evaluates"
  else
    test_fail "per-machine-settings fixture evaluates" "meta.name expected '\"PerMachineClan\"', got '$result'"
  fi
else
  test_fail "per-machine-settings fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 7: let bindings survive edits
###############################################################################
echo "Test 7: Let bindings survive edits"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/with-let-bindings.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path meta.name --value '"ModifiedLetClan"'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"ModifiedLetClan"' ]; then
    test_pass "let bindings survive edits"
  else
    test_fail "let bindings survive edits" "meta.name expected '\"ModifiedLetClan\"', got '$result'"
  fi
else
  test_fail "let bindings survive edits" "nix eval failed after editing file with let bindings"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 8: set a role setting, verify via nix eval
###############################################################################
echo "Test 8: Set role setting, verify via nix eval"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/with-inventory.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path inventory.instances.sshd.roles.server.settings.authorizedKeys.newkey --value '"ssh-ed25519 AAAA-test-key"'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  test_pass "role setting added and evaluates"
else
  test_fail "role setting added" "nix eval failed after setting role setting"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 9: invalid edit causes nix eval to fail (negative test)
###############################################################################
echo "Test 9: Invalid structure causes nix eval failure (negative test)"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
# Set meta.name to a non-string value (should be a string, not an attrset)
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path meta.name --value '42'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  # nix eval succeeded - check if the value is invalid type (it accepted 42 as a name, which is wrong)
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '42' ]; then
    # Nix module system may accept ints for strings in some cases
    test_fail "invalid type rejected" "nix eval accepted integer for meta.name (expected type error)"
  else
    test_pass "invalid structure detected"
  fi
else
  test_pass "invalid structure causes nix eval failure"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 10: flake-parts-simple fixture evaluates via clan.inventory
###############################################################################
echo "Test 10: Flake-parts simple fixture evaluates via clan.inventory"
eval_dir="$(setup_flake_parts_eval_dir "$FIXTURES_DIR/flake-parts-simple.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"friedow"' ]; then
    test_pass "flake-parts-simple fixture evaluates"
  else
    test_fail "flake-parts-simple fixture evaluates" "meta.name expected '\"friedow\"', got '$result'"
  fi
else
  test_fail "flake-parts-simple fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 11: flake-parts-complex fixture evaluates via clan.inventory
###############################################################################
echo "Test 11: Flake-parts complex fixture evaluates via clan.inventory"
eval_dir="$(setup_flake_parts_eval_dir "$FIXTURES_DIR/flake-parts-complex.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"heliosphere"' ]; then
    test_pass "flake-parts-complex fixture evaluates"
  else
    test_fail "flake-parts-complex fixture evaluates" "meta.name expected '\"heliosphere\"', got '$result'"
  fi
else
  test_fail "flake-parts-complex fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 12: clan-edit add-machine on flake-parts fixture
###############################################################################
echo "Test 12: Add machine on flake-parts fixture, verify via nix eval"
eval_dir="$(setup_flake_parts_eval_dir "$FIXTURES_DIR/flake-parts-simple.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path clan.inventory.machines.newbox.name --value '"newbox"'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.machines.newbox.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.machines.newbox.name")" || true
  if [ "$result" = '"newbox"' ]; then
    test_pass "machine added to flake-parts fixture"
  else
    test_fail "machine added to flake-parts fixture" "expected '\"newbox\"', got '$result'"
  fi
else
  test_fail "machine added to flake-parts fixture" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 13: clan-edit set on flake-parts fixture
###############################################################################
echo "Test 13: Set meta.name on flake-parts fixture, verify via nix eval"
eval_dir="$(setup_flake_parts_eval_dir "$FIXTURES_DIR/flake-parts-simple.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path clan.meta.name --value '"EditedFlakeParts"'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
if [ "$result" = '"EditedFlakeParts"' ]; then
  test_pass "meta.name updated on flake-parts fixture"
else
  test_fail "meta.name updated on flake-parts fixture" "expected '\"EditedFlakeParts\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 14: valid edit succeeds with verification enabled
###############################################################################
echo "Test 14: Valid edit with verification succeeds"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
# No --no-verify: clan-edit will auto-detect the flake and run nix eval
$CLAN_EDIT --file "$eval_dir/clan.nix" set --path meta.name --value '"VerifiedName"'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
if [ "$result" = '"VerifiedName"' ]; then
  test_pass "valid edit with verification"
else
  test_fail "valid edit with verification" "expected '\"VerifiedName\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 15: invalid edit is rolled back with verification enabled
###############################################################################
echo "Test 15: Invalid edit is rolled back by verification"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
original_name="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
# Try to set meta.name to an undefined variable (should fail verification)
if $CLAN_EDIT --file "$eval_dir/clan.nix" set --path meta.name --value 'UndefinedVar' 2>/dev/null; then
  test_fail "invalid edit rolled back" "clan-edit should have exited non-zero"
else
  # Verify the file was restored to its original content
  git -C "$eval_dir" add -A
  restored_name="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$restored_name" = "$original_name" ]; then
    test_pass "invalid edit rolled back"
  else
    test_fail "invalid edit rolled back" "file not restored, got '$restored_name' instead of '$original_name'"
  fi
fi
rm -rf "$eval_dir"

###############################################################################
# Test 16: --no-verify allows invalid edit to persist
###############################################################################
echo "Test 16: --no-verify allows invalid edit to persist"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
# With --no-verify, the invalid value should be written without rollback
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path meta.name --value 'UndefinedVar'
# The file should contain UndefinedVar (not rolled back)
if grep -q 'UndefinedVar' "$eval_dir/clan.nix"; then
  test_pass "--no-verify allows invalid edit"
else
  test_fail "--no-verify allows invalid edit" "UndefinedVar not found in file"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 17: special-names fixture evaluates
###############################################################################
echo "Test 17: Special names fixture evaluates"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/special-names.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"SpecialNamesClan"' ]; then
    test_pass "special-names fixture evaluates"
  else
    test_fail "special-names fixture evaluates" "meta.name expected '\"SpecialNamesClan\"', got '$result'"
  fi
else
  test_fail "special-names fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 18: add machine with name that needs quoting (starts with digit)
###############################################################################
echo "Test 18: Add machine with digit-prefixed name"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path 'inventory.machines.3rd-node' --value '{ }'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.machines.\"3rd-node\".name")" || true
if [ "$result" = '"3rd-node"' ]; then
  test_pass "machine with digit-prefixed name added"
else
  test_fail "machine with digit-prefixed name added" "expected '\"3rd-node\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 19: add machine with space in name
###############################################################################
echo "Test 19: Add machine with space in name"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/minimal.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path 'inventory.machines.my server' --value '{ }'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.machines.\"my server\".name")" || true
if [ "$result" = '"my server"' ]; then
  test_pass "machine with space in name added"
else
  test_fail "machine with space in name added" "expected '\"my server\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 20: get/set on existing special-name machine
###############################################################################
echo "Test 20: Set value on existing quoted-key machine"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/special-names.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path 'inventory.machines.webserver 2.deploy.targetHost' --value '"10.0.0.99"'
git -C "$eval_dir" add -A
result="$(nix_eval "$eval_dir" "${INV}.machines.\"webserver 2\".deploy.targetHost")" || true
if [ "$result" = '"10.0.0.99"' ]; then
  test_pass "set value on quoted-key machine"
else
  test_fail "set value on quoted-key machine" "expected '\"10.0.0.99\"', got '$result'"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 21: delete a special-name machine
###############################################################################
echo "Test 21: Delete quoted-key machine"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/special-names.nix")"
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" delete --path 'inventory.machines.2nd-backup'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  # Verify the machine is gone
  if nix_eval_succeeds "$eval_dir" "${INV}.machines.\"2nd-backup\""; then
    test_fail "delete quoted-key machine" "machine still exists after delete"
  else
    test_pass "delete quoted-key machine"
  fi
else
  test_fail "delete quoted-key machine" "nix eval failed after delete"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 22: instance-let-binding fixture evaluates
###############################################################################
echo "Test 22: Instance let-binding fixture evaluates"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/instance-let-binding.nix")"
if nix_eval_succeeds "$eval_dir" "${INV}.meta.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.meta.name")" || true
  if [ "$result" = '"LetInstanceClan"' ]; then
    test_pass "instance-let-binding fixture evaluates"
  else
    test_fail "instance-let-binding fixture evaluates" "meta.name expected '\"LetInstanceClan\"', got '$result'"
  fi
else
  test_fail "instance-let-binding fixture evaluates" "nix eval failed"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 23: editing around a let-in instance preserves it
###############################################################################
echo "Test 23: Editing around let-in instance preserves it"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/instance-let-binding.nix")"
# Add a new machine (doesn't touch the let-in instance)
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path 'inventory.machines.newbox' --value '{ }'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.machines.newbox.name"; then
  # Verify the let-in instance still evaluates (commonKey must resolve)
  if nix_eval_succeeds "$eval_dir" "${INV}.instances.sshd.module.name"; then
    # Verify the let keyword is still in the file (not corrupted)
    if grep -q 'commonKey' "$eval_dir/clan.nix"; then
      test_pass "let-in instance preserved after editing elsewhere"
    else
      test_fail "let-in instance preserved" "let binding variable 'commonKey' missing from file"
    fi
  else
    test_fail "let-in instance preserved" "sshd instance no longer evaluates after editing elsewhere"
  fi
else
  test_fail "let-in instance preserved" "nix eval failed after adding machine"
fi
rm -rf "$eval_dir"

###############################################################################
# Test 24: replacing a let-in instance value wholesale
###############################################################################
echo "Test 24: Replace let-in instance value wholesale"
eval_dir="$(setup_eval_dir "$FIXTURES_DIR/instance-let-binding.nix")"
# Replace the entire sshd instance (which is a let...in) with a plain attrset
$CLAN_EDIT --no-verify --file "$eval_dir/clan.nix" set --path 'inventory.instances.sshd' --value '{
    module = {
      name = "sshd";
      input = "clan-core";
    };
    roles.server.tags.all = { };
  }'
git -C "$eval_dir" add -A
if nix_eval_succeeds "$eval_dir" "${INV}.instances.sshd.module.name"; then
  result="$(nix_eval "$eval_dir" "${INV}.instances.sshd.module.name")" || true
  if [ "$result" = '"sshd"' ]; then
    # Verify the let binding is gone from the file
    if grep -q 'commonKey' "$eval_dir/clan.nix"; then
      test_fail "replace let-in instance" "let binding still present in file"
    else
      test_pass "replace let-in instance"
    fi
  else
    test_fail "replace let-in instance" "module.name expected '\"sshd\"', got '$result'"
  fi
else
  test_fail "replace let-in instance" "nix eval failed after replacing instance"
fi
rm -rf "$eval_dir"

###############################################################################
# Summary
###############################################################################
echo ""
echo "Results: $pass passed, $fail failed out of $((pass + fail)) tests"

if [ "$fail" -gt 0 ]; then
  exit 1
fi
