use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use clan_edit::ast;

#[derive(Parser)]
#[command(name = "clan-edit", about = "Edit clan.nix inventory files")]
struct Cli {
    /// Path to the clan.nix file
    #[arg(short, long, default_value = "clan.nix")]
    file: PathBuf,

    /// Skip nix eval verification after writes
    #[arg(long)]
    no_verify: bool,

    /// Flake directory for verification (default: auto-detect from file location)
    #[arg(long)]
    flake: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Get the value at an attribute path
    Get {
        /// Dot-separated attribute path (e.g., meta.name)
        #[arg(long)]
        path: String,
    },
    /// Set a value at an attribute path
    Set {
        /// Dot-separated attribute path
        #[arg(long)]
        path: String,
        /// Nix value to set (e.g., '"hello"', '{ }', '[ "a" ]')
        #[arg(long)]
        value: String,
    },
    /// Delete an attribute at a path
    Delete {
        /// Dot-separated attribute path
        #[arg(long)]
        path: String,
    },
}

fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

/// Walk up from a file's parent directory looking for flake.nix.
pub fn find_flake_root(file_path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(file_path).ok()?;
    let mut dir = canonical.parent()?;
    loop {
        if dir.join("flake.nix").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Map a clan.nix attribute path to the corresponding flake output path under
/// `clan.inventory`.
///
/// Examples:
///   "meta.name"                    → "meta.name"
///   "inventory.machines.test"      → "machines.test"
///   "clan.meta.name"               → "meta.name"        (flake-parts)
///   "clan.inventory.machines.test" → "machines.test"     (flake-parts)
fn map_to_inventory_path(attr_path: &str) -> &str {
    let p = attr_path.strip_prefix("clan.").unwrap_or(attr_path);
    p.strip_prefix("inventory.").unwrap_or(p)
}

/// Run `nix eval` on a flake attribute and check it succeeds.
fn nix_eval_attr(flake_dir: &Path, attr: &str) -> Result<()> {
    let flake_ref = format!("path:{}#{}", flake_dir.display(), attr);
    let output = Command::new("nix")
        .args(["eval", &flake_ref, "--no-warn-dirty"])
        .output()
        .context("failed to run nix eval")?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix eval verification failed:\n{stderr}")
    }
}

/// Verify by evaluating `clan.inventory` (basic sanity) and, when an edit path
/// is known, additionally evaluate the specific edited attribute.  The targeted
/// eval forces the Nix module system to type-check that particular entry — a
/// plain lazy eval of the whole inventory would miss nested type errors.
fn nix_eval_verify(flake_dir: &Path, attr_path: Option<&str>) -> Result<()> {
    // Basic sanity: the inventory as a whole must evaluate
    nix_eval_attr(flake_dir, "clan.inventory")?;

    // Targeted eval: force the specific path that was edited
    if let Some(ap) = attr_path {
        let inv_path = map_to_inventory_path(ap);
        let full_attr = format!("clan.inventory.{inv_path}");
        nix_eval_attr(flake_dir, &full_attr)?;
    }

    Ok(())
}

/// Write new content to a file, then verify with nix eval.
/// If verification fails, restore the original content and return the error.
/// `attr_path` is the clan.nix attribute path that was edited, used for
/// targeted verification of the specific change.
fn write_and_verify(
    path: &Path,
    new_content: &str,
    original: &str,
    no_verify: bool,
    flake_override: Option<&Path>,
    attr_path: Option<&str>,
) -> Result<()> {
    write_file(path, new_content)?;

    if no_verify {
        return Ok(());
    }

    // Check if nix is available
    if Command::new("nix").arg("--version").output().is_err() {
        eprintln!("warning: nix not found in PATH, skipping verification");
        return Ok(());
    }

    // Find flake root
    let flake_dir = match flake_override {
        Some(dir) => Some(dir.to_path_buf()),
        None => find_flake_root(path),
    };

    let Some(flake_dir) = flake_dir else {
        eprintln!("warning: no flake.nix found, skipping verification");
        return Ok(());
    };

    // Verify; rollback on failure
    match nix_eval_verify(&flake_dir, attr_path) {
        Ok(()) => Ok(()),
        Err(eval_err) => {
            // Rollback
            if let Err(rollback_err) = write_file(path, original) {
                bail!(
                    "verification failed AND rollback failed!\n\
                     Eval error: {eval_err}\n\
                     Rollback error: {rollback_err}"
                );
            }
            Err(eval_err)
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let flake_override = cli.flake.as_deref();

    match &cli.command {
        Commands::Get { path } => {
            let source = read_file(&cli.file)?;
            let root = ast::parse_nix(&source)?;
            let value = ast::get_attr(&root, path)?;
            println!("{value}");
        }
        Commands::Set { path, value } => {
            let source = read_file(&cli.file)?;
            let result = ast::set_attr(&source, path, value)?;
            write_and_verify(&cli.file, &result, &source, cli.no_verify, flake_override, Some(path))?;
        }
        Commands::Delete { path } => {
            let source = read_file(&cli.file)?;
            let result = ast::delete_attr(&source, path)?;
            write_and_verify(&cli.file, &result, &source, cli.no_verify, flake_override, None)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_flake_root_same_dir() {
        let tmpdir = tempfile::tempdir().unwrap();
        fs::write(tmpdir.path().join("flake.nix"), "{}").unwrap();
        fs::write(tmpdir.path().join("clan.nix"), "{}").unwrap();

        let result = find_flake_root(&tmpdir.path().join("clan.nix"));
        assert_eq!(result.unwrap(), tmpdir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_find_flake_root_parent_dir() {
        let tmpdir = tempfile::tempdir().unwrap();
        fs::write(tmpdir.path().join("flake.nix"), "{}").unwrap();
        let subdir = tmpdir.path().join("config");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("clan.nix"), "{}").unwrap();

        let result = find_flake_root(&subdir.join("clan.nix"));
        assert_eq!(result.unwrap(), tmpdir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_map_to_inventory_path() {
        // Standard clan.nix paths
        assert_eq!(map_to_inventory_path("meta.name"), "meta.name");
        assert_eq!(map_to_inventory_path("inventory.machines.test"), "machines.test");
        assert_eq!(map_to_inventory_path("inventory.instances.sshd"), "instances.sshd");

        // Flake-parts paths (clan. prefix)
        assert_eq!(map_to_inventory_path("clan.meta.name"), "meta.name");
        assert_eq!(map_to_inventory_path("clan.inventory.machines.test"), "machines.test");
    }

    #[test]
    fn test_find_flake_root_not_found() {
        let tmpdir = tempfile::tempdir().unwrap();
        fs::write(tmpdir.path().join("clan.nix"), "{}").unwrap();

        let result = find_flake_root(&tmpdir.path().join("clan.nix"));
        // Might find a flake.nix somewhere above /tmp, so just verify it doesn't
        // return the tmpdir itself (which has no flake.nix)
        if let Some(found) = result {
            assert_ne!(found, tmpdir.path().canonicalize().unwrap());
        }
    }
}
