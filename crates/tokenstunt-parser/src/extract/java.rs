use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct JavaExtractor;

impl LanguageExtractor for JavaExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        collect_declarations(root, source, &mut symbols);
        symbols
    }

    fn extract_references(&self, _root: Node<'_>, _source: &[u8]) -> Vec<RawReference> {
        vec![]
    }
}

fn collect_declarations(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" => {
                extract_class(child, source, out);
            }
            "interface_declaration" => {
                if let Some(sym) = extract_interface(child, source) {
                    out.push(sym);
                }
            }
            "enum_declaration" => {
                if let Some(sym) = extract_enum(child, source) {
                    out.push(sym);
                }
            }
            "method_declaration" => {
                if let Some(sym) = extract_method(child, source) {
                    out.push(sym);
                }
            }
            "field_declaration" => {
                if is_final_field(child, source) {
                    extract_constants(child, source, out);
                }
            }
            _ => {
                collect_declarations(child, source, out);
            }
        }
    }
}

fn extract_class(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let name = match child_text_by_field(node, "name", source) {
        Some(n) => n,
        None => return,
    };
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let (methods, constants) = extract_class_members(body, source);

    out.push(ParsedSymbol {
        name,
        kind: "class",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: methods,
    });

    out.extend(constants);
}

fn extract_interface(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    let body = node.child_by_field_name("body")?;
    let (methods, _) = extract_class_members(body, source);

    Some(ParsedSymbol {
        name,
        kind: "interface",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: methods,
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

fn extract_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    Some(ParsedSymbol {
        name,
        kind: "method",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: vec![],
    })
}

fn extract_class_members(
    body: Node<'_>,
    source: &[u8],
) -> (Vec<ParsedSymbol>, Vec<ParsedSymbol>) {
    let mut methods = Vec::new();
    let mut constants = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                if let Some(sym) = extract_method(child, source) {
                    methods.push(sym);
                }
            }
            "field_declaration" => {
                if is_final_field(child, source) {
                    extract_constants(child, source, &mut constants);
                }
            }
            _ => {}
        }
    }

    (methods, constants)
}

fn is_final_field(node: Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, source);
            if text.contains("final") {
                return true;
            }
        }
    }
    false
}

fn extract_constants(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = match child_text_by_field(child, "name", source) {
                Some(n) => n,
                None => continue,
            };

            let content = node_text(node, source);
            let signature = content.clone();

            out.push(ParsedSymbol {
                name,
                kind: "constant",
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature,
                children: vec![],
            });
        }
    }
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
