use tree_sitter::Node;

use super::helpers::{child_text_by_field, extract_preceding_comments, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

const COMMENT_KINDS: &[&str] = &["comment", "documentation_comment"];

pub(crate) struct DartExtractor;

impl LanguageExtractor for DartExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            visit_node(child, source, &mut symbols);
        }

        symbols
    }

    fn extract_references(&self, root: Node<'_>, source: &[u8]) -> Vec<RawReference> {
        let mut refs = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            let text = node_text(child, source);
            if text.starts_with("import")
                && let Some(r) = extract_import_ref(child, source)
            {
                refs.push(r);
            }
        }

        refs
    }
}

fn visit_node(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    match node.kind() {
        "function_signature" | "function_definition" => {
            if let Some(sym) = extract_function(node, source) {
                out.push(sym);
            }
        }
        "class_definition" => {
            if let Some(sym) = extract_class(node, source) {
                out.push(sym);
            }
        }
        "enum_declaration" => {
            if let Some(sym) = extract_enum(node, source) {
                out.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_function(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    Some(ParsedSymbol {
        name,
        kind: "function",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: vec![],
    })
}

fn extract_class(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        collect_methods(body, source, &mut children);
    } else {
        // Try direct children if no "body" field
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_body" {
                collect_methods(child, source, &mut children);
            }
        }
    }

    Some(ParsedSymbol {
        name,
        kind: "class",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children,
    })
}

fn extract_enum(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    Some(ParsedSymbol {
        name,
        kind: "enum",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: vec![],
    })
}

fn collect_methods(body: Node<'_>, source: &[u8], children: &mut Vec<ParsedSymbol>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "method_signature"
            && let Some(method) = extract_method_signature(child, source)
        {
            children.push(method);
        }
    }
}

fn extract_method_signature(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    // method_signature contains a function_signature child with the name field
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_signature" {
            let name = child_text_by_field(child, "name", source)?;
            let content = node_text(node, source);
            let signature = content.clone();

            let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);
            return Some(ParsedSymbol {
                name,
                kind: "method",
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature,
                docstring,
                children: vec![],
            });
        }
    }
    None
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

/// Walk the node tree to find the first string literal descendant.
fn find_string_literal<'a>(node: Node<'a>, source: &[u8]) -> Option<String> {
    // Direct match: string_literal or uri (used in some grammar versions)
    if node.kind() == "string_literal"
        || node.kind() == "uri"
        || node.kind() == "string"
        || node.kind() == "uri_expr"
    {
        let raw = node_text(node, source);
        return Some(strip_quotes(&raw));
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(s) = find_string_literal(child, source) {
            return Some(s);
        }
    }

    None
}

fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Extract the last meaningful path segment from a Dart import path, stripping `.dart`.
///
/// Examples:
/// - `"dart:io"`           -> `"io"`
/// - `"package:flutter/material.dart"` -> `"material"`
/// - `"helper.dart"`       -> `"helper"`
fn import_target_name(path: &str) -> String {
    // Strip scheme prefix (dart: / package:)
    let after_scheme = if let Some(pos) = path.find(':') {
        &path[pos + 1..]
    } else {
        path
    };

    // Take last slash-separated segment
    let segment = after_scheme.split('/').next_back().unwrap_or(after_scheme);

    // Strip .dart extension
    segment.strip_suffix(".dart").unwrap_or(segment).to_string()
}

fn extract_import_ref(node: Node<'_>, source: &[u8]) -> Option<RawReference> {
    let line = node.start_position().row as u32 + 1;
    let path = find_string_literal(node, source)?;
    let target_name = import_target_name(&path);

    Some(RawReference {
        source_symbol: String::new(),
        target_name,
        kind: "import",
        line,
    })
}
