use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct TypeScriptExtractor;

impl LanguageExtractor for TypeScriptExtractor {
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
            if let Some(sym) = extract_class(node, source) {
                out.push(sym);
            }
        }
        "interface_declaration" => {
            if let Some(sym) = extract_interface(node, source) {
                out.push(sym);
            }
        }
        "type_alias_declaration" => {
            if let Some(sym) = extract_type_alias(node, source) {
                out.push(sym);
            }
        }
        "enum_declaration" => {
            if let Some(sym) = extract_enum(node, source) {
                out.push(sym);
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            extract_variable_decl(node, source, out);
        }
        "export_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit_node(child, source, out);
            }
        }
        _ => {}
    }
}

fn extract_function(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_function_signature(node, source);

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

fn extract_function_signature(node: Node<'_>, source: &[u8]) -> String {
    let name = child_text_by_field(node, "name", source).unwrap_or_default();
    let params = child_text_by_field(node, "parameters", source).unwrap_or_default();
    let return_type = child_text_by_field(node, "return_type", source).unwrap_or_default();

    if return_type.is_empty() {
        format!("function {name}{params}")
    } else {
        format!("function {name}{params}{return_type}")
    }
}

fn extract_class(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);

    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_definition" | "public_field_definition" => {
                    if let Some(method_name) = child_text_by_field(child, "name", source) {
                        methods.push(ParsedSymbol {
                            name: method_name,
                            kind: "method",
                            start_line: child.start_position().row as u32 + 1,
                            end_line: child.end_position().row as u32 + 1,
                            content: node_text(child, source),
                            signature: String::new(),
                            children: vec![],
                        });
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

fn extract_interface(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("interface {name}");

    Some(ParsedSymbol {
        name,
        kind: "interface",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: vec![],
    })
}

fn extract_type_alias(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("type {name}");

    Some(ParsedSymbol {
        name,
        kind: "type_alias",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: vec![],
    })
}

fn extract_enum(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("enum {name}");

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

fn extract_variable_decl(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = match child_text_by_field(child, "name", source) {
                Some(n) => n,
                None => continue,
            };

            let value = child.child_by_field_name("value");
            let is_arrow_or_function = value.is_some_and(|v| {
                matches!(v.kind(), "arrow_function" | "function_expression" | "function")
            });

            let kind = if is_arrow_or_function {
                "function"
            } else {
                "constant"
            };

            let content = node_text(node, source);
            let signature = format!("const {name}");

            out.push(ParsedSymbol {
                name,
                kind,
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature,
                children: vec![],
            });
        }
    }
}
