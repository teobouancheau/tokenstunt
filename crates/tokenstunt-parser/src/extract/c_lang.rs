use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct CExtractor;

impl LanguageExtractor for CExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        collect_declarations(root, source, &mut symbols);
        symbols
    }

    fn extract_references(&self, root: Node<'_>, source: &[u8]) -> Vec<RawReference> {
        let mut refs = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() != "preproc_include" {
                continue;
            }

            let path_node = match child.child_by_field_name("path") {
                Some(n) => n,
                None => continue,
            };

            let raw = node_text(path_node, source);
            let stripped = raw.trim_matches(|c| c == '<' || c == '>' || c == '"');
            let target_name = stripped
                .rsplit('/')
                .next()
                .unwrap_or(stripped)
                .trim_end_matches(".h")
                .to_string();

            refs.push(RawReference {
                source_symbol: String::new(),
                target_name,
                kind: "import",
                line: child.start_position().row as u32 + 1,
            });
        }

        refs
    }
}

fn collect_declarations(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sym) = extract_function(child, source) {
                    out.push(sym);
                }
            }
            "struct_specifier" => {
                if let Some(sym) = extract_struct(child, source) {
                    out.push(sym);
                }
            }
            "class_specifier" => {
                if let Some(sym) = extract_class(child, source) {
                    out.push(sym);
                }
            }
            "enum_specifier" => {
                if let Some(sym) = extract_enum(child, source) {
                    out.push(sym);
                }
            }
            "declaration" => {
                collect_declarations(child, source, out);
            }
            "namespace_definition" => {
                if let Some(body) = child.child_by_field_name("body") {
                    collect_declarations(body, source, out);
                }
            }
            _ => {
                collect_declarations(child, source, out);
            }
        }
    }
}

fn extract_function(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = find_function_name(declarator, source)?;
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

fn find_function_name(declarator: Node<'_>, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "function_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            find_function_name(inner, source)
        }
        "pointer_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            find_function_name(inner, source)
        }
        "qualified_identifier" | "field_identifier" | "identifier" => {
            Some(node_text(declarator, source))
        }
        "destructor_name" => Some(node_text(declarator, source)),
        _ => child_text_by_field(declarator, "name", source),
    }
}

fn extract_struct(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    let methods = extract_body_methods(node, source);

    Some(ParsedSymbol {
        name,
        kind: "struct",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        children: methods,
    })
}

fn extract_class(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);

    let methods = extract_body_methods(node, source);

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

fn extract_body_methods(node: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
    let mut methods = Vec::new();
    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return methods,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_definition"
            && let Some(sym) = extract_function(child, source)
        {
            methods.push(ParsedSymbol {
                kind: "method",
                ..sym
            });
        }
    }

    methods
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

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}
