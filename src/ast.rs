use anyhow::{bail, Context, Result};
use rnix::SyntaxKind;
use rowan::ast::AstNode;

/// Parse a Nix source string into an rnix Root AST node.
/// Returns an error if the file contains parse errors.
pub fn parse_nix(source: &str) -> Result<rnix::Root> {
    let parse = rnix::Root::parse(source);
    let errors = parse.errors();
    if !errors.is_empty() {
        let mut msg = String::from("Parse errors:\n");
        for err in errors {
            msg.push_str(&format!("  {err}\n"));
        }
        bail!(msg);
    }
    Ok(parse.tree())
}

/// Print a parsed Root back to a string (roundtrip).
pub fn print_nix(root: &rnix::Root) -> String {
    root.syntax().to_string()
}

/// Represents the result of looking up an attribute path in a Nix attrset.
pub struct AttrLookup {
    /// The syntax node of the value (right-hand side of the binding).
    pub value_node: rnix::SyntaxNode,
    /// The full binding node (`key = value;`).
    pub binding_node: rnix::SyntaxNode,
}

/// Navigate to an attribute path in the root expression.
///
/// Supports both nested forms (`a = { b = x; }`) and dotted key forms (`a.b = x`).
/// The root expression must be an attribute set (possibly with `let ... in { ... }`).
///
/// Returns the value node and binding node at the given path, or None if not found.
pub fn lookup_attr_path(root: &rnix::Root, path: &[&str]) -> Option<AttrLookup> {
    let expr = root.expr()?;
    let attrset = find_root_attrset(&expr)?;
    lookup_in_attrset(attrset.syntax(), path)
}

/// Find the root attribute set from the top-level expression,
/// handling `let ... in { ... }`, `{ ... }`, and `{ ... }: { ... }` (lambda) forms.
fn find_root_attrset(expr: &rnix::ast::Expr) -> Option<rnix::ast::AttrSet> {
    match expr {
        rnix::ast::Expr::AttrSet(attrset) => Some(attrset.clone()),
        rnix::ast::Expr::LetIn(let_in) => {
            let body = let_in.body()?;
            find_root_attrset(&body)
        }
        rnix::ast::Expr::Lambda(lambda) => {
            let body = lambda.body()?;
            find_root_attrset(&body)
        }
        _ => None,
    }
}

/// Recursively look up an attribute path within an attrset syntax node.
fn lookup_in_attrset(attrset_node: &rnix::SyntaxNode, path: &[&str]) -> Option<AttrLookup> {
    if path.is_empty() {
        return None;
    }

    // Iterate over all bindings (HasEntry items) in this attrset
    for child in attrset_node.children() {
        if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let binding: rnix::ast::AttrpathValue = AstNode::cast(child.clone())?;
        let attrpath = binding.attrpath()?;
        let keys: Vec<String> = attrpath
            .attrs()
            .filter_map(|attr| attr_to_string(&attr))
            .collect();

        if keys.is_empty() {
            continue;
        }

        // Check if this binding's key path matches the beginning of our target path
        let target = path;

        if keys.len() <= target.len() && keys.iter().zip(target.iter()).all(|(a, b)| a == b) {
            let remaining = &target[keys.len()..];

            if remaining.is_empty() {
                // Exact match - return this binding's value
                let value = binding.value()?;
                return Some(AttrLookup {
                    value_node: value.syntax().clone(),
                    binding_node: child,
                });
            }

            // We matched a prefix; look deeper into the value if it's an attrset
            let value = binding.value()?;
            if let rnix::ast::Expr::AttrSet(inner) = value {
                return lookup_in_attrset(inner.syntax(), remaining);
            }
        }
    }

    None
}

/// Convert an Attr AST node to a string key name.
fn attr_to_string(attr: &rnix::ast::Attr) -> Option<String> {
    match attr {
        rnix::ast::Attr::Ident(ident) => Some(ident.ident_token()?.text().to_string()),
        rnix::ast::Attr::Str(s) => {
            // Handle quoted attribute names like "gchq-local"
            // Extract the string content from the string literal
            let text = s.syntax().to_string();
            // Remove quotes
            let inner = text.trim_matches('"');
            Some(inner.to_string())
        }
        _ => None,
    }
}

/// Get the Nix source text of the value at the given attribute path.
pub fn get_attr(root: &rnix::Root, path_str: &str) -> Result<String> {
    let parts: Vec<&str> = path_str.split('.').collect();
    match lookup_attr_path(root, &parts) {
        Some(lookup) => Ok(lookup.value_node.to_string()),
        None => bail!("attribute path not found: {path_str}"),
    }
}

