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

/// Run `nix eval` to extract definition file paths from a
/// `definitionsWithLocations` attribute, optionally filtered by sub-path.
///
/// When `filter_segments` is empty, returns all definition files.
/// When non-empty (e.g., `["machines", "jon"]`), only returns files whose
/// definition value contains those nested keys.
fn nix_eval_definition_files(
    flake_dir: &Path,
    attr: &str,
    filter_segments: &[&str],
) -> Result<Vec<String>> {
    let flake_ref = format!("path:{}#{}", flake_dir.display(), attr);
    let apply_expr = build_definition_filter(filter_segments);
    let output = Command::new("nix")
        .args([
            "eval",
            &flake_ref,
            "--apply",
            &apply_expr,
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

/// Build a Nix `--apply` expression that filters `definitionsWithLocations`
/// to find definitions containing a specific inventory sub-path.
///
/// Given inventory-relative path segments like `["machines", "jon"]`, produces:
/// ```nix
/// defs: map (d: d.file) (builtins.filter (d:
///   builtins.isAttrs d.value
///   && d.value ? "machines"
///   && d.value."machines" ? "jon"
/// ) defs)
/// ```
///
/// With no segments, returns all files (no filtering beyond `isAttrs`).
fn build_definition_filter(segments: &[&str]) -> String {
    if segments.is_empty() {
        return "defs: map (d: d.file) defs".to_string();
    }

    let mut conditions = vec!["builtins.isAttrs d.value".to_string()];
    let mut prefix = "d.value".to_string();

    for seg in segments {
        let escaped = seg.replace('\\', "\\\\").replace('"', "\\\"");
        let quoted = format!("\"{escaped}\"");
        conditions.push(format!("{prefix} ? {quoted}"));
        prefix = format!("{prefix}.{quoted}");
    }

    let filter_expr = conditions.join(" && ");
    format!("defs: map (d: d.file) (builtins.filter (d: {filter_expr}) defs)")
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

/// Extract the inventory-relative filter segments from an attribute path.
///
/// For path-specific discovery, we need the first two segments of the
/// inventory-relative path (category + item name, e.g., `["machines", "jon"]`).
///
/// Returns `None` if the path is too short for meaningful filtering.
fn inventory_filter_segments(attr_path: &str) -> Option<Vec<String>> {
    let inv_path = map_to_inventory_path(attr_path);
    let segments: Vec<&str> = inv_path.split('.').collect();
    if segments.len() >= 2 {
        Some(segments[..2].iter().map(|s| s.to_string()).collect())
    } else {
        None
    }
}

/// Try to discover a file using `definitionsWithLocations` from one of the
/// known flake output attributes, optionally filtered by sub-path segments.
fn try_discover(flake_dir: &Path, filter_segments: &[&str]) -> Option<PathBuf> {
    let attrs = [
        "clanOptions.inventory.definitionsWithLocations",
        "clan.options.inventory.definitionsWithLocations",
    ];
    for attr in &attrs {
        if let Ok(files) = nix_eval_definition_files(flake_dir, attr, filter_segments) {
            if let Some(path) = find_local_definition(&files, flake_dir) {
                return Some(path);
            }
        }
    }
    None
}

/// Discover the file that defines a specific attribute path within the
/// inventory, or fall back to general discovery.
///
/// When `attr_path` is provided and has enough segments (e.g.,
/// `inventory.machines.jon.deploy.targetHost`), uses `definitionsWithLocations`
/// with a filter to find the specific file that defines that attribute.
///
/// Falls back to unfiltered discovery (any local inventory definition file),
/// then to a hard error if no options are exposed at all.
fn discover_file(flake_dir: &Path, attr_path: Option<&str>) -> Result<PathBuf> {
    // Try path-specific discovery first
    if let Some(ap) = attr_path {
        if let Some(segments) = inventory_filter_segments(ap) {
            let seg_refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
            if let Some(path) = try_discover(flake_dir, &seg_refs) {
                return Ok(path);
            }
        }
    }

    // Fall back to general (unfiltered) discovery
    if let Some(path) = try_discover(flake_dir, &[]) {
        return Ok(path);
    }

    bail!(
        "clanOptions not exposed in flake outputs.\n\
         For a standard flake, add `clanOptions = clan.options;` to your flake outputs.\n\
         For flake-parts, add:\n  \
           flake.clanOptions = options.flake.valueMeta.configuration.options.clan.valueMeta.configuration.options;\n\
         Or use --file to specify the target file directly."
    )
}

/// Resolve the target file to edit: use `-f` if provided, otherwise attempt
/// option discovery (path-specific then general), falling back to `clan.nix`.
///
/// When `attr_path` is provided, path-specific discovery is attempted first
/// to find the exact file that defines the target attribute (useful for
/// multi-file configurations).
fn resolve_file(
    explicit_file: Option<&Path>,
    flake_override: Option<&Path>,
    attr_path: Option<&str>,
) -> Result<PathBuf> {
    if let Some(f) = explicit_file {
        return Ok(f.to_path_buf());
    }

    // Try discovery
    let cwd = std::env::current_dir().context("cannot get current directory")?;
    let flake_dir = flake_override
        .map(|p| p.to_path_buf())
        .or_else(|| find_flake_root_from_dir(&cwd));

    if let Some(flake_dir) = &flake_dir {
        return discover_file(flake_dir, attr_path);
    }

    bail!(
        "No flake.nix found. Use --file to specify the target file, \
         or run from within a flake directory."
    )
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

/// Try an attribute path, falling back to stripping the `clan.` prefix.
///
/// In flake-parts setups, users specify paths like `clan.inventory.machines.jon`
/// but imported clan modules use paths without the `clan.` prefix. When a file
/// is discovered via `definitionsWithLocations`, it might be a plain clan module
/// where the `clan.` prefix doesn't exist in the AST.
fn effective_path<'a>(path: &'a str, source: &str) -> &'a str {
    if let Some(stripped) = path.strip_prefix("clan.") {
        // Check if the file has a `clan` key at its root — if so, keep the prefix
        if let Ok(root) = ast::parse_nix(source) {
            if ast::get_attr(&root, "clan").is_err() {
                return stripped;
            }
        }
    }
    path
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let flake_override = cli.flake.as_deref();

    // Extract the attribute path for commands that have one, used for
    // path-specific file discovery in multi-file configurations.
    let attr_path: Option<&str> = match &cli.command {
        Commands::Get { path } => Some(path),
        Commands::Set { path, .. } => Some(path),
        Commands::Delete { path } => Some(path),
    };

    let file = resolve_file(cli.file.as_deref(), flake_override, attr_path)?;

    match &cli.command {
        Commands::Get { path } => {
            let source = read_file(&file)?;
            let ep = effective_path(path, &source);
            let root = ast::parse_nix(&source)?;
            let value = ast::get_attr(&root, ep)?;
            println!("{value}");
        }
        Commands::Set { path, value } => {
            let source = read_file(&file)?;
            let ep = effective_path(path, &source);
            let result = ast::set_attr(&source, ep, value)?;
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
            let ep = effective_path(path, &source);
            let result = ast::delete_attr(&source, ep)?;
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

        let result = discover_file(tmpdir.path(), None);
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
        assert!(
            msg.contains("--file"),
            "expected hint about --file workaround, got: {msg}"
        );
        assert!(
            msg.contains("flake-parts"),
            "expected hint about flake-parts, got: {msg}"
        );
    }

    #[test]
    fn test_build_definition_filter_empty() {
        let result = build_definition_filter(&[]);
        assert_eq!(result, "defs: map (d: d.file) defs");
    }

    #[test]
    fn test_build_definition_filter_one_segment() {
        let result = build_definition_filter(&["machines"]);
        assert!(result.contains(r#"d.value ? "machines""#));
        assert!(result.contains("builtins.isAttrs d.value"));
    }

    #[test]
    fn test_build_definition_filter_two_segments() {
        let result = build_definition_filter(&["machines", "jon"]);
        assert!(result.contains(r#"d.value ? "machines""#));
        assert!(result.contains(r#"d.value."machines" ? "jon""#));
    }

    #[test]
    fn test_build_definition_filter_special_chars() {
        let result = build_definition_filter(&["machines", "3rd-node"]);
        assert!(result.contains(r#"d.value."machines" ? "3rd-node""#));
    }

    #[test]
    fn test_inventory_filter_segments() {
        // Standard paths
        assert_eq!(
            inventory_filter_segments("inventory.machines.jon.deploy"),
            Some(vec!["machines".to_string(), "jon".to_string()])
        );
        assert_eq!(
            inventory_filter_segments("machines.jon"),
            Some(vec!["machines".to_string(), "jon".to_string()])
        );

        // Flake-parts paths
        assert_eq!(
            inventory_filter_segments("clan.inventory.machines.jon"),
            Some(vec!["machines".to_string(), "jon".to_string()])
        );

        // meta.name has 2 segments, so it's filterable
        assert_eq!(
            inventory_filter_segments("meta.name"),
            Some(vec!["meta".to_string(), "name".to_string()])
        );
        assert_eq!(
            inventory_filter_segments("clan.meta.name"),
            Some(vec!["meta".to_string(), "name".to_string()])
        );

        // Single segment is too short
        assert_eq!(inventory_filter_segments("meta"), None);
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
