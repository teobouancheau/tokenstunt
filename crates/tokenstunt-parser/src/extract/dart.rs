use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

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

    fn extract_references(&self, _root: Node<'_>, _source: &[u8]) -> Vec<RawReference> {
        vec![]
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

fn extract_class(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

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
        children,
    })
}

fn extract_enum(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    Some(ParsedSymbol {
        name,
        kind: "enum",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: vec![],
    })
}

fn collect_methods(body: Node<'_>, source: &[u8], children: &mut Vec<ParsedSymbol>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "method_signature" {
            if let Some(method) = extract_method_signature(child, source) {
                children.push(method);
            }
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

            return Some(ParsedSymbol {
                name,
                kind: "method",
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature,
                children: vec![],
            });
        }
    }
    None
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
