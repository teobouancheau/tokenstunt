use tree_sitter::Node;

use super::helpers::{child_text_by_field, extract_preceding_comments, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

const COMMENT_KINDS: &[&str] = &["comment", "multiline_comment"];

pub(crate) struct KotlinExtractor;

impl LanguageExtractor for KotlinExtractor {
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
            match child.kind() {
                "import_header" => extract_import_ref(child, source, &mut refs),
                "import_list" => {
                    let mut inner = child.walk();
                    for import in child.children(&mut inner) {
                        if import.kind() == "import_header" {
                            extract_import_ref(import, source, &mut refs);
                        }
                    }
                }
                _ => {}
            }
        }

        refs
    }
}

fn extract_import_ref(node: Node<'_>, source: &[u8], out: &mut Vec<RawReference>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() != "identifier" {
            continue;
        }

        let full_path = node_text(child, source);
        let last_segment = match full_path.rsplit('.').next() {
            Some(s) => s.to_string(),
            None => full_path.clone(),
        };

        if last_segment == "*" {
            continue;
        }

        let line = node.start_position().row as u32 + 1;

        out.push(RawReference {
            source_symbol: String::new(),
            target_name: last_segment,
            kind: "import",
            line,
        });
    }
}

fn visit_node(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(sym) = extract_function(node, source) {
                out.push(sym);
            }
        }
        "class_declaration" => {
            if let Some(sym) = extract_class(node, source) {
                out.push(sym);
            }
        }
        "object_declaration" => {
            if let Some(sym) = extract_object(node, source) {
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

    let kind = infer_class_kind(node, source);

    let mut children = Vec::new();
    if let Some(body) = find_child_by_kind(node, "class_body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_declaration"
                && let Some(method) = extract_function(child, source)
            {
                children.push(ParsedSymbol {
                    kind: "method",
                    ..method
                });
            }
        }
    }

    Some(ParsedSymbol {
        name,
        kind,
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children,
    })
}

fn extract_object(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let mut children = Vec::new();
    if let Some(body) = find_child_by_kind(node, "class_body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_declaration"
                && let Some(method) = extract_function(child, source)
            {
                children.push(ParsedSymbol {
                    kind: "method",
                    ..method
                });
            }
        }
    }

    Some(ParsedSymbol {
        name,
        kind: "module",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children,
    })
}

fn infer_class_kind<'a>(node: Node<'_>, source: &[u8]) -> &'a str {
    let content = node_text(node, source);
    let trimmed = content.trim_start();
    if trimmed.starts_with("interface ") {
        "interface"
    } else if trimmed.starts_with("enum ") || trimmed.starts_with("enum class ") {
        "enum"
    } else {
        "class"
    }
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
