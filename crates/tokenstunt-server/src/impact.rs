use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use tokenstunt_store::Store;

const MAX_DEPTH_CAP: u32 = 5;

pub struct ImpactNode {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub dep_kind: String,
    pub depth: u32,
}

pub struct ImpactResult {
    pub source: String,
    pub dependents: Vec<ImpactNode>,
    pub affected_files: Vec<String>,
}

pub fn walk_dependents(
    store: &Store,
    source: &str,
    max_depth: Option<u32>,
) -> Result<ImpactResult> {
    let max_depth = max_depth.unwrap_or(3).min(MAX_DEPTH_CAP);

    let symbols = store.lookup_symbol(source, None)?;
    if symbols.is_empty() {
        return Ok(ImpactResult {
            source: source.to_string(),
            dependents: Vec::new(),
            affected_files: Vec::new(),
        });
    }

    let mut visited: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<(i64, u32)> = VecDeque::new();
    let mut dependents: Vec<ImpactNode> = Vec::new();
    let mut affected_files: HashSet<String> = HashSet::new();

    for symbol in &symbols {
        visited.insert(symbol.id);
        queue.push_back((symbol.id, 0));
    }

    while let Some((block_id, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let deps = store.get_dependents(block_id)?;
        for (block, dep_kind) in deps {
            if visited.contains(&block.id) {
                continue;
            }
            visited.insert(block.id);

            let file_path = block.file_path.clone().unwrap_or_default();
            affected_files.insert(file_path.clone());

            dependents.push(ImpactNode {
                name: block.name.clone(),
                kind: block.kind.to_string(),
                file_path,
                dep_kind,
                depth: depth + 1,
            });

            queue.push_back((block.id, depth + 1));
        }
    }

    let mut affected: Vec<String> = affected_files.into_iter().collect();
    affected.sort();

    Ok(ImpactResult {
        source: source.to_string(),
        dependents,
        affected_files: affected,
    })
}

pub fn format_impact(result: &ImpactResult) -> String {
    if result.dependents.is_empty() {
        return format!(
            "## Impact Analysis: `{}`\n\nNo dependents found. This symbol can be safely modified.",
            result.source
        );
    }

    let mut out = format!(
        "## Impact Analysis: `{}`\n\n**{} dependents** across **{} files**\n",
        result.source,
        result.dependents.len(),
        result.affected_files.len()
    );

    let max_depth = result.dependents.iter().map(|d| d.depth).max().unwrap_or(0);

    for depth in 1..=max_depth {
        let label = if depth == 1 { "Direct" } else { "Transitive" };
        let nodes: Vec<&ImpactNode> = result
            .dependents
            .iter()
            .filter(|d| d.depth == depth)
            .collect();
        if nodes.is_empty() {
            continue;
        }

        out.push_str(&format!("\n### {label} (depth {depth})\n\n"));
        for node in &nodes {
            out.push_str(&format!(
                "- **{}** ({}) in `{}` [{}]\n",
                node.name, node.kind, node.file_path, node.dep_kind
            ));
        }
    }

    if !result.affected_files.is_empty() {
        out.push_str("\n### Affected Files\n\n");
        for path in &result.affected_files {
            out.push_str(&format!("- `{path}`\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenstunt_store::{CodeBlockKind, Store};

    struct TestFixture {
        store: Store,
        block_a: i64,
        block_b: i64,
        block_c: i64,
    }

    fn setup() -> TestFixture {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/core.ts", 111, "typescript", 0)
            .unwrap();

        let block_a = store
            .insert_code_block(
                file_id,
                "funcA",
                CodeBlockKind::Function,
                1,
                5,
                "function funcA() {}",
                "function funcA()",
                None,
            )
            .unwrap();
        let block_b = store
            .insert_code_block(
                file_id,
                "funcB",
                CodeBlockKind::Function,
                10,
                15,
                "function funcB() { funcA(); }",
                "function funcB()",
                None,
            )
            .unwrap();
        let file_id2 = store
            .upsert_file(repo_id, "src/util.ts", 222, "typescript", 0)
            .unwrap();
        let block_c = store
            .insert_code_block(
                file_id2,
                "funcC",
                CodeBlockKind::Function,
                1,
                5,
                "function funcC() { funcB(); }",
                "function funcC()",
                None,
            )
            .unwrap();

        // B depends on A, C depends on B
        store
            .insert_dependency(block_b, Some(block_a), "funcA", "call")
            .unwrap();
        store
            .insert_dependency(block_c, Some(block_b), "funcB", "call")
            .unwrap();

        TestFixture {
            store,
            block_a,
            block_b,
            block_c,
        }
    }

    #[test]
    fn test_no_dependents() {
        let f = setup();
        let result = walk_dependents(&f.store, "funcC", None).unwrap();
        assert!(result.dependents.is_empty());
        let _ = f.block_c; // used by setup
    }

    #[test]
    fn test_direct() {
        let f = setup();
        let result = walk_dependents(&f.store, "funcA", Some(1)).unwrap();
        assert_eq!(result.dependents.len(), 1);
        assert_eq!(result.dependents[0].name, "funcB");
        assert_eq!(result.dependents[0].depth, 1);
        let _ = (f.block_a, f.block_b);
    }

    #[test]
    fn test_transitive() {
        let f = setup();
        let result = walk_dependents(&f.store, "funcA", None).unwrap();
        assert_eq!(result.dependents.len(), 2);
        assert!(
            result
                .dependents
                .iter()
                .any(|d| d.name == "funcB" && d.depth == 1)
        );
        assert!(
            result
                .dependents
                .iter()
                .any(|d| d.name == "funcC" && d.depth == 2)
        );
        assert_eq!(result.affected_files.len(), 2);
    }

    #[test]
    fn test_cycle_detection() {
        let store = Store::open_in_memory().unwrap();
        let repo_id = store.ensure_repo("/test", "test").unwrap();
        let file_id = store
            .upsert_file(repo_id, "src/cycle.ts", 111, "typescript", 0)
            .unwrap();

        let a = store
            .insert_code_block(
                file_id,
                "cycleA",
                CodeBlockKind::Function,
                1,
                5,
                "fn a() {}",
                "fn a()",
                None,
            )
            .unwrap();
        let b = store
            .insert_code_block(
                file_id,
                "cycleB",
                CodeBlockKind::Function,
                10,
                15,
                "fn b() {}",
                "fn b()",
                None,
            )
            .unwrap();

        // A -> B -> A (cycle)
        store
            .insert_dependency(b, Some(a), "cycleA", "call")
            .unwrap();
        store
            .insert_dependency(a, Some(b), "cycleB", "call")
            .unwrap();

        let result = walk_dependents(&store, "cycleA", None).unwrap();
        // Should not loop forever; cycleB depends on cycleA, cycleA depends on cycleB
        assert!(result.dependents.len() <= 2);
    }

    #[test]
    fn test_max_depth() {
        let f = setup();
        let result = walk_dependents(&f.store, "funcA", Some(1)).unwrap();
        assert_eq!(result.dependents.len(), 1);
        assert!(!result.dependents.iter().any(|d| d.name == "funcC"));
    }

    #[test]
    fn test_format_empty() {
        let result = ImpactResult {
            source: "test".to_string(),
            dependents: Vec::new(),
            affected_files: Vec::new(),
        };
        let output = format_impact(&result);
        assert!(output.contains("No dependents found"));
        assert!(output.contains("safely modified"));
    }

    #[test]
    fn test_format_grouped() {
        let result = ImpactResult {
            source: "funcA".to_string(),
            dependents: vec![
                ImpactNode {
                    name: "funcB".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/core.ts".to_string(),
                    dep_kind: "call".to_string(),
                    depth: 1,
                },
                ImpactNode {
                    name: "funcC".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/util.ts".to_string(),
                    dep_kind: "call".to_string(),
                    depth: 2,
                },
            ],
            affected_files: vec!["src/core.ts".to_string(), "src/util.ts".to_string()],
        };
        let output = format_impact(&result);
        assert!(output.contains("Direct (depth 1)"));
        assert!(output.contains("Transitive (depth 2)"));
        assert!(output.contains("funcB"));
        assert!(output.contains("funcC"));
        assert!(output.contains("Affected Files"));
        assert!(output.contains("2 dependents"));
        assert!(output.contains("2 files"));
    }
}
