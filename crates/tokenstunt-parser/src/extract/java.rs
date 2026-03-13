use tree_sitter::Node;

use super::helpers::{child_text_by_field, extract_preceding_comments, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

const COMMENT_KINDS: &[&str] = &["block_comment", "line_comment"];

pub(crate) struct JavaExtractor;

impl LanguageExtractor for JavaExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        collect_declarations(root, source, &mut symbols);
        symbols
    }

    fn extract_references(&self, root: Node<'_>, source: &[u8]) -> Vec<RawReference> {
        let mut refs = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "import_declaration" {
                extract_import_ref(child, source, &mut refs);
            }
        }

        refs
    }
}

fn extract_import_ref(node: Node<'_>, source: &[u8], out: &mut Vec<RawReference>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let target_name = match child.kind() {
            "scoped_identifier" => {
                let name_node = match child.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let name = node_text(name_node, source);
                // skip wildcard imports
                if name == "*" {
                    continue;
                }
                name
            }
            "identifier" => node_text(child, source),
            _ => continue,
        };

        let line = node.start_position().row as u32 + 1;
        out.push(RawReference {
            source_symbol: String::new(),
            target_name,
            kind: "import",
            line,
        });
        // only one import path per declaration
        return;
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
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

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
        docstring,
        children: methods,
    });

    out.extend(constants);
}

fn extract_interface(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let body = node.child_by_field_name("body")?;
    let (methods, _) = extract_class_members(body, source);

    Some(ParsedSymbol {
        name,
        kind: "interface",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: methods,
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

fn extract_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    Some(ParsedSymbol {
        name,
        kind: "method",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: vec![],
    })
}

fn extract_class_members(body: Node<'_>, source: &[u8]) -> (Vec<ParsedSymbol>, Vec<ParsedSymbol>) {
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

            let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);
            out.push(ParsedSymbol {
                name,
                kind: "constant",
                start_line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                content,
                signature,
                docstring,
                children: vec![],
            });
        }
    }
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
