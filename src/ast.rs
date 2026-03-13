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

// ---------------------------------------------------------------------------
// Expression classification
// ---------------------------------------------------------------------------

/// Classification of a Nix expression node.
#[derive(Debug)]
pub enum ExprKind {
    /// A plain attribute set `{ ... }`
    AttrSet,
    /// `lib.mkDefault value` or `mkDefault value`
    MkDefault,
    /// `lib.mkForce value` or `mkForce value`
    MkForce,
    /// The `//` merge/update operator
    MergeOperator,
    /// A function application (not mkDefault/mkForce)
    FunctionApplication,
    /// A `let ... in ...` expression
    LetIn,
    /// A lambda `x: body` or `{ ... }: body`
    Lambda,
    /// Anything else (literals, lists, strings, etc.)
    Other,
}

/// Classify a Nix expression node.
pub fn classify_expr(node: &rnix::SyntaxNode) -> ExprKind {
    if rnix::ast::AttrSet::cast(node.clone()).is_some() {
        return ExprKind::AttrSet;
    }

    if let Some(apply) = rnix::ast::Apply::cast(node.clone()) {
        if let Some(wrapper_kind) = detect_mk_wrapper_kind(&apply) {
            return wrapper_kind;
        }
        return ExprKind::FunctionApplication;
    }

    if let Some(binop) = rnix::ast::BinOp::cast(node.clone()) {
        if matches!(binop.operator(), Some(rnix::ast::BinOpKind::Update)) {
            return ExprKind::MergeOperator;
        }
        return ExprKind::Other;
    }

    if rnix::ast::LetIn::cast(node.clone()).is_some() {
        return ExprKind::LetIn;
    }

    if rnix::ast::Lambda::cast(node.clone()).is_some() {
        return ExprKind::Lambda;
    }

    ExprKind::Other
}

/// Check if an Apply node is `lib.mkDefault`, `lib.mkForce`, `mkDefault`, or
/// `mkForce`.  Returns the matching `ExprKind` variant if recognized.
fn detect_mk_wrapper_kind(apply: &rnix::ast::Apply) -> Option<ExprKind> {
    let func = apply.lambda()?;
    let func_name = match &func {
        rnix::ast::Expr::Select(select) => {
            // lib.mkDefault or lib.mkForce
            let base = select.expr()?;
            if let rnix::ast::Expr::Ident(ident) = base {
                if ident.ident_token()?.text() != "lib" {
                    return None;
                }
            } else {
                return None;
            }
            let attrpath = select.attrpath()?;
            let attrs: Vec<String> = attrpath
                .attrs()
                .filter_map(|a| attr_to_string(&a))
                .collect();
            if attrs.len() != 1 {
                return None;
            }
            attrs.into_iter().next()?
        }
        rnix::ast::Expr::Ident(ident) => {
            // mkDefault or mkForce (bare, e.g. via `with lib;`)
            ident.ident_token()?.text().to_string()
        }
        _ => return None,
    };

    match func_name.as_str() {
        "mkDefault" => Some(ExprKind::MkDefault),
        "mkForce" => Some(ExprKind::MkForce),
        _ => None,
    }
}

