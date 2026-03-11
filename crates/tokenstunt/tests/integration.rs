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
    let indexer = Indexer::new(store).unwrap();
    let stats = indexer.index_directory(dir.path()).unwrap();

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
