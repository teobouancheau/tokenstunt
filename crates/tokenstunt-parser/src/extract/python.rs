use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct PythonExtractor;

impl LanguageExtractor for PythonExtractor {
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
        "function_definition" => {
            if let Some(sym) = extract_function(node, source) {
                out.push(sym);
            }
        }
        "class_definition" => {
            if let Some(sym) = extract_class(node, source) {
                out.push(sym);
            }
        }
        "decorated_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "decorator" {
                    visit_node(child, source, out);
                    if let Some(last) = out.last_mut() {
                        last.start_line = node.start_position().row as u32 + 1;
                        last.content = node_text(node, source);
                    }
                }
            }
        }
        "expression_statement" => {
            if let Some(sym) = extract_assignment(node, source) {
                out.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_function(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let params = child_text_by_field(node, "parameters", source).unwrap_or_default();
    let return_type = child_text_by_field(node, "return_type", source);

    let signature = match return_type {
        Some(rt) => format!("def {name}{params} -> {rt}"),
        None => format!("def {name}{params}"),
    };

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

    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(method) = extract_function(child, source) {
                        methods.push(ParsedSymbol {
                            kind: "method",
                            ..method
                        });
                    }
                }
                "decorated_definition" => {
                    let mut inner_cursor = child.walk();
                    for inner in child.children(&mut inner_cursor) {
                        if inner.kind() == "function_definition" {
                            if let Some(method) = extract_function(inner, source) {
                                methods.push(ParsedSymbol {
                                    kind: "method",
                                    start_line: child.start_position().row as u32 + 1,
                                    content: node_text(child, source),
                                    ..method
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let signature = format!("class {name}");

    Some(ParsedSymbol {
        name,
        kind: "class",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: methods,
    })
}

fn extract_assignment(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "assignment" {
            let left = child.child_by_field_name("left")?;
            if left.kind() != "identifier" {
                return None;
            }
            let name = node_text(left, source);
            if !name.chars().next().is_some_and(|c| c.is_uppercase()) {
                return None;
            }
            let content = node_text(node, source);
            return Some(ParsedSymbol {
                name,
                kind: "constant",
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature: String::new(),
                children: vec![],
            });
        }
    }
    None
}
