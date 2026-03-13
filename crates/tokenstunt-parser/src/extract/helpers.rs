use tree_sitter::Node;

pub(crate) fn child_text_by_field(node: Node<'_>, field: &str, source: &[u8]) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    Some(node_text(child, source))
}

pub(crate) fn node_text(node: Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Collect consecutive comment nodes immediately preceding `node` among its siblings.
/// Returns the cleaned docstring with comment delimiters stripped.
pub(crate) fn extract_preceding_comments(
    node: Node<'_>,
    source: &[u8],
    comment_kinds: &[&str],
) -> String {
    let parent = match node.parent() {
        Some(p) => p,
        None => return String::new(),
    };

    // Find this node's index among its parent's children
    let mut idx = None;
    let mut cursor = parent.walk();
    for (i, child) in parent.children(&mut cursor).enumerate() {
        if child.id() == node.id() {
            idx = Some(i);
            break;
        }
    }

    let idx = match idx {
        Some(i) => i,
        None => return String::new(),
    };

    // Walk backwards from the preceding sibling, collecting consecutive comment nodes
    let mut comments = Vec::new();
    let mut cursor2 = parent.walk();
    let children: Vec<_> = parent.children(&mut cursor2).collect();

    for i in (0..idx).rev() {
        let sibling = children[i];
        if comment_kinds.contains(&sibling.kind()) {
            comments.push(node_text(sibling, source));
        } else {
            break;
        }
    }

    comments.reverse();
    let cleaned: Vec<String> = comments
        .iter()
        .map(|c| strip_comment_delimiters(c))
        .collect();
    cleaned.join("\n")
}

/// Extract Python-style docstrings from the first statement in a function/class body.
pub(crate) fn extract_python_docstring(node: Node<'_>, source: &[u8]) -> String {
    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return String::new(),
    };

    let mut cursor = body.walk();
    let first_child = match body.children(&mut cursor).next() {
        Some(c) => c,
        None => return String::new(),
    };

    if first_child.kind() != "expression_statement" {
        return String::new();
    }

    let mut inner_cursor = first_child.walk();
    let string_node = match first_child.children(&mut inner_cursor).next() {
        Some(n) if n.kind() == "string" => n,
        _ => return String::new(),
    };

    let raw = node_text(string_node, source);
    strip_python_docstring(&raw)
}

fn strip_comment_delimiters(comment: &str) -> String {
    let trimmed = comment.trim();

    // Block comments: /* ... */ or /** ... */
    if trimmed.starts_with("/*") && trimmed.ends_with("*/") {
        let inner = &trimmed[2..trimmed.len() - 2];
        let inner = inner.strip_prefix('*').unwrap_or(inner);
        return inner
            .lines()
            .map(|line| {
                let stripped = line.trim();
                stripped
                    .strip_prefix("* ")
                    .unwrap_or(stripped.strip_prefix('*').unwrap_or(stripped))
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
    }

    // Line comments: /// or // or #
    if let Some(rest) = trimmed.strip_prefix("///") {
        return rest.strip_prefix(' ').unwrap_or(rest).to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        return rest.strip_prefix(' ').unwrap_or(rest).to_string();
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        return rest.strip_prefix(' ').unwrap_or(rest).to_string();
    }

    trimmed.to_string()
}

fn strip_python_docstring(raw: &str) -> String {
    let trimmed = raw.trim();

    // Triple-quoted strings
    for delim in &["\"\"\"", "'''"] {
        if trimmed.starts_with(delim)
            && trimmed.ends_with(delim)
            && trimmed.len() >= delim.len() * 2
        {
            let inner = &trimmed[delim.len()..trimmed.len() - delim.len()];
            return inner.trim().to_string();
        }
    }

    // Single-quoted string
    for delim in &["\"", "'"] {
        if trimmed.starts_with(delim) && trimmed.ends_with(delim) && trimmed.len() >= 2 {
            let inner = &trimmed[delim.len()..trimmed.len() - delim.len()];
            return inner.trim().to_string();
        }
    }

    trimmed.to_string()
}
