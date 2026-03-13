use tree_sitter::Node;

use super::helpers::{child_text_by_field, extract_preceding_comments, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

const COMMENT_KINDS: &[&str] = &["comment"];

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

    fn extract_references(&self, root: Node<'_>, source: &[u8]) -> Vec<RawReference> {
        let mut refs = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "call"
                && let Some(raw_ref) = extract_require(child, source)
            {
                refs.push(raw_ref);
            }
        }

        refs
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
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let signature = format!("def {name}{params}");

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

fn extract_singleton_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let params = child_text_by_field(node, "parameters", source).unwrap_or_default();
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let object = child_text_by_field(node, "object", source).unwrap_or_default();
    let signature = format!("def {object}.{name}{params}");

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
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

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
        docstring,
        children: methods,
    })
}

fn extract_module(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

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
        docstring,
        children,
    })
}

fn extract_require(node: Node<'_>, source: &[u8]) -> Option<RawReference> {
    let method = node.child_by_field_name("method")?;
    let method_name = node_text(method, source);
    if method_name != "require" && method_name != "require_relative" {
        return None;
    }

    let arguments = node.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let string_node = arguments
        .children(&mut cursor)
        .find(|n| n.kind() == "string")?;

    let mut str_cursor = string_node.walk();
    let content_node = string_node
        .children(&mut str_cursor)
        .find(|n| n.kind() == "string_content")?;

    let raw_path = node_text(content_node, source);
    let target_name = raw_path
        .split('/')
        .next_back()
        .unwrap_or(&raw_path)
        .trim_end_matches(".rb")
        .to_string();

    Some(RawReference {
        source_symbol: String::new(),
        target_name,
        kind: "import",
        line: node.start_position().row as u32 + 1,
    })
}

fn extract_constant_assignment(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let left = node.child_by_field_name("left")?;
    if left.kind() != "constant" {
        return None;
    }
    let name = node_text(left, source);
    let content = node_text(node, source);

    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);
    Some(ParsedSymbol {
        name,
        kind: "constant",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature: String::new(),
        docstring,
        children: vec![],
    })
}
