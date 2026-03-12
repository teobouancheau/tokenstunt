mod helpers;
mod python;
mod typescript;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use crate::languages::{Language, LanguageRegistry};
use python::PythonExtractor;
use typescript::TypeScriptExtractor;

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

#[derive(Debug, Clone)]
pub struct RawReference {
    pub source_symbol: String,
    pub target_name: String,
    pub kind: &'static str,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct ParseResult {
    pub symbols: Vec<ParsedSymbol>,
    pub references: Vec<RawReference>,
}

pub(crate) trait LanguageExtractor {
    fn extract_symbols(&self, root: Node<'_>, source: &[u8]) -> Vec<ParsedSymbol>;
    fn extract_references(&self, root: Node<'_>, source: &[u8]) -> Vec<RawReference>;
}

pub struct SymbolExtractor {
    registry: LanguageRegistry,
}

impl SymbolExtractor {
    pub fn new(registry: LanguageRegistry) -> Self {
        Self { registry }
    }

    pub fn extract(&self, source: &str, language: Language) -> Result<ParseResult> {
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

        let (symbols, references) = match language {
            Language::TypeScript | Language::Tsx | Language::JavaScript => {
                let extractor = TypeScriptExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            Language::Python => {
                let extractor = PythonExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            _ => (vec![], vec![]),
        };

        Ok(ParseResult {
            symbols,
            references,
        })
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
        let result = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        let symbols = &result.symbols;
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
        let result = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        let symbols = &result.symbols;
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
        let result = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        let symbols = &result.symbols;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Config");
        assert_eq!(symbols[0].kind, "interface");
    }

    #[test]
    fn test_typescript_arrow_function() {
        let src = r#"const fetchData = async (url: string): Promise<Response> => {
    return fetch(url);
};"#;
        let result = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        let symbols = &result.symbols;
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
        let result = make_extractor().extract(src, Language::Python).unwrap();
        let symbols = &result.symbols;
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
        let result = make_extractor().extract(src, Language::Python).unwrap();
        let symbols = &result.symbols;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "DataProcessor");
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[0].children.len(), 2);
    }

    #[test]
    fn test_parse_result_contains_empty_references() {
        let src = "function hello() {}";
        let result = make_extractor()
            .extract(src, Language::TypeScript)
            .unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert!(result.references.is_empty());
    }
}
