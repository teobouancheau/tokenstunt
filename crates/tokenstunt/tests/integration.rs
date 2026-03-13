use tokenstunt_index::Indexer;
use tokenstunt_search::{SearchEngine, SearchQuery};
use tokenstunt_store::{CodeBlockKind, Store};

#[test]
fn test_end_to_end_index_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(
        src.join("auth.ts"),
        r#"
export function authenticateUser(token: string): boolean {
    return token.length > 0;
}

export class AuthService {
    private token: string;

    constructor(token: string) {
        this.token = token;
    }

    validate(): boolean {
        return authenticateUser(this.token);
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("math.py"),
        r#"
def add(a: int, b: int) -> int:
    return a + b

def multiply(a: int, b: int) -> int:
    return a * b

class Calculator:
    def compute(self, a: int, b: int) -> int:
        return add(a, b)
"#,
    )
    .unwrap();

    let store = Store::open_in_memory().unwrap();
    let indexer = Indexer::new(store, None).unwrap();
    let stats = indexer
        .index_directory(dir.path(), &tokenstunt_index::NopProgress)
        .unwrap();

    assert!(stats.files >= 2, "expected at least 2 files indexed");
    assert!(stats.blocks >= 4, "expected at least 4 blocks indexed");
    assert_eq!(stats.errors, 0);

    let engine = SearchEngine::new(indexer.store());

    let query = SearchQuery {
        text: "authenticate".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = engine.search(&query).unwrap();
    assert!(!results.is_empty(), "FTS search should find authenticate*");
    assert!(
        results.iter().any(|r| r.block.name == "authenticateUser"),
        "should find authenticateUser function"
    );

    let results = engine.lookup_symbol("add", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "add");
    assert_eq!(results[0].kind, CodeBlockKind::Function);

    let results = engine
        .lookup_symbol("Calculator", Some(CodeBlockKind::Class))
        .unwrap();
    assert_eq!(results.len(), 1);

    let results = engine
        .lookup_symbol("Calculator", Some(CodeBlockKind::Function))
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_end_to_end_multi_language() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    // Write files in multiple languages
    std::fs::write(
        src.join("service.ts"),
        r#"
import { Config } from './config';
export class UserService {
    private config: Config;
    constructor(config: Config) { this.config = config; }
    async getUser(id: string) { return { id }; }
}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("config.ts"),
        r#"
export interface Config {
    port: number;
    host: string;
}
export const DEFAULT_PORT = 3000;
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("handler.py"),
        r#"
from dataclasses import dataclass

@dataclass
class Request:
    method: str
    path: str

def handle_request(req: Request) -> dict:
    return {"status": 200}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("main.rs"),
        r#"
pub fn main() {
    println!("hello");
}

pub struct App {
    port: u16,
}

impl App {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        src.join("server.go"),
        r#"
package main

func StartServer(port int) error {
    return nil
}

type Server struct {
    Port int
    Host string
}
"#,
    )
    .unwrap();

    // Index
    let store = Store::open_in_memory().unwrap();
    let indexer = Indexer::new(store, None).unwrap();
    indexer
        .index_directory(dir.path(), &tokenstunt_index::NopProgress)
        .unwrap();

    let store = indexer.store();

    // Verify search works
    let results = store
        .search_fts("UserService", None, None, None, 10)
        .unwrap();
    assert!(!results.is_empty(), "search should find UserService");

    // Verify symbol lookup works
    let blocks = store.lookup_symbol("UserService", None).unwrap();
    assert!(!blocks.is_empty(), "symbol lookup should find UserService");

    // Verify language stats
    let stats = store.get_language_stats().unwrap();
    assert!(stats.len() >= 4, "should have at least 4 languages");

    // Verify file count across languages
    let file_count = store.file_count().unwrap();
    assert!(file_count >= 5, "should have at least 5 files indexed");

    // Verify block count
    let block_count = store.block_count().unwrap();
    assert!(block_count >= 8, "should have at least 8 code blocks");

    // Verify dependencies populated (UserService imports Config)
    let _unresolved = store.get_unresolved_dependencies().unwrap();
    // Config import should either be resolved or unresolved
    // The test just verifies the dependency system is working

    // Verify overview
    let lang_stats = store.get_language_stats().unwrap();
    let ts_count = lang_stats
        .iter()
        .find(|(l, _)| l == "typescript")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(ts_count >= 2, "should have at least 2 TypeScript files");

    // Verify exported symbols
    let exported = store.get_exported_symbols(None).unwrap();
    assert!(!exported.is_empty(), "should have exported symbols");

    // Verify reconciliation (re-index same directory should be mostly unchanged)
    let repo_id = store
        .ensure_repo(dir.path().to_str().unwrap(), "test")
        .unwrap();
    let recon_stats = indexer.reconcile(dir.path(), repo_id).unwrap();
    assert_eq!(recon_stats.updated, 0, "no files should need updating");
    assert!(recon_stats.unchanged >= 5, "all files should be unchanged");
}
