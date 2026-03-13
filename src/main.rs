use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use clan_edit::ast;

#[derive(Parser)]
#[command(name = "clan-edit", about = "Edit clan.nix inventory files")]
struct Cli {
    /// Path to the clan.nix file (auto-discovered via definitionsWithLocations if omitted)
    #[arg(short, long)]
    file: Option<PathBuf>,

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

/// Walk up from a directory looking for flake.nix.
pub fn find_flake_root_from_dir(dir: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(dir).ok()?;
    let mut d = canonical.as_path();
    loop {
        if d.join("flake.nix").is_file() {
            return Some(d.to_path_buf());
        }
        d = d.parent()?;
    }
}

/// Run `nix eval` to extract all definition file paths from a
/// `definitionsWithLocations` attribute.
fn nix_eval_definition_files(flake_dir: &Path, attr: &str) -> Result<Vec<String>> {
    let flake_ref = format!("path:{}#{}", flake_dir.display(), attr);
    let output = Command::new("nix")
        .args([
            "eval",
            &flake_ref,
            "--apply",
            "defs: map (d: d.file) defs",
            "--json",
            "--no-warn-dirty",
        ])
        .output()
        .context("failed to run nix eval for option discovery")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> =
            serde_json::from_str(&stdout).context("failed to parse definition files as JSON")?;
        Ok(files)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("nix eval failed for {attr}:\n{stderr}")
    }
}

/// Convert a Nix store path to a real filesystem path relative to the flake
/// directory.  Store paths look like `/nix/store/<hash>-source/subdir/file.nix`;
/// we strip the store prefix and join with `flake_dir`.
fn store_path_to_real_path(store_file: &str, flake_dir: &Path) -> PathBuf {
    if let Some(rest) = store_file.strip_prefix("/nix/store/") {
        // Skip the <hash>-<name> component
        if let Some(slash_pos) = rest.find('/') {
            let relative = &rest[slash_pos + 1..];
            return flake_dir.join(relative);
        }
    }
    // Fallback: assume it's already a real path
    PathBuf::from(store_file)
}

/// From a list of store paths returned by `definitionsWithLocations`, find the
/// one that maps to a real file in the user's flake directory.  Definitions from
/// flake inputs (clan-core internals) won't exist on disk so we skip them.
fn find_local_definition(files: &[String], flake_dir: &Path) -> Option<PathBuf> {
    for store_file in files.iter().rev() {
        let real = store_path_to_real_path(store_file, flake_dir);
        if real.is_file() {
            return Some(real);
        }
    }
    None
}

/// Discover the file that defines inventory options by evaluating
/// `definitionsWithLocations` from the flake outputs.
///
/// Tries `clanOptions.inventory.definitionsWithLocations` (non-flake-parts)
/// first, then `clan.options.inventory.definitionsWithLocations` (flake-parts).
///
/// From the returned definitions, picks the one that maps to an actual file in
/// the user's flake directory (skipping clan-core internal modules).
fn discover_file(flake_dir: &Path) -> Result<PathBuf> {
    // Try non-flake-parts
    if let Ok(files) =
        nix_eval_definition_files(flake_dir, "clanOptions.inventory.definitionsWithLocations")
    {
        if let Some(path) = find_local_definition(&files, flake_dir) {
            return Ok(path);
        }
    }

    // Try flake-parts
    if let Ok(files) =
        nix_eval_definition_files(flake_dir, "clan.options.inventory.definitionsWithLocations")
    {
        if let Some(path) = find_local_definition(&files, flake_dir) {
            return Ok(path);
        }
    }

    bail!(
        "clanOptions not exposed in flake outputs. \
         Add `clanOptions = clan.options;` to your flake.nix outputs."
    )
}

/// Resolve the target file to edit: use `-f` if provided, otherwise attempt
/// option discovery, falling back to `clan.nix`.
fn resolve_file(explicit_file: Option<&Path>, flake_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(f) = explicit_file {
        return Ok(f.to_path_buf());
    }

    // Try discovery
    let cwd = std::env::current_dir().context("cannot get current directory")?;
    let flake_dir = flake_override
        .map(|p| p.to_path_buf())
        .or_else(|| find_flake_root_from_dir(&cwd));

    if let Some(flake_dir) = &flake_dir {
        match discover_file(flake_dir) {
            Ok(path) => return Ok(path),
            Err(_) => {
                // Discovery failed; fall back to clan.nix in flake dir
            }
        }
    }

    // Default: clan.nix in flake directory (if known) or current directory
    match flake_dir {
        Some(dir) => Ok(dir.join("clan.nix")),
        None => Ok(PathBuf::from("clan.nix")),
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
    let file = resolve_file(cli.file.as_deref(), flake_override)?;

    match &cli.command {
        Commands::Get { path } => {
            let source = read_file(&file)?;
            let root = ast::parse_nix(&source)?;
            let value = ast::get_attr(&root, path)?;
            println!("{value}");
        }
        Commands::Set { path, value } => {
            let source = read_file(&file)?;
            let result = ast::set_attr(&source, path, value)?;
            write_and_verify(
                &file,
                &result,
                &source,
                cli.no_verify,
                flake_override,
                Some(path),
            )?;
        }
        Commands::Delete { path } => {
            let source = read_file(&file)?;
            let result = ast::delete_attr(&source, path)?;
            write_and_verify(&file, &result, &source, cli.no_verify, flake_override, None)?;
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
        assert_eq!(
            map_to_inventory_path("inventory.machines.test"),
            "machines.test"
        );
        assert_eq!(
            map_to_inventory_path("inventory.instances.sshd"),
            "instances.sshd"
        );

        // Flake-parts paths (clan. prefix)
        assert_eq!(map_to_inventory_path("clan.meta.name"), "meta.name");
        assert_eq!(
            map_to_inventory_path("clan.inventory.machines.test"),
            "machines.test"
        );
    }

    #[test]
    fn test_store_path_to_real_path() {
        let flake_dir = Path::new("/home/user/project");

        // Normal store path
        let store = "/nix/store/abc123-source/clan.nix";
        assert_eq!(
            store_path_to_real_path(store, flake_dir),
            PathBuf::from("/home/user/project/clan.nix")
        );

        // Store path with subdirectory
        let store = "/nix/store/abc123-source/config/inventory.nix";
        assert_eq!(
            store_path_to_real_path(store, flake_dir),
            PathBuf::from("/home/user/project/config/inventory.nix")
        );

        // Already a real path (fallback)
        let real = "/home/user/project/clan.nix";
        assert_eq!(
            store_path_to_real_path(real, flake_dir),
            PathBuf::from("/home/user/project/clan.nix")
        );
    }

    #[test]
    fn test_discover_file_error_message() {
        // When clanOptions is not exposed, discover_file should return a
        // helpful error message
        let tmpdir = tempfile::tempdir().unwrap();
        // Create a minimal flake that does NOT expose clanOptions
        fs::write(
            tmpdir.path().join("flake.nix"),
            r#"{ outputs = { self, ... }: { }; }"#,
        )
        .unwrap();

        // Initialize git repo (required for flakes)
        let _ = Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmpdir.path())
            .output();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(tmpdir.path())
            .output();

        let result = discover_file(tmpdir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("clanOptions not exposed"),
            "expected clanOptions error, got: {msg}"
        );
        assert!(
            msg.contains("clanOptions = clan.options"),
            "expected hint about adding clanOptions, got: {msg}"
        );
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