/// If `node` is a recognized mkDefault/mkForce Apply, return the inner (argument)
/// value node.
pub fn unwrap_mk_wrapper(node: &rnix::SyntaxNode) -> Option<rnix::SyntaxNode> {
    let apply = rnix::ast::Apply::cast(node.clone())?;
    let kind = detect_mk_wrapper_kind(&apply)?;
    match kind {
        ExprKind::MkDefault | ExprKind::MkForce => {
            let arg = apply.argument()?;
            Some(arg.syntax().clone())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Attribute path lookup
// ---------------------------------------------------------------------------

/// Represents the result of looking up an attribute path in a Nix attrset.
pub struct AttrLookup {
    /// The syntax node of the value (right-hand side of the binding).
    /// For mkDefault/mkForce, this is the INNER value (unwrapped).
    pub value_node: rnix::SyntaxNode,
    /// The full binding node (`key = value;`).
    pub binding_node: rnix::SyntaxNode,
    /// If the value was wrapped in mkDefault/mkForce, this is the full Apply
    /// node.  `set_attr` uses this to replace only the inner value.
    pub wrapper_node: Option<rnix::SyntaxNode>,
}

/// Result of an attribute-path lookup operation.
pub enum LookupResult {
    /// Found the attribute at the exact path.
    Found(AttrLookup),
    /// Attribute path not found (no matching bindings).
    NotFound,
    /// A prefix matched but the value is a complex expression that cannot be
    /// navigated into.
    Blocked { path: String, reason: String },
}

/// Navigate to an attribute path in the root expression.
///
/// Supports both nested forms (`a = { b = x; }`) and dotted key forms
/// (`a.b = x`).  The root expression must be an attribute set (possibly with
/// `let ... in { ... }` or a lambda wrapper).
pub fn lookup_attr_path(root: &rnix::Root, path: &[&str]) -> LookupResult {
    let Some(expr) = root.expr() else {
        return LookupResult::NotFound;
    };
    let Some(attrset) = find_root_attrset(&expr) else {
        return LookupResult::NotFound;
    };
    lookup_in_attrset(attrset.syntax(), path, path)
}

/// Find the root attribute set from the top-level expression,
/// handling `let ... in { ... }`, `{ ... }`, and `{ ... }: { ... }` (lambda)
/// forms.
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
///
/// `full_path` is the complete original path (used for error messages).
/// `path` is the remaining path to resolve at this level.
fn lookup_in_attrset(
    attrset_node: &rnix::SyntaxNode,
    path: &[&str],
    full_path: &[&str],
) -> LookupResult {
    if path.is_empty() {
        return LookupResult::NotFound;
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

        // Check if this binding's key path matches the beginning of our
        // target path.
        if keys.len() <= path.len() && keys.iter().zip(path.iter()).all(|(a, b)| a == b) {
            let remaining = &path[keys.len()..];

            if remaining.is_empty() {
                // --- Exact match -----------------------------------------
                let Some(value_expr) = binding.value() else {
                    continue;
                };
                let value_node = value_expr.syntax().clone();

                // Unwrap mkDefault/mkForce
                if let Some(inner) = unwrap_mk_wrapper(&value_node) {
                    return LookupResult::Found(AttrLookup {
                        value_node: inner,
                        binding_node: child,
                        wrapper_node: Some(value_node),
                    });
                }

                return LookupResult::Found(AttrLookup {
                    value_node,
                    binding_node: child,
                    wrapper_node: None,
                });
            }

            // --- Prefix match – navigate deeper -------------------------
            let Some(value_expr) = binding.value() else {
                continue;
            };
            let value_node = value_expr.syntax().clone();

            match classify_expr(&value_node) {
                ExprKind::AttrSet => {
                    if let Some(inner) = rnix::ast::AttrSet::cast(value_node) {
                        return lookup_in_attrset(inner.syntax(), remaining, full_path);
                    }
                }
                ExprKind::MkDefault | ExprKind::MkForce => {
                    if let Some(inner) = unwrap_mk_wrapper(&value_node) {
                        if let Some(inner_attrset) = rnix::ast::AttrSet::cast(inner.clone()) {
                            return lookup_in_attrset(inner_attrset.syntax(), remaining, full_path);
                        }
                        // Inner value is not an attrset
                        let consumed = full_path.len() - remaining.len();
                        return LookupResult::Blocked {
                            path: full_path[..consumed].join("."),
                            reason: "value inside mkDefault/mkForce is not an attribute set"
                                .to_string(),
                        };
                    }
                }
                ExprKind::MergeOperator => {
                    let consumed = full_path.len() - remaining.len();
                    return LookupResult::Blocked {
                        path: full_path[..consumed].join("."),
                        reason:
                            "value uses merge operator (//). clan-edit cannot safely modify values constructed with //."
                                .to_string(),
                    };
                }
                ExprKind::FunctionApplication => {
                    let consumed = full_path.len() - remaining.len();
                    return LookupResult::Blocked {
                        path: full_path[..consumed].join("."),
                        reason:
                            "value is a function application. clan-edit can only navigate into plain attribute sets."
                                .to_string(),
                    };
                }
                ExprKind::LetIn => {
                    let consumed = full_path.len() - remaining.len();
                    return LookupResult::Blocked {
                        path: full_path[..consumed].join("."),
                        reason:
                            "value is a let-in expression. clan-edit cannot navigate into let-in bodies."
                                .to_string(),
                    };
                }
                ExprKind::Lambda => {
                    let consumed = full_path.len() - remaining.len();
                    return LookupResult::Blocked {
                        path: full_path[..consumed].join("."),
                        reason:
                            "value is a function/lambda. clan-edit can only navigate into plain attribute sets."
                                .to_string(),
                    };
                }
                ExprKind::Other => {
                    // Can't navigate into literals, lists, etc. – not an
                    // error, just not found.
                }
            }
        }
    }

    LookupResult::NotFound
}

// ---------------------------------------------------------------------------
// Intermediate path collection (for paths that span multiple dotted-key
// bindings but have no single binding of their own).
// ---------------------------------------------------------------------------

/// Collect all bindings in `attrset_node` whose key path starts with `prefix`.
/// Returns (remaining_keys, value_source_text) pairs.
fn collect_prefix_bindings(
    attrset_node: &rnix::SyntaxNode,
    prefix: &[&str],
) -> Vec<(Vec<String>, String)> {
    let mut results = Vec::new();

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

        if keys.len() > prefix.len()
            && keys[..prefix.len()]
                .iter()
                .zip(prefix.iter())
                .all(|(a, b)| a == b)
        {
            let remaining: Vec<String> = keys[prefix.len()..].to_vec();
            if let Some(value) = binding.value() {
                results.push((remaining, value.syntax().to_string()));
            }
        }
    }

    results
}

/// Navigate to the attrset containing `path` and collect prefix bindings
/// from there.  This is the entry-point used by `get_attr` when an exact
/// lookup returns `NotFound`.
fn collect_prefix_bindings_recursive(
    root: &rnix::Root,
    full_path: &[&str],
) -> Vec<(Vec<String>, String)> {
    let Some(expr) = root.expr() else {
        return Vec::new();
    };
    let Some(attrset) = find_root_attrset(&expr) else {
        return Vec::new();
    };
    collect_prefix_in_attrset(attrset.syntax(), full_path)
}

/// Navigate as deep as possible along `path`, then collect prefix bindings for
/// the remaining path segments.
fn collect_prefix_in_attrset(
    attrset_node: &rnix::SyntaxNode,
    path: &[&str],
) -> Vec<(Vec<String>, String)> {
    if path.is_empty() {
        return Vec::new();
    }

    // Try to navigate deeper first
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
                // Exact match for this binding – caller should have found it
                // via normal lookup.  Nothing to collect here.
                return Vec::new();
            }

            // Try descending into the value
            if let Some(value) = binding.value() {
                if let rnix::ast::Expr::AttrSet(inner) = value {
                    return collect_prefix_in_attrset(inner.syntax(), remaining);
                }
                // Try unwrapping mkDefault/mkForce
                if let Some(inner) = unwrap_mk_wrapper(value.syntax()) {
                    if let Some(inner_attrset) = rnix::ast::AttrSet::cast(inner) {
                        return collect_prefix_in_attrset(inner_attrset.syntax(), remaining);
                    }
                }
            }
            return Vec::new();
        }
    }

    // Could not navigate further — collect prefix bindings at this level
    collect_prefix_bindings(attrset_node, path)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Get the Nix source text of the value at the given attribute path.
