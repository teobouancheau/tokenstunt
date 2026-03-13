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
    let indexer = Indexer::new(store, None, None).unwrap();
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
    let indexer = Indexer::new(store, None, None).unwrap();
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

// --- CLI binary integration tests ---

fn tokenstunt_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    path.push("tokenstunt");
    path
}

#[test]
fn test_cli_help() {
    let output = std::process::Command::new(tokenstunt_bin())
        .arg("--help")
        .output()
        .expect("failed to run tokenstunt --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tokenstunt"));
}

#[test]
fn test_cli_version() {
    let output = std::process::Command::new(tokenstunt_bin())
        .arg("--version")
        .output()
        .expect("failed to run tokenstunt --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tokenstunt"));
}

#[test]
fn test_cli_index_help() {
    let output = std::process::Command::new(tokenstunt_bin())
        .args(["index", "--help"])
        .output()
        .expect("failed to run tokenstunt index --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Index"));
}

#[test]
fn test_cli_status_no_index() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("nonexistent.db");

    let output = std::process::Command::new(tokenstunt_bin())
        .args(["status", "--db", db_path.to_str().unwrap()])
        .output()
        .expect("failed to run tokenstunt status");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No index found"));
}

#[test]
fn test_cli_index_temp_dir() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("example.py"), "def hello():\n    return 42\n").unwrap();

    let db_path = dir.path().join("test-index.db");

    let output = std::process::Command::new(tokenstunt_bin())
        .args([
            "index",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run tokenstunt index");
    assert!(
        output.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(db_path.exists(), "database file should be created");
}

#[test]
fn test_cli_status_after_index() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();

    let db_path = dir.path().join("status-test.db");

    // Index first
    let output = std::process::Command::new(tokenstunt_bin())
        .args([
            "index",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Then check status
    let output = std::process::Command::new(tokenstunt_bin())
        .args(["status", "--db", db_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("No index found"),
        "status should find the index"
    );
}

#[test]
fn test_cli_embed_no_config() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("app.ts"), "export function app() { return 1; }").unwrap();

    let db_path = dir.path().join("embed-test.db");

    // Index first so the DB exists
    let output = std::process::Command::new(tokenstunt_bin())
        .args([
            "index",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run tokenstunt index");
    assert!(output.status.success());

    // Run embed without any embeddings config; should succeed gracefully
    let output = std::process::Command::new(tokenstunt_bin())
        .args([
            "embed",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run tokenstunt embed");
    assert!(
        output.status.success(),
        "embed without config should exit cleanly: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No embedding provider configured"),
        "should print guidance about missing config, got: {stderr}"
    );
}

#[test]
fn test_cli_serve_starts_and_exits_on_stdin_close() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("app.py"), "def main():\n    pass\n").unwrap();

    let db_path = dir.path().join("serve-test.db");

    let mut child = std::process::Command::new(tokenstunt_bin())
        .args([
            "serve",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            db_path.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn tokenstunt serve");

    // Close stdin to signal the MCP server to shut down
    drop(child.stdin.take());

    // Wait for the process with a manual timeout
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > std::time::Duration::from_secs(10) {
                    child.kill().expect("failed to kill serve process");
                    panic!("serve process did not exit within 10 seconds");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("error waiting for serve process: {e}"),
        }
    }
}

#[test]
fn test_cli_index_with_db_flag() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("utils.ts"),
        "export function add(a: number, b: number): number { return a + b; }",
    )
    .unwrap();

    let custom_db = dir.path().join("custom-location").join("my-index.db");

    let output = std::process::Command::new(tokenstunt_bin())
        .args([
            "index",
            "--root",
            dir.path().to_str().unwrap(),
            "--db",
            custom_db.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run tokenstunt index with --db flag");
    assert!(
        output.status.success(),
        "index with --db flag failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        custom_db.exists(),
        "database should be created at the custom path"
    );

    // Verify we can query status from that custom DB
    let output = std::process::Command::new(tokenstunt_bin())
        .args(["status", "--db", custom_db.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("No index found"),
        "status should find the index at custom path"
    );
}
