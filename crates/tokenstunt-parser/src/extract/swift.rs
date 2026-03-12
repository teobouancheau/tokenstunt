use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct SwiftExtractor;

impl LanguageExtractor for SwiftExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            visit_node(child, source, &mut symbols);
        }

        symbols
    }

    fn extract_references(&self, _root: Node<'_>, _source: &[u8]) -> Vec<RawReference> {
        vec![]
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
            let kind = infer_class_kind(node, source);
            if let Some(sym) = extract_class_like(node, source, kind) {
                out.push(sym);
            }
        }
        "protocol_declaration" => {
            if let Some(sym) = extract_class_like(node, source, "interface") {
                out.push(sym);
            }
        }
        _ => {}
    }
}

fn infer_class_kind<'a>(node: Node<'_>, source: &[u8]) -> &'a str {
    let content = node_text(node, source);
    let trimmed = content.trim_start();
    if trimmed.starts_with("struct ") {
        "struct"
    } else if trimmed.starts_with("enum ") {
        "enum"
    } else {
        "class"
    }
}

fn extract_function(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    Some(ParsedSymbol {
        name,
        kind: "function",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: vec![],
    })
}

fn extract_class_like(
    node: Node<'_>,
    source: &[u8],
    kind: &'static str,
) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_declaration" {
                if let Some(method) = extract_function(child, source) {
                    children.push(ParsedSymbol {
                        kind: "method",
                        ..method
                    });
                }
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
        children,
    })
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