pub fn get_attr(root: &rnix::Root, path_str: &str) -> Result<String> {
    let parts: Vec<&str> = path_str.split('.').collect();
    match lookup_attr_path(root, &parts) {
        LookupResult::Found(lookup) => Ok(lookup.value_node.to_string()),
        LookupResult::Blocked { path, reason } => {
            bail!("cannot navigate into expression at '{path}': {reason}")
        }
        LookupResult::NotFound => {
            // Try intermediate path reconstruction
            let bindings = collect_prefix_bindings_recursive(root, &parts);
            if bindings.is_empty() {
                bail!("attribute path not found: {path_str}")
            }
            let mut result = String::from("{\n");
            for (remaining_keys, value_text) in &bindings {
                let key = remaining_keys
                    .iter()
                    .map(|k| format_attr_key(k))
                    .collect::<Vec<_>>()
                    .join(".");
                result.push_str(&format!("  {key} = {value_text};\n"));
            }
            result.push('}');
            Ok(result)
        }
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
        LookupResult::Found(lookup) => {
            // Replace the value.  For mk-wrapped values this replaces just
            // the inner value, preserving the wrapper (since value_node
            // points to the unwrapped argument).
            let old_range = lookup.value_node.text_range();
            let start: usize = old_range.start().into();
            let end: usize = old_range.end().into();
            let mut result = String::with_capacity(source.len());
            result.push_str(&source[..start]);
            result.push_str(value_str);
            result.push_str(&source[end..]);
            Ok(result)
        }
        LookupResult::Blocked { path, reason } => {
            bail!("cannot navigate into expression at '{path}': {reason}")
        }
        LookupResult::NotFound => {
            // Find the deepest existing ancestor and insert there
            insert_attr(source, &root, &parts, value_str)
        }
    }
}