/// Set (insert or update) a value at the given attribute path.
///
/// If the path exists, replaces the value. If it doesn't exist, inserts a new
/// binding in the nearest existing ancestor attrset.
///
/// Returns the modified source as a string.
pub fn set_attr(source: &str, path_str: &str, value_str: &str) -> Result<String> {
    let root = parse_nix(source)?;
    let parts: Vec<&str> = path_str.split('.').collect();

    match lookup_attr_path(&root, &parts) {
        Some(lookup) => {
            // Replace existing value
            let old_range = lookup.value_node.text_range();
            let start: usize = old_range.start().into();
            let end: usize = old_range.end().into();
            let mut result = String::with_capacity(source.len());
            result.push_str(&source[..start]);
            result.push_str(value_str);
            result.push_str(&source[end..]);
            Ok(result)
        }
        None => {
            // Find the deepest existing ancestor and insert there
            insert_attr(source, &root, &parts, value_str)
        }
    }
}

/// Insert a new attribute binding by finding the deepest existing ancestor attrset.
fn insert_attr(source: &str, root: &rnix::Root, path: &[&str], value_str: &str) -> Result<String> {
    let expr = root.expr().context("file has no top-level expression")?;
    let root_attrset =
        find_root_attrset(&expr).context("top-level expression is not an attrset")?;

    // Find the deepest existing ancestor
    let (ancestor_node, remaining_path) = find_deepest_ancestor(root_attrset.syntax(), path);

    if remaining_path.is_empty() {
        bail!("attribute path already exists");
    }

    // Build the dotted key for the new binding
    let dotted_key = remaining_path
        .iter()
        .map(|p| format_attr_key(p))
        .collect::<Vec<_>>()
        .join(".");
    let new_binding = format!("{dotted_key} = {value_str};");

    // Find the position just before the closing brace of the ancestor attrset
    let insert_pos = find_insert_position(&ancestor_node);

    // Determine indentation from existing bindings or default
    let indent = detect_indent(&ancestor_node);

    let mut result = String::with_capacity(source.len() + new_binding.len() + indent.len() + 2);
    result.push_str(&source[..insert_pos]);
    result.push_str(&indent);
    result.push_str(&new_binding);
    result.push('\n');
    result.push_str(&source[insert_pos..]);
    Ok(result)
}

/// Format an attribute key, quoting it if it contains special characters.
fn format_attr_key(key: &str) -> String {
    if key
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        // Simple identifiers can stay unquoted; hyphens are valid in Nix idents
        key.to_string()
    } else {
        format!("\"{key}\"")
    }
}

/// Find the deepest ancestor attrset that exists along the given path.
/// Returns the ancestor node and the remaining path segments.
fn find_deepest_ancestor<'a>(
    attrset_node: &rnix::SyntaxNode,
    path: &'a [&'a str],
) -> (rnix::SyntaxNode, &'a [&'a str]) {
    if path.is_empty() {
        return (attrset_node.clone(), path);
    }

    for child in attrset_node.children() {
        if child.kind() != SyntaxKind::NODE_ATTRPATH_VALUE {
            continue;
        }
        let Some(binding) = <rnix::ast::AttrpathValue as AstNode>::cast(child.clone()) else {
            continue;
        };
        let Some(attrpath) = binding.attrpath() else {
            continue;
        };
        let keys: Vec<String> = attrpath
            .attrs()
            .filter_map(|attr| attr_to_string(&attr))
            .collect();

        if keys.is_empty() {
            continue;
        }

        if keys.len() <= path.len() && keys.iter().zip(path.iter()).all(|(a, b)| a == b) {
            let remaining = &path[keys.len()..];
            if remaining.is_empty() {
                return (attrset_node.clone(), path);
            }
            // Check if the value is an attrset we can descend into
            if let Some(rnix::ast::Expr::AttrSet(inner)) = binding.value() {
                return find_deepest_ancestor(inner.syntax(), remaining);
            }
            // Value exists but isn't an attrset - can't descend further
            return (attrset_node.clone(), path);
        }
    }

    (attrset_node.clone(), path)
}

/// Find the byte position just before the closing `}` of an attrset node.
fn find_insert_position(node: &rnix::SyntaxNode) -> usize {
    // Look for the closing brace token - collect to vec since iterator doesn't support rev
    let mut last_brace_pos = None;
    for child in node.children_with_tokens() {
        if let rowan::NodeOrToken::Token(token) = child {
            if token.kind() == SyntaxKind::TOKEN_R_BRACE {
                last_brace_pos = Some(token.text_range().start().into());
            }
        }
    }
    // Return the position of the last closing brace, or the end of the node
    last_brace_pos.unwrap_or_else(|| node.text_range().end().into())
}

