"""Shared fixtures for clan-edit integration tests."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from collections.abc import Generator
from pathlib import Path
from typing import Any, Protocol

import pytest  # type: ignore[import-not-found]


class EvalDirFactory(Protocol):
    """Protocol for eval dir factory callables."""

    def __call__(self, fixture_name: str) -> EvalDir: ...


CLAN_CORE_PATH: str = os.environ.get("CLAN_CORE_PATH", "")
FIXTURES_DIR: str = os.environ.get(
    "FIXTURES_DIR",
    str(Path(__file__).parent.parent / "fixtures"),
)
INV: str = "clan.inventory"


def _find_clan_edit() -> str:
    """Find the clan-edit binary."""
    result = shutil.which("clan-edit")
    if result is not None:
        return result
    # Fallback: cargo run
    project_dir = str(Path(__file__).parent.parent.parent)
    return f"cargo run --quiet --manifest-path {project_dir}/Cargo.toml --"


CLAN_EDIT_CMD: str = _find_clan_edit()


def run_clan_edit(
    *args: str,
    check: bool = True,
    capture_output: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Run clan-edit with the given arguments."""
    cmd_parts: list[str] = CLAN_EDIT_CMD.split() + list(args)
    return subprocess.run(
        cmd_parts,
        check=check,
        capture_output=capture_output,
        text=True,
    )


def nix_eval(flake_dir: Path, attr: str) -> Any:
    """Evaluate a flake attribute and return the JSON-parsed result."""
    flake_ref = f"path:{flake_dir}#{attr}"
    result = subprocess.run(
        ["nix", "eval", flake_ref, "--json", "--no-warn-dirty"],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def nix_eval_succeeds(flake_dir: Path, attr: str) -> bool:
    """Check if a flake attribute evaluates successfully."""
    flake_ref = f"path:{flake_dir}#{attr}"
    result = subprocess.run(
        ["nix", "eval", flake_ref, "--json", "--no-warn-dirty"],
        capture_output=True,
        text=True,
    )
    return result.returncode == 0


def git_init(directory: Path) -> None:
    """Initialize a git repo and add all files."""
    subprocess.run(
        ["git", "init", "-q"],
        cwd=directory,
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["git", "add", "."],
        cwd=directory,
        check=True,
        capture_output=True,
    )


def git_add_all(directory: Path) -> None:
    """Stage all changes in a git repo."""
    subprocess.run(
        ["git", "add", "-A"],
        cwd=directory,
        check=True,
        capture_output=True,
    )


class EvalDir:
    """A temporary directory with a wrapping flake for evaluation."""

    def __init__(self, tmpdir: Path) -> None:
        self.path: Path = tmpdir
        self.clan_nix: Path = tmpdir / "clan.nix"
        self.flake_nix: Path = tmpdir / "flake.nix"

    def git_add(self) -> None:
        """Stage all changes."""
        git_add_all(self.path)

    def nix_eval(self, attr: str) -> Any:
        """Evaluate a flake attribute."""
        return nix_eval(self.path, attr)

    def nix_eval_succeeds(self, attr: str) -> bool:
        """Check if evaluation succeeds."""
        return nix_eval_succeeds(self.path, attr)

    def run_clan_edit(
        self,
        *args: str,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        """Run clan-edit against this eval dir's clan.nix."""
        return run_clan_edit(
            "--file",
            str(self.clan_nix),
            *args,
            check=check,
        )


@pytest.fixture()  # type: ignore[untyped-decorator]
def eval_dir_factory() -> Generator[EvalDirFactory, None, None]:
    """Factory fixture that creates eval dirs from a fixture file."""
    created_dirs: list[Path] = []

    def _create(fixture_name: str) -> EvalDir:
        fixture_path = Path(FIXTURES_DIR) / fixture_name
        assert fixture_path.exists(), f"Fixture not found: {fixture_path}"

        tmpdir = Path(tempfile.mkdtemp())
        created_dirs.append(tmpdir)

        # Copy fixture as clan.nix
        shutil.copy2(fixture_path, tmpdir / "clan.nix")
        (tmpdir / "clan.nix").chmod(0o644)

        # Create wrapping flake.nix (handles both plain attrset and
        # function modules like {lib, ...}: { ... })
        flake_content = f"""{{
  inputs = {{
    clan-core.url = "path:{CLAN_CORE_PATH}";
    nixpkgs.follows = "clan-core/nixpkgs";
  }};

  outputs = {{ self, clan-core, nixpkgs, ... }}:
    let
      rawModule = import ./clan.nix;
      clanConfig =
        if builtins.isFunction rawModule
        then rawModule {{ lib = nixpkgs.lib; }}
        else rawModule;
      clan = clan-core.lib.clan ({{
        inherit self;
      }} // clanConfig);
    in
    {{
      clan = clan.config;
      clanOptions = clan.options;
    }};
}}
"""
        (tmpdir / "flake.nix").write_text(flake_content)
        git_init(tmpdir)
        return EvalDir(tmpdir)

    yield _create

    for d in created_dirs:
        shutil.rmtree(d, ignore_errors=True)


@pytest.fixture()  # type: ignore[untyped-decorator]
def flake_parts_eval_dir_factory() -> Generator[EvalDirFactory, None, None]:
    """Factory fixture that creates flake-parts eval dirs."""
    created_dirs: list[Path] = []

    def _create(fixture_name: str) -> EvalDir:
        fixture_path = Path(FIXTURES_DIR) / fixture_name
        assert fixture_path.exists(), f"Fixture not found: {fixture_path}"

        tmpdir = Path(tempfile.mkdtemp())
        created_dirs.append(tmpdir)

        shutil.copy2(fixture_path, tmpdir / "clan.nix")
        (tmpdir / "clan.nix").chmod(0o644)

        flake_content = f"""{{
  inputs = {{
    clan-core.url = "path:{CLAN_CORE_PATH}";
    nixpkgs.follows = "clan-core/nixpkgs";
    flake-parts.follows = "clan-core/flake-parts";
  }};

  outputs = inputs@{{ flake-parts, ... }}:
    flake-parts.lib.mkFlake {{ inherit inputs; }} {{
      systems = [ "x86_64-linux" "aarch64-linux" ];
      imports = [
        inputs.clan-core.flakeModules.default
        ./clan.nix
      ];
    }};
}}
"""
        (tmpdir / "flake.nix").write_text(flake_content)
        git_init(tmpdir)
        return EvalDir(tmpdir)

    yield _create

    for d in created_dirs:
        shutil.rmtree(d, ignore_errors=True)
