use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct RubyExtractor;

impl LanguageExtractor for RubyExtractor {
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
        "method" => {
            if let Some(sym) = extract_method(node, source) {
                out.push(sym);
            }
        }
        "singleton_method" => {
            if let Some(sym) = extract_singleton_method(node, source) {
                out.push(sym);
            }
        }
        "class" => {
            if let Some(sym) = extract_class(node, source) {
                out.push(sym);
            }
        }
        "module" => {
            if let Some(sym) = extract_module(node, source) {
                out.push(sym);
            }
        }
        "assignment" => {
            if let Some(sym) = extract_constant_assignment(node, source) {
                out.push(sym);
            }
        }
        _ => {}
    }
}

fn extract_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let params = child_text_by_field(node, "parameters", source).unwrap_or_default();

    let signature = format!("def {name}{params}");

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

fn extract_singleton_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let params = child_text_by_field(node, "parameters", source).unwrap_or_default();

    let object = child_text_by_field(node, "object", source).unwrap_or_default();
    let signature = format!("def {object}.{name}{params}");

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
                "method" => {
                    if let Some(method) = extract_method(child, source) {
                        methods.push(ParsedSymbol {
                            kind: "method",
                            ..method
                        });
                    }
                }
                "singleton_method" => {
                    if let Some(method) = extract_singleton_method(child, source) {
                        methods.push(ParsedSymbol {
                            kind: "method",
                            ..method
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let superclass = child_text_by_field(node, "superclass", source);
    let signature = match superclass {
        Some(sc) => format!("class {name} < {sc}"),
        None => format!("class {name}"),
    };

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

fn extract_module(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);

    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method" => {
                    if let Some(method) = extract_method(child, source) {
                        children.push(ParsedSymbol {
                            kind: "method",
                            ..method
                        });
                    }
                }
                "singleton_method" => {
                    if let Some(method) = extract_singleton_method(child, source) {
                        children.push(ParsedSymbol {
                            kind: "method",
                            ..method
                        });
                    }
                }
                "class" => {
                    if let Some(cls) = extract_class(child, source) {
                        children.push(cls);
                    }
                }
                "module" => {
                    if let Some(m) = extract_module(child, source) {
                        children.push(m);
                    }
                }
                _ => {}
            }
        }
    }

    let signature = format!("module {name}");

    Some(ParsedSymbol {
        name,
        kind: "module",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children,
    })
}

fn extract_constant_assignment(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let left = node.child_by_field_name("left")?;
    if left.kind() != "constant" {
        return None;
    }
    let name = node_text(left, source);
    let content = node_text(node, source);

    Some(ParsedSymbol {
        name,
        kind: "constant",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature: String::new(),
        children: vec![],
    })
}
