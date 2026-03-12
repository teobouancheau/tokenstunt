mod c_lang;
#[cfg(feature = "lang-dart")]
mod dart;
mod go;
mod helpers;
mod java;
#[cfg(feature = "lang-kotlin")]
mod kotlin;
mod python;
mod ruby;
mod rust_lang;
#[cfg(feature = "lang-swift")]
mod swift;
mod typescript;

use anyhow::{Context, Result};
use tree_sitter::{Node, Parser};

use crate::languages::{Language, LanguageRegistry};
use c_lang::CExtractor;
#[cfg(feature = "lang-dart")]
use dart::DartExtractor;
use go::GoExtractor;
use java::JavaExtractor;
#[cfg(feature = "lang-kotlin")]
use kotlin::KotlinExtractor;
use python::PythonExtractor;
use ruby::RubyExtractor;
use rust_lang::RustExtractor;
#[cfg(feature = "lang-swift")]
use swift::SwiftExtractor;
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
            Language::Rust => {
                let extractor = RustExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            Language::Go => {
                let extractor = GoExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            Language::Java => {
                let extractor = JavaExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            Language::C | Language::Cpp => {
                let extractor = CExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            Language::Ruby => {
                let extractor = RubyExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            #[cfg(feature = "lang-swift")]
            Language::Swift => {
                let extractor = SwiftExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            #[cfg(feature = "lang-kotlin")]
            Language::Kotlin => {
                let extractor = KotlinExtractor;
                (
                    extractor.extract_symbols(root, source_bytes),
                    extractor.extract_references(root, source_bytes),
                )
            }
            #[cfg(feature = "lang-dart")]
            Language::Dart => {
                let extractor = DartExtractor;
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
    fn test_rust_function_and_struct() {
        let src = r#"
use std::collections::HashMap;

pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub struct Config {
    pub port: u16,
    pub host: String,
}

impl Config {
    pub fn new(port: u16, host: String) -> Self {
        Self { port, host }
    }
}

pub trait Service {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>>;
}

pub enum Status {
    Running,
    Stopped,
    Error(String),
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Rust).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {names:?}");
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(
            names.contains(&"Service"),
            "missing Service, got: {names:?}"
        );
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");
    }

    #[test]
    fn test_rust_impl_methods() {
        let src = r#"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn default_port() -> u16 {
        8080
    }
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Rust).unwrap();
        let methods: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "method")
            .map(|s| s.name.as_str())
            .collect();
        assert!(methods.contains(&"new"), "missing new, got: {methods:?}");
        assert!(
            methods.contains(&"default_port"),
            "missing default_port, got: {methods:?}"
        );
    }

    #[test]
    fn test_rust_trait_methods() {
        let src = r#"
pub trait Service {
    fn start(&self) -> Result<(), Box<dyn std::error::Error>>;
    fn stop(&self);
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Rust).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "Service");
        assert_eq!(result.symbols[0].kind, "trait");
        let method_names: Vec<&str> = result.symbols[0]
            .children
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"start"),
            "missing start, got: {method_names:?}"
        );
        assert!(
            method_names.contains(&"stop"),
            "missing stop, got: {method_names:?}"
        );
    }

    #[test]
    fn test_rust_const_and_static() {
        let src = r#"
const MAX_SIZE: usize = 1024;
static COUNTER: AtomicUsize = AtomicUsize::new(0);
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Rust).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"MAX_SIZE"),
            "missing MAX_SIZE, got: {names:?}"
        );
        assert!(
            names.contains(&"COUNTER"),
            "missing COUNTER, got: {names:?}"
        );
        assert!(result.symbols.iter().all(|s| s.kind == "constant"));
    }

    #[test]
    fn test_go_function_and_struct() {
        let src = r#"
package main

func Greet(name string) string {
    return "Hello, " + name + "!"
}

type Config struct {
    Port int
    Host string
}

type Service interface {
    Start() error
    Stop()
}

const MaxSize = 1024

var DefaultHost = "localhost"
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Go).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Greet"), "missing Greet, got: {names:?}");
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(
            names.contains(&"Service"),
            "missing Service, got: {names:?}"
        );
        assert!(
            names.contains(&"MaxSize"),
            "missing MaxSize, got: {names:?}"
        );
        assert!(
            names.contains(&"DefaultHost"),
            "missing DefaultHost, got: {names:?}"
        );
    }

    #[test]
    fn test_go_method_declaration() {
        let src = r#"
package main

type Server struct {
    port int
}

func (s *Server) Start() error {
    return nil
}

func (s *Server) Stop() {
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Go).unwrap();
        let methods: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == "method")
            .map(|s| s.name.as_str())
            .collect();
        assert!(methods.contains(&"Start"), "missing Start, got: {methods:?}");
        assert!(methods.contains(&"Stop"), "missing Stop, got: {methods:?}");
    }

    #[test]
    fn test_go_interface_methods() {
        let src = r#"
package main

type Reader interface {
    Read(p []byte) (n int, err error)
    Close() error
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Go).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "Reader");
        assert_eq!(result.symbols[0].kind, "interface");
        let method_names: Vec<&str> = result.symbols[0]
            .children
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"Read"),
            "missing Read, got: {method_names:?}"
        );
        assert!(
            method_names.contains(&"Close"),
            "missing Close, got: {method_names:?}"
        );
    }

    #[test]
    fn test_java_class_method_interface_enum() {
        let src = r#"
public class UserService {
    private static final int MAX_USERS = 100;

    public User getUser(String id) {
        return users.get(id);
    }

    public void deleteUser(String id) {
        users.remove(id);
    }
}

public interface Repository<T> {
    T findById(String id);
    void save(T entity);
}

public enum Status {
    ACTIVE,
    INACTIVE,
    DELETED
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Java).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"UserService"),
            "missing UserService, got: {names:?}"
        );
        assert!(
            names.contains(&"Repository"),
            "missing Repository, got: {names:?}"
        );
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");
        assert!(
            names.contains(&"MAX_USERS"),
            "missing MAX_USERS, got: {names:?}"
        );

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "getUser");
        assert_eq!(class.children[1].name, "deleteUser");

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .unwrap();
        assert_eq!(iface.kind, "interface");
        assert_eq!(iface.children.len(), 2);

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .unwrap();
        assert_eq!(enm.kind, "enum");
    }

    #[test]
    fn test_c_function_struct_enum() {
        let src = r#"
struct Config {
    int port;
    char* host;
};

enum Status {
    RUNNING,
    STOPPED,
    ERROR
};

void greet(const char* name) {
    printf("Hello, %s!\n", name);
}

int add(int a, int b) {
    return a + b;
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::C).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {names:?}");
        assert!(names.contains(&"add"), "missing add, got: {names:?}");
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");

        let config = result
            .symbols
            .iter()
            .find(|s| s.name == "Config")
            .unwrap();
        assert_eq!(config.kind, "struct");

        let status = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .unwrap();
        assert_eq!(status.kind, "enum");

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        assert_eq!(greet.kind, "function");
    }

    #[test]
    fn test_cpp_class_struct_enum_function() {
        let src = r#"
struct Point {
    double x;
    double y;
};

enum Color {
    RED,
    GREEN,
    BLUE
};

class UserService {
public:
    void getUser(int id) {
        return;
    }

    void deleteUser(int id) {
        return;
    }
};

int main() {
    return 0;
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Cpp).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "missing Point, got: {names:?}");
        assert!(names.contains(&"Color"), "missing Color, got: {names:?}");
        assert!(
            names.contains(&"UserService"),
            "missing UserService, got: {names:?}"
        );
        assert!(names.contains(&"main"), "missing main, got: {names:?}");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "getUser");
        assert_eq!(class.children[0].kind, "method");

        let point = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .unwrap();
        assert_eq!(point.kind, "struct");

        let color = result
            .symbols
            .iter()
            .find(|s| s.name == "Color")
            .unwrap();
        assert_eq!(color.kind, "enum");
    }

    #[test]
    fn test_ruby_class_method_module_constant() {
        let src = r#"
module Helpers
  def format_name(name)
    name.strip.capitalize
  end
end

class User < ActiveRecord::Base
  def initialize(name)
    @name = name
  end

  def greet
    "Hello, #{@name}!"
  end

  def self.find_by_name(name)
    new(name)
  end
end

MAX_RETRIES = 3
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Ruby).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Helpers"),
            "missing Helpers, got: {names:?}"
        );
        assert!(names.contains(&"User"), "missing User, got: {names:?}");
        assert!(
            names.contains(&"MAX_RETRIES"),
            "missing MAX_RETRIES, got: {names:?}"
        );

        let module = result
            .symbols
            .iter()
            .find(|s| s.name == "Helpers")
            .unwrap();
        assert_eq!(module.kind, "module");
        assert_eq!(module.children.len(), 1);
        assert_eq!(module.children[0].name, "format_name");
        assert_eq!(module.children[0].kind, "method");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 3);
        assert_eq!(class.children[0].name, "initialize");
        assert_eq!(class.children[0].kind, "method");
        assert_eq!(class.children[1].name, "greet");
        assert_eq!(class.children[2].name, "find_by_name");
        assert!(class.signature.contains("< ActiveRecord::Base"));

        let constant = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_RETRIES")
            .unwrap();
        assert_eq!(constant.kind, "constant");
    }

    #[test]
    fn test_ruby_standalone_method() {
        let src = r#"
def process(data)
  data.map { |x| x * 2 }
end
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Ruby).unwrap();
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "process");
        assert_eq!(result.symbols[0].kind, "function");
    }

    #[cfg(feature = "lang-swift")]
    #[test]
    fn test_swift_class_function_protocol_enum() {
        let src = r#"
func greet(name: String) -> String {
    return "Hello, \(name)!"
}

class UserService {
    func getUser(id: String) -> User {
        return users[id]
    }

    func deleteUser(id: String) {
        users.removeValue(forKey: id)
    }
}

struct Config {
    var port: Int
    var host: String
}

protocol Repository {
    func findById(id: String) -> Entity
    func save(entity: Entity)
}

enum Status {
    case active
    case inactive
    case deleted
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Swift).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {names:?}");
        assert!(
            names.contains(&"UserService"),
            "missing UserService, got: {names:?}"
        );
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(
            names.contains(&"Repository"),
            "missing Repository, got: {names:?}"
        );
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        assert_eq!(greet.kind, "function");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "getUser");
        assert_eq!(class.children[0].kind, "method");

        let config = result
            .symbols
            .iter()
            .find(|s| s.name == "Config")
            .unwrap();
        assert_eq!(config.kind, "struct");

        let protocol = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .unwrap();
        assert_eq!(protocol.kind, "interface");

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .unwrap();
        assert_eq!(enm.kind, "enum");
    }

    #[cfg(feature = "lang-kotlin")]
    #[test]
    fn test_kotlin_class_function_object_interface() {
        let src = r#"
fun greet(name: String): String {
    return "Hello, $name!"
}

class UserService {
    fun getUser(id: String): User {
        return users[id]
    }

    fun deleteUser(id: String) {
        users.remove(id)
    }
}

object AppConfig {
    fun getPort(): Int {
        return 8080
    }
}

interface Repository {
    fun findById(id: String): Entity
    fun save(entity: Entity)
}

enum class Status {
    ACTIVE,
    INACTIVE,
    DELETED
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Kotlin).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {names:?}");
        assert!(
            names.contains(&"UserService"),
            "missing UserService, got: {names:?}"
        );
        assert!(
            names.contains(&"AppConfig"),
            "missing AppConfig, got: {names:?}"
        );
        assert!(
            names.contains(&"Repository"),
            "missing Repository, got: {names:?}"
        );
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        assert_eq!(greet.kind, "function");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "getUser");
        assert_eq!(class.children[0].kind, "method");

        let obj = result
            .symbols
            .iter()
            .find(|s| s.name == "AppConfig")
            .unwrap();
        assert_eq!(obj.kind, "module");

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .unwrap();
        assert_eq!(iface.kind, "interface");

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .unwrap();
        assert_eq!(enm.kind, "enum");
    }

    #[cfg(feature = "lang-dart")]
    #[test]
    fn test_dart_class_function_enum() {
        let src = r#"
String greet(String name) {
    return "Hello, $name!";
}

class UserService {
    User getUser(String id) {
        return users[id];
    }

    void deleteUser(String id) {
        users.remove(id);
    }
}

enum Status {
    active,
    inactive,
    deleted,
}
"#;
        let extractor = make_extractor();
        let result = extractor.extract(src, Language::Dart).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "missing greet, got: {names:?}");
        assert!(
            names.contains(&"UserService"),
            "missing UserService, got: {names:?}"
        );
        assert!(names.contains(&"Status"), "missing Status, got: {names:?}");

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        assert_eq!(greet.kind, "function");

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.children.len(), 2);
        assert_eq!(class.children[0].name, "getUser");
        assert_eq!(class.children[0].kind, "method");

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .unwrap();
        assert_eq!(enm.kind, "enum");
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