/// Delete the attribute at the given path.
/// Returns the modified source as a string.
pub fn delete_attr(source: &str, path_str: &str) -> Result<String> {
    let root = parse_nix(source)?;
    let parts: Vec<&str> = path_str.split('.').collect();

    match lookup_attr_path(&root, &parts) {
        LookupResult::Found(lookup) => {
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
        LookupResult::Blocked { path, reason } => {
            bail!("cannot navigate into expression at '{path}': {reason}")
        }
        LookupResult::NotFound => bail!("attribute path not found: {path_str}"),
    }
}

// ---------------------------------------------------------------------------
// Insertion helpers
// ---------------------------------------------------------------------------

/// Insert a new attribute binding by finding the deepest existing ancestor
/// attrset.
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
            // Try unwrapping mkDefault/mkForce
            if let Some(value) = binding.value() {
                if let Some(inner) = unwrap_mk_wrapper(value.syntax()) {
                    if let Some(inner_attrset) = rnix::ast::AttrSet::cast(inner) {
                        return find_deepest_ancestor(inner_attrset.syntax(), remaining);
                    }
                }
            }
            // Value exists but isn't an attrset - can't descend further
            return (attrset_node.clone(), path);
        }
    }

    (attrset_node.clone(), path)
}

/// Find the byte position just before the closing `}` of an attrset node.
fn find_insert_position(node: &rnix::SyntaxNode) -> usize {
    let mut last_brace_pos = None;
    for child in node.children_with_tokens() {
        if let rowan::NodeOrToken::Token(token) = child {
            if token.kind() == SyntaxKind::TOKEN_R_BRACE {
                last_brace_pos = Some(token.text_range().start().into());
            }
        }
    }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        let result = set_attr(
            source,
            "inventory.machines.my-server.deploy.targetHost",
            "\"10.0.0.1\"",
        )
        .unwrap();
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

        // Cannot navigate INTO the let...in body — now gives a descriptive error
        let err = get_attr(&root, "inventory.instances.sshd.module.name").unwrap_err();
        assert!(
            err.to_string().contains("let-in"),
            "expected let-in error, got: {err}"
        );
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
        let result = set_attr(
            source,
            "inventory.instances.sshd",
            "{ module.name = \"sshd\"; roles.server.tags.all = { }; }",
        )
        .unwrap();
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

    // -----------------------------------------------------------------------
    // Expression classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_attrset() {
        let root = parse_nix(r#"{ a = { x = 1; }; }"#).unwrap();
        let _val = get_attr(&root, "a").unwrap();
        // Verify it's navigable
        let val2 = get_attr(&root, "a.x").unwrap();
        assert_eq!(val2.trim(), "1");
    }

    #[test]
    fn test_classify_merge_operator() {
        let source = r#"{ a = { x = 1; } // { y = 2; }; }"#;
        let root = parse_nix(source).unwrap();

        // Getting the whole value works
        let val = get_attr(&root, "a").unwrap();
        assert!(val.contains("//"));

        // Navigating into it fails with a descriptive error
        let err = get_attr(&root, "a.x").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("merge operator"),
            "expected merge error, got: {msg}"
        );
        assert!(msg.contains("//"), "expected // in error, got: {msg}");
    }

    #[test]
    fn test_classify_function_application() {
        let source = r#"{ a = someFunc { x = 1; }; }"#;
        let root = parse_nix(source).unwrap();

        // Getting the whole value works
        let val = get_attr(&root, "a").unwrap();
        assert!(val.contains("someFunc"));

        // Navigating into it fails
        let err = get_attr(&root, "a.x").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("function application"),
            "expected function application error, got: {msg}"
        );
    }

    #[test]
    fn test_classify_lambda() {
        let source = r#"{ a = x: { name = x; }; }"#;
        let root = parse_nix(source).unwrap();

        // Navigating into lambda fails
        let err = get_attr(&root, "a.name").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("function/lambda"),
            "expected lambda error, got: {msg}"
        );
    }

    #[test]
    fn test_classify_let_in() {
        let source = r#"{ a = let x = 1; in { y = x; }; }"#;
        let root = parse_nix(source).unwrap();

        let err = get_attr(&root, "a.y").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("let-in"), "expected let-in error, got: {msg}");
    }

    // -----------------------------------------------------------------------
    // mkDefault / mkForce tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_mkdefault_unwraps() {
        let source = r#"{ meta.name = lib.mkDefault "MyClan"; }"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "meta.name").unwrap();
        assert_eq!(val, "\"MyClan\"");
    }

    #[test]
    fn test_get_mkforce_unwraps() {
        let source = r#"{ meta.name = lib.mkForce "ForcedClan"; }"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "meta.name").unwrap();
        assert_eq!(val, "\"ForcedClan\"");
    }

    #[test]
    fn test_get_bare_mkdefault_unwraps() {
        let source = r#"{ meta.name = mkDefault "MyClan"; }"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "meta.name").unwrap();
        assert_eq!(val, "\"MyClan\"");
    }

    #[test]
    fn test_set_preserves_mkdefault_wrapper() {
        let source = r#"{ meta.name = lib.mkDefault "OldName"; }"#;
        let result = set_attr(source, "meta.name", "\"NewName\"").unwrap();
        assert!(
            result.contains("lib.mkDefault \"NewName\""),
            "expected lib.mkDefault preserved, got: {result}"
        );
        assert!(!result.contains("OldName"));
    }

    #[test]
    fn test_set_preserves_mkforce_wrapper() {
        let source = r#"{ meta.name = lib.mkForce "OldName"; }"#;
        let result = set_attr(source, "meta.name", "\"NewName\"").unwrap();
        assert!(
            result.contains("lib.mkForce \"NewName\""),
            "expected lib.mkForce preserved, got: {result}"
        );
    }

    #[test]
    fn test_navigate_through_mkdefault_attrset() {
        let source = r#"{ inventory.instances.sshd = lib.mkDefault { module.name = "sshd"; }; }"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "inventory.instances.sshd.module.name").unwrap();
        assert_eq!(val, "\"sshd\"");
    }

    #[test]
    fn test_set_through_mkdefault_attrset() {
        let source = r#"{ inventory.instances.sshd = lib.mkDefault { module.name = "sshd"; }; }"#;
        let result = set_attr(
            source,
            "inventory.instances.sshd.module.name",
            "\"newsshd\"",
        )
        .unwrap();
        assert!(result.contains("lib.mkDefault"));
        assert!(result.contains("\"newsshd\""));
        assert!(!result.contains("\"sshd\""));
    }

    #[test]
    fn test_other_function_not_unwrapped() {
        // someTransform is not mkDefault/mkForce, so it's treated as a
        // function application
        let source = r#"{ meta.name = someTransform "value"; }"#;
        let root = parse_nix(source).unwrap();

        // Getting the leaf value returns the full expression
        let val = get_attr(&root, "meta.name").unwrap();
        assert!(
            val.contains("someTransform"),
            "expected full expr, got: {val}"
        );
    }

    // -----------------------------------------------------------------------
    // Intermediate path navigation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_intermediate_path_basic() {
        let source = r#"{
  instances.yggdrasil = {
    module.name = "yggdrasil";
    roles.default.tags = [ "all" ];
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "instances.yggdrasil.roles").unwrap();
        assert!(
            val.contains("default.tags"),
            "expected roles subtree, got: {val}"
        );
    }

    #[test]
    fn test_intermediate_path_multiple_children() {
        let source = r#"{
  instances.sshd = {
    module.name = "sshd";
    roles.server.tags.all = { };
    roles.client.tags.all = { };
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "instances.sshd.roles").unwrap();
        assert!(
            val.contains("server.tags.all"),
            "expected server, got: {val}"
        );
        assert!(
            val.contains("client.tags.all"),
            "expected client, got: {val}"
        );
    }

    #[test]
    fn test_intermediate_path_deep() {
        let source = r#"{
  instances.sshd = {
    roles.server.tags.all = { };
    roles.server.settings.key = "val";
  };
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "instances.sshd.roles.server").unwrap();
        assert!(val.contains("tags.all"), "expected tags.all, got: {val}");
        assert!(
            val.contains("settings.key"),
            "expected settings.key, got: {val}"
        );
    }

    #[test]
    fn test_intermediate_path_not_found() {
        let source = r#"{ instances.sshd.module.name = "sshd"; }"#;
        let root = parse_nix(source).unwrap();
        assert!(get_attr(&root, "instances.sshd.nonexistent").is_err());
    }

    #[test]
    fn test_intermediate_path_with_dotted_at_root() {
        // Both bindings are at the root with dotted keys sharing a prefix
        let source = r#"{
  a.b = "hello";
  a.c = 42;
}"#;
        let root = parse_nix(source).unwrap();
        let val = get_attr(&root, "a").unwrap();
        assert!(val.contains("b = \"hello\""), "expected b, got: {val}");
        assert!(val.contains("c = 42"), "expected c, got: {val}");
    }

    // -----------------------------------------------------------------------
    // Complex expression error tests (bug report cases)
    // -----------------------------------------------------------------------

    #[test]
    fn test_merge_operator_blocks_navigation() {
        let source = r#"{
  instances.machine-type = {
    module.input = "self";
    module.name = "@pinpox/machine-type";
  } // {
    foo = "bar";
  };
}"#;
        let root = parse_nix(source).unwrap();

        // Getting the whole value works
        let val = get_attr(&root, "instances.machine-type").unwrap();
        assert!(val.contains("//"));

        // Navigating into any sub-path fails with merge operator error
        for subpath in &[
            "instances.machine-type.foo",
            "instances.machine-type.module",
            "instances.machine-type.module.input",
        ] {
            let err = get_attr(&root, subpath).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("merge operator"),
                "path '{subpath}' expected merge error, got: {msg}"
            );
        }
    }

    #[test]
    fn test_set_through_merge_fails() {
        let source = r#"{ a = { x = 1; } // { y = 2; }; }"#;
        let err = set_attr(source, "a.x", "42").unwrap_err();
        assert!(err.to_string().contains("merge operator"));
    }

    #[test]
    fn test_set_through_function_fails() {
        let source = r#"{ a = someFunc { x = 1; }; }"#;
        let err = set_attr(source, "a.x", "42").unwrap_err();
        assert!(err.to_string().contains("function application"));
    }
}