/// Detect the indentation used in an attrset by looking at existing bindings.
fn detect_indent(node: &rnix::SyntaxNode) -> String {
    for child in node.children() {
        if child.kind() == SyntaxKind::NODE_ATTRPATH_VALUE {
            // Look at whitespace before this binding
            if let Some(rowan::NodeOrToken::Token(token)) = child.prev_sibling_or_token() {
                let text = token.text();
                if let Some(last_newline) = text.rfind('\n') {
                    return text[last_newline..].to_string();
                }
            }
        }
    }
    "\n  ".to_string()
}

/// Delete the attribute at the given path.
/// Returns the modified source as a string.
pub fn delete_attr(source: &str, path_str: &str) -> Result<String> {
    let root = parse_nix(source)?;
    let parts: Vec<&str> = path_str.split('.').collect();

    match lookup_attr_path(&root, &parts) {
        Some(lookup) => {
            let binding = &lookup.binding_node;
            let start: usize = binding.text_range().start().into();
            let end: usize = binding.text_range().end().into();

            // Include the semicolon after the binding if present
            let after_binding = &source[end..];
            let extra = if after_binding.starts_with(';') { 1 } else { 0 };

            // Also consume leading whitespace (back to the previous newline)
            let before_binding = &source[..start];
            let leading_ws = before_binding
                .rfind('\n')
                .map(|pos| start - pos - 1)
                .unwrap_or(0);

            let trim_start = start - leading_ws;
            let trim_end = end + extra;

            // Also eat a trailing newline if present
            let after_trim = &source[trim_end..];
            let trailing_nl = if after_trim.starts_with('\n') { 1 } else { 0 };

            let mut result = String::with_capacity(source.len());
            result.push_str(&source[..trim_start]);
            result.push_str(&source[trim_end + trailing_nl..]);
            Ok(result)
        }
        None => bail!("attribute path not found: {path_str}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_nix() {
        let source = r#"{ meta.name = "test"; }"#;
        let root = parse_nix(source).unwrap();
        assert!(root.expr().is_some());
    }

    #[test]
    fn test_parse_invalid_nix() {
        let source = r#"{ meta.name = ; }"#;
        assert!(parse_nix(source).is_err());
    }

    #[test]
    fn test_roundtrip() {
        let source = r#"{
  meta.name = "MyClan";
  meta.domain = "example.com";

  # A comment
  inventory.machines = {
    server = { };
  };
}"#;
        let root = parse_nix(source).unwrap();
        let output = print_nix(&root);
        assert_eq!(source, output);
    }

    #[test]
    fn test_get_dotted_attr() {
        let source = r#"{ meta.name = "MyClan"; }"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "meta.name").unwrap();
        assert_eq!(val, "\"MyClan\"");
    }

    #[test]
    fn test_get_nested_attr() {
        let source = r#"{
  inventory = {
    machines = {
      server = { };
    };
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "inventory.machines.server").unwrap();
        assert_eq!(val.trim(), "{ }");
    }

    #[test]
    fn test_get_mixed_path() {
        let source = r#"{
  inventory.instances = {
    sshd.roles.server = { };
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "inventory.instances.sshd.roles.server").unwrap();
        assert_eq!(val.trim(), "{ }");
    }

    #[test]
    fn test_get_not_found() {
        let source = r#"{ meta.name = "test"; }"#;
        let root = parse_nix(source).unwrap();
        assert!(get_attr(&root, "meta.domain").is_err());
    }

    #[test]
    fn test_set_existing_value() {
        let source = r#"{ meta.name = "old"; }"#;
        let result = set_attr(source, "meta.name", "\"new\"").unwrap();
        assert_eq!(result, r#"{ meta.name = "new"; }"#);
    }

    #[test]
    fn test_set_new_value_in_existing_attrset() {
        let source = "{\n  meta.name = \"test\";\n}\n";
        let result = set_attr(source, "meta.domain", "\"example.com\"").unwrap();
        assert!(result.contains("meta.domain = \"example.com\""));
        // Original value should still be there
        assert!(result.contains("meta.name = \"test\""));
    }

    #[test]
    fn test_delete_attr() {
        let source = "{\n  meta.name = \"test\";\n  meta.domain = \"example.com\";\n}\n";
        let result = delete_attr(source, "meta.domain").unwrap();
        assert!(!result.contains("meta.domain"));
        assert!(result.contains("meta.name"));
    }

    #[test]
    fn test_delete_not_found() {
        let source = r#"{ meta.name = "test"; }"#;
        assert!(delete_attr(source, "meta.domain").is_err());
    }

    #[test]
    fn test_set_preserves_comments() {
        let source = "{\n  # Important comment\n  meta.name = \"old\";\n}\n";
        let result = set_attr(source, "meta.name", "\"new\"").unwrap();
        assert!(result.contains("# Important comment"));
        assert!(result.contains("\"new\""));
    }

    #[test]
    fn test_get_quoted_key() {
        let source = r#"{
  inventory.instances.sshd.roles.server.machines."gchq-local".settings = {
    certificate.searchDomains = [ "*.gchq.icu" ];
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(
            &root,
            "inventory.instances.sshd.roles.server.machines.gchq-local.settings",
        )
        .unwrap();
        assert!(val.contains("certificate.searchDomains"));
    }

    #[test]
    fn test_get_quoted_key_with_space() {
        let source = r#"{
  inventory.machines."webserver 2" = {
    deploy.targetHost = "10.0.0.2";
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "inventory.machines.webserver 2").unwrap();
        assert!(val.contains("deploy.targetHost"));
    }

    #[test]
    fn test_set_key_needs_quoting() {
        // Key starting with a digit must be quoted in Nix
        let source = "{\n  inventory.machines = {\n    alpha = { };\n  };\n}\n";
        let result = set_attr(source, "inventory.machines.2nd-server", "{ }").unwrap();
        assert!(result.contains(r#""2nd-server" = { }"#));
    }

    #[test]
    fn test_set_key_with_space() {
        let source = "{\n  inventory.machines = {\n    alpha = { };\n  };\n}\n";
        let result = set_attr(source, "inventory.machines.my server", "{ }").unwrap();
        assert!(result.contains(r#""my server" = { }"#));
    }

    #[test]
    fn test_get_set_delete_dashed_key() {
        let source = "{\n  inventory.machines.my-server = { };\n}\n";
        let root = parse_nix(source).unwrap();
        // Dashes are valid in Nix identifiers, should work without quoting
        let val = get_attr(&root, "inventory.machines.my-server").unwrap();
        assert_eq!(val.trim(), "{ }");

        // Set a sub-path on the dashed key
        let result = set_attr(source, "inventory.machines.my-server.deploy.targetHost", "\"10.0.0.1\"").unwrap();
        assert!(result.contains("my-server"));
        assert!(result.contains("10.0.0.1"));

        // Delete the dashed key
        let result = delete_attr(source, "inventory.machines.my-server").unwrap();
        assert!(!result.contains("my-server"));
    }

    #[test]
    fn test_value_is_let_in_opaque() {
        // When a value is a let...in expression, we can read it as a whole
        // but cannot navigate into its body
        let source = r#"{
  inventory.instances.sshd = let
    commonKey = "ssh-ed25519 AAAA";
  in {
    module.name = "sshd";
    roles.server.settings.authorizedKeys.admin = commonKey;
  };
}"#;
        let root = parse_nix(source).unwrap();

        // Can get the whole let...in expression
        let val = get_attr(&root, "inventory.instances.sshd").unwrap();
        assert!(val.contains("commonKey"));
        assert!(val.contains("module.name"));

        // Cannot navigate INTO the let...in body
        assert!(get_attr(&root, "inventory.instances.sshd.module.name").is_err());
    }

    #[test]
    fn test_set_replaces_let_in_value() {
        // Setting a path whose value is a let...in replaces the entire expression
        let source = r#"{
  inventory.instances.sshd = let
    x = 1;
  in {
    module.name = "sshd";
  };
}"#;
        let result = set_attr(source, "inventory.instances.sshd", "{ module.name = \"sshd\"; roles.server.tags.all = { }; }").unwrap();
        assert!(!result.contains("let"));
        assert!(result.contains("roles.server.tags.all"));
    }

    #[test]
    fn test_roundtrip_clan_nix() {
        let source = r#"{
  meta.name = "Qubasas_Clan";
  meta.domain = "dark";

  inventory.instances = {
    sshd = {
      module = {
        name = "sshd";
        input = "clan-core";
      };
      roles.server.tags.all = { };
    };
  };
}"#;
        let root = parse_nix(source).unwrap();
        let output = print_nix(&root);
        assert_eq!(source, output);
    }
}
