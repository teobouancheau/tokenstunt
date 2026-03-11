use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use crate::languages::{Language, LanguageRegistry};

#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: &'static str,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub signature: String,
    pub children: Vec<ParsedSymbol>,
}

pub struct SymbolExtractor {
    registry: LanguageRegistry,
}

impl SymbolExtractor {
    pub fn new(registry: LanguageRegistry) -> Self {
        Self { registry }
    }

    pub fn extract(&self, source: &str, language: Language) -> Result<Vec<ParsedSymbol>> {
        let ts_lang = self.registry.get_ts_language(language)?;
        let mut parser = Parser::new();
        parser
            .set_language(&ts_lang)
            .context("failed to set parser language")?;

        let tree = parser
            .parse(source, None)
            .context("failed to parse source")?;

        let root = tree.root_node();
        let source_bytes = source.as_bytes();

        match language {
            Language::TypeScript | Language::Tsx | Language::JavaScript => {
                Ok(self.extract_typescript(root, source_bytes))
            }
            Language::Python => Ok(self.extract_python(root, source_bytes)),
            _ => Ok(vec![]),
        }
    }

    fn extract_typescript(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            self.visit_ts_node(child, source, &mut symbols);
        }

        symbols
    }

    fn visit_ts_node(&self, node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
        match node.kind() {
            "function_declaration" => {
                if let Some(sym) = self.extract_ts_function(node, source) {
                    out.push(sym);
                }
            }
            "class_declaration" => {
                if let Some(sym) = self.extract_ts_class(node, source) {
                    out.push(sym);
                }
            }
            "interface_declaration" => {
                if let Some(sym) = self.extract_ts_interface(node, source) {
                    out.push(sym);
                }
            }
            "type_alias_declaration" => {
                if let Some(sym) = self.extract_ts_type_alias(node, source) {
                    out.push(sym);
                }
            }
            "enum_declaration" => {
                if let Some(sym) = self.extract_ts_enum(node, source) {
                    out.push(sym);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.extract_ts_variable_decl(node, source, out);
            }
            "export_statement" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.visit_ts_node(child, source, out);
                }
            }
            _ => {}
        }
    }

    fn extract_ts_function(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);
        let signature = self.extract_ts_function_signature(node, source);

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

    fn extract_ts_function_signature(&self, node: Node<'_>, source: &[u8]) -> String {
        let name = self
            .child_text_by_field(node, "name", source)
            .unwrap_or_default();
        let params = self
            .child_text_by_field(node, "parameters", source)
            .unwrap_or_default();
        let return_type = self
            .child_text_by_field(node, "return_type", source)
            .unwrap_or_default();

        if return_type.is_empty() {
            format!("function {name}{params}")
        } else {
            format!("function {name}{params}{return_type}")
        }
    }

    fn extract_ts_class(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);

        let mut methods = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "method_definition" | "public_field_definition" => {
                        if let Some(method_name) =
                            self.child_text_by_field(child, "name", source)
                        {
                            methods.push(ParsedSymbol {
                                name: method_name,
                                kind: "method",
                                start_line: child.start_position().row as u32 + 1,
                                end_line: child.end_position().row as u32 + 1,
                                content: self.node_text(child, source),
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

    fn extract_ts_interface(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);
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

    fn extract_ts_type_alias(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);
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

    fn extract_ts_enum(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);
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

    fn extract_ts_variable_decl(
        &self,
        node: Node<'_>,
        source: &[u8],
        out: &mut Vec<ParsedSymbol>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                let name = match self.child_text_by_field(child, "name", source) {
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

                let content = self.node_text(node, source);
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

    fn extract_python(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol> {
        let mut symbols = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            self.visit_py_node(child, source, &mut symbols);
        }

        symbols
    }

    fn visit_py_node(&self, node: Node<'_>, source: &[u8], out: &mut Vec<ParsedSymbol>) {
        match node.kind() {
            "function_definition" => {
                if let Some(sym) = self.extract_py_function(node, source) {
                    out.push(sym);
                }
            }
            "class_definition" => {
                if let Some(sym) = self.extract_py_class(node, source) {
                    out.push(sym);
                }
            }
            "decorated_definition" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() != "decorator" {
                        self.visit_py_node(child, source, out);
                        if let Some(last) = out.last_mut() {
                            last.start_line = node.start_position().row as u32 + 1;
                            last.content = self.node_text(node, source);
                        }
                    }
                }
            }
            "expression_statement" => {
                if let Some(sym) = self.extract_py_assignment(node, source) {
                    out.push(sym);
                }
            }
            _ => {}
        }
    }

    fn extract_py_function(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);
        let params = self
            .child_text_by_field(node, "parameters", source)
            .unwrap_or_default();
        let return_type = self.child_text_by_field(node, "return_type", source);

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

    fn extract_py_class(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let name = self.child_text_by_field(node, "name", source)?;
        let content = self.node_text(node, source);

        let mut methods = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "function_definition" => {
                        if let Some(method) = self.extract_py_function(child, source) {
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
                                if let Some(method) = self.extract_py_function(inner, source) {
                                    methods.push(ParsedSymbol {
                                        kind: "method",
                                        start_line: child.start_position().row as u32 + 1,
                                        content: self.node_text(child, source),
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

    fn extract_py_assignment(&self, node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "assignment" {
                let left = child.child_by_field_name("left")?;
                if left.kind() != "identifier" {
                    return None;
                }
                let name = self.node_text_str(left, source);
                if !name.chars().next().is_some_and(|c| c.is_uppercase()) {
                    return None;
                }
                let content = self.node_text(node, source);
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

    fn child_text_by_field(
        &self,
        node: Node<'_>,
        field: &str,
        source: &[u8],
    ) -> Option<String> {
        let child = node.child_by_field_name(field)?;
        Some(self.node_text_str(child, source))
    }

    fn node_text(&self, node: Node<'_>, source: &[u8]) -> String {
        self.node_text_str(node, source)
    }

    fn node_text_str(&self, node: Node<'_>, source: &[u8]) -> String {
        node.utf8_text(source).unwrap_or("").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_extractor() -> SymbolExtractor {
        SymbolExtractor::new(LanguageRegistry::new().unwrap())
    }

    #[test]
    fn test_typescript_function() {
        let src = r#"function greet(name: string): string {
    return `Hello, ${name}!`;
}"#;
        let symbols = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, "function");
        assert_eq!(symbols[0].start_line, 1);
        assert_eq!(symbols[0].end_line, 3);
    }

    #[test]
    fn test_typescript_class() {
        let src = r#"class UserService {
    getUser(id: string): User {
        return this.users.get(id);
    }
    deleteUser(id: string): void {
        this.users.delete(id);
    }
}"#;
        let symbols = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "UserService");
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[0].children.len(), 2);
        assert_eq!(symbols[0].children[0].name, "getUser");
    }

    #[test]
    fn test_typescript_interface() {
        let src = r#"interface Config {
    port: number;
    host: string;
}"#;
        let symbols = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Config");
        assert_eq!(symbols[0].kind, "interface");
    }

    #[test]
    fn test_typescript_arrow_function() {
        let src = r#"const fetchData = async (url: string): Promise<Response> => {
    return fetch(url);
};"#;
        let symbols = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "fetchData");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_python_function() {
        let src = r#"def process_data(items: list[str]) -> dict:
    result = {}
    for item in items:
        result[item] = len(item)
    return result"#;
        let symbols = make_extractor().extract(src, Language::Python).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "process_data");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_python_class() {
        let src = r#"class DataProcessor:
    def __init__(self, config: Config):
        self.config = config

    def run(self) -> None:
        pass"#;
        let symbols = make_extractor().extract(src, Language::Python).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "DataProcessor");
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[0].children.len(), 2);
    }
}
