use tree_sitter::Node;

use super::helpers::{child_text_by_field, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

pub(crate) struct GoExtractor;

impl LanguageExtractor for GoExtractor {
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
            if child.kind() == "import_declaration" {
                extract_import_refs(child, source, &mut refs);
            }
        }

        refs
    }
}

fn visit_node(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(sym) = extract_function(node, source) {
                out.push(sym);
            }
        }
        "method_declaration" => {
            if let Some(sym) = extract_method(node, source) {
                out.push(sym);
            }
        }
        "type_declaration" => {
            extract_type_declaration(node, source, out);
        }
        "const_declaration" | "var_declaration" => {
            extract_const_or_var(node, source, out);
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

fn extract_method(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);

    let receiver = node
        .child_by_field_name("receiver")
        .map(|r| node_text(r, source))
        .unwrap_or_default();

    let signature = format!("func {} {name}(...)", receiver);

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

fn extract_type_declaration(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "type_spec" {
            continue;
        }

        let name = match child_text_by_field(child, "name", source) {
            Some(n) => n,
            None => continue,
        };

        let type_node = match child.child_by_field_name("type") {
            Some(n) => n,
            None => continue,
        };

        let content = node_text(child, source);

        let (kind, signature, children) = match type_node.kind() {
            "struct_type" => {
                let sig = format!("type {name} struct");
                ("struct", sig, vec![])
            }
            "interface_type" => {
                let methods = extract_interface_methods(type_node, source);
                let sig = format!("type {name} interface");
                ("interface", sig, methods)
            }
            _ => {
                let sig = format!("type {name}");
                ("type", sig, vec![])
            }
        };

        out.push(ParsedSymbol {
            name,
            kind,
            start_line: child.start_position().row as u32 + 1,
            end_line: child.end_position().row as u32 + 1,
            content,
            signature,
            children,
        });
    }
}

fn extract_interface_methods(iface_node: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
    let mut methods = Vec::new();
    let mut cursor = iface_node.walk();

    for child in iface_node.children(&mut cursor) {
        if child.kind() != "method_elem" {
            continue;
        }

        let name = match find_first_child_by_kind(child, "field_identifier", source) {
            Some(n) => n,
            None => continue,
        };

        let content = node_text(child, source);

        methods.push(ParsedSymbol {
            name,
            kind: "method",
            start_line: child.start_position().row as u32 + 1,
            end_line: child.end_position().row as u32 + 1,
            content: content.clone(),
            signature: content,
            children: vec![],
        });
    }

    methods
}

fn find_first_child_by_kind(node: Node<'_>, kind: &str, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(child, source));
        }
    }
    None
}

fn extract_const_or_var(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() != "const_spec" && child.kind() != "var_spec" {
            continue;
        }

        let name = match child_text_by_field(child, "name", source) {
            Some(n) => n,
            None => continue,
        };

        let content = node_text(child, source);
        let signature = content.clone();

        out.push(ParsedSymbol {
            name,
            kind: "constant",
            start_line: child.start_position().row as u32 + 1,
            end_line: child.end_position().row as u32 + 1,
            content,
            signature,
            children: vec![],
        });
    }
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

fn extract_import_refs(node: Node<'_>, source: &[u8], out: &mut Vec<RawReference>) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => extract_import_spec(child, source, out),
            "import_spec_list" => {
                let mut list_cursor = child.walk();
                for spec in child.children(&mut list_cursor) {
                    if spec.kind() == "import_spec" {
                        extract_import_spec(spec, source, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_import_spec(node: Node<'_>, source: &[u8], out: &mut Vec<RawReference>) {
    let path_node = match node.child_by_field_name("path") {
        Some(n) => n,
        None => return,
    };

    let raw_path = node_text(path_node, source);
    let path = raw_path.trim_matches('"');

    let target_name = if let Some(alias_node) = node.child_by_field_name("name") {
        let alias = node_text(alias_node, source);
        // "_" is a blank import, "." is a dot import — skip both
        if alias == "_" || alias == "." {
            return;
        }
        alias
    } else {
        path.split('/').next_back().unwrap_or(path).to_string()
    };

    let line = node.start_position().row as u32 + 1;

    out.push(RawReference {
        source_symbol: String::new(),
        target_name,
        kind: "import",
        line,
    });
}
