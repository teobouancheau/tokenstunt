use tree_sitter::Node;

use super::helpers::{child_text_by_field, extract_preceding_comments, node_text};
use super::{LanguageExtractor, ParsedSymbol, RawReference};

const COMMENT_KINDS: &[&str] = &["line_comment", "block_comment"];

pub(crate) struct RustExtractor;

impl LanguageExtractor for RustExtractor {
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
            if child.kind() == "use_declaration" {
                extract_use_refs(child, source, &mut refs);
            }
        }

        refs
    }
}

fn visit_node(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    match node.kind() {
        "function_item" => {
            if let Some(sym) = extract_function(node, source) {
                out.push(sym);
            }
        }
        "struct_item" => {
            if let Some(sym) = extract_struct(node, source) {
                out.push(sym);
            }
        }
        "enum_item" => {
            if let Some(sym) = extract_enum(node, source) {
                out.push(sym);
            }
        }
        "trait_item" => {
            if let Some(sym) = extract_trait(node, source) {
                out.push(sym);
            }
        }
        "impl_item" => {
            extract_impl(node, source, out);
        }
        "const_item" | "static_item" => {
            if let Some(sym) = extract_const(node, source) {
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
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

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

fn extract_struct(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("struct {name}");
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    Some(ParsedSymbol {
        name,
        kind: "struct",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: vec![],
    })
}

fn extract_enum(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("enum {name}");
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

fn extract_trait(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = format!("trait {name}");
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    let mut methods = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if (child.kind() == "function_signature_item" || child.kind() == "function_item")
                && let Some(method_name) = child_text_by_field(child, "name", source)
            {
                let method_content = node_text(child, source);
                let method_doc = extract_preceding_comments(child, source, COMMENT_KINDS);
                methods.push(ParsedSymbol {
                    name: method_name,
                    kind: "method",
                    start_line: child.start_position().row as u32 + 1,
                    end_line: child.end_position().row as u32 + 1,
                    content: method_content.clone(),
                    signature: extract_first_line(&method_content),
                    docstring: method_doc,
                    children: vec![],
                });
            }
        }
    }

    Some(ParsedSymbol {
        name,
        kind: "trait",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: methods,
    })
}

fn extract_impl(node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
    let type_node = node.child_by_field_name("type");
    let type_name = type_node.map(|n| node_text(n, source));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "function_item"
                && let Some(mut sym) = extract_function(child, source)
            {
                sym.kind = "method";
                if let Some(ref tn) = type_name {
                    sym.signature = format!("impl {tn} :: {}", extract_first_line(&sym.content));
                }
                out.push(sym);
            }
        }
    }
}

fn extract_const(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let name = child_text_by_field(node, "name", source)?;
    let content = node_text(node, source);
    let signature = extract_first_line(&content);
    let docstring = extract_preceding_comments(node, source, COMMENT_KINDS);

    Some(ParsedSymbol {
        name,
        kind: "constant",
        start_line: node.start_position().row as u32 + 1,
        end_line: node.end_position().row as u32 + 1,
        content,
        signature,
        docstring,
        children: vec![],
    })
}

fn extract_first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

fn extract_use_refs(node: Node<'_>, source: &[u8], out: &mut Vec<RawReference>) {
    let line = node.start_position().row as u32 + 1;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        collect_use_names(child, source, line, out);
    }
}

fn collect_use_names(node: Node<'_>, source: &[u8], line: u32, out: &mut Vec<RawReference>) {
    match node.kind() {
        // `use foo::bar::Baz;` — last segment is the imported name
        "scoped_identifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                out.push(RawReference {
                    source_symbol: String::new(),
                    target_name: name,
                    kind: "import",
                    line,
                });
            }
        }
        // `use foo::bar::{A, B};` — recurse into the list
        "scoped_use_list" => {
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_names(list, source, line, out);
            }
        }
        // `{A, B, C}` — each child is a use item
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_use_names(child, source, line, out);
            }
        }
        // `A as B` — use the alias (what's actually bound in scope)
        "use_as_clause" => {
            if let Some(alias) = node.child_by_field_name("alias") {
                let name = node_text(alias, source);
                out.push(RawReference {
                    source_symbol: String::new(),
                    target_name: name,
                    kind: "import",
                    line,
                });
            }
        }
        // bare identifier: `use serde;` or inside a use_list
        "identifier" => {
            let name = node_text(node, source);
            out.push(RawReference {
                source_symbol: String::new(),
                target_name: name,
                kind: "import",
                line,
            });
        }
        // `use foo::*;` — glob import, skip (no specific name to resolve)
        "use_wildcard" => {}
        _ => {}
    }
}
