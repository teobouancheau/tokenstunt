use tokenstunt_store::CodeBlock;

pub fn format_block(block: &CodeBlock, score: Option<f64>) -> String {
    let file_path = block.file_path.as_deref().unwrap_or("unknown");
    let language = block.language.as_deref().unwrap_or("text");
    let score_str = score.map(|s| format!(" [{:.2}]", s)).unwrap_or_default();

    format!(
        "## {} ({}) -- {}:{}-{}{}\n\n```{}\n{}\n```",
        block.name,
        block.kind,
        file_path,
        block.start_line,
        block.end_line,
        score_str,
        language,
        block.content,
    )
}

pub fn format_blocks(blocks: &[(CodeBlock, Option<f64>)]) -> String {
    blocks
        .iter()
        .map(|(block, score)| format_block(block, *score))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenstunt_store::CodeBlockKind;

    fn sample_block(name: &str) -> CodeBlock {
        CodeBlock {
            id: 1,
            file_id: 1,
            name: name.to_string(),
            kind: CodeBlockKind::Function,
            start_line: 1,
            end_line: 5,
            content: "function greet() {}".to_string(),
            signature: "function greet()".to_string(),
            parent_id: None,
            file_path: Some("src/main.ts".to_string()),
            language: Some("typescript".to_string()),
        }
    }

    #[test]
    fn test_format_block() {
        let block = sample_block("greet");
        let output = format_block(&block, Some(0.95));

        assert!(output.contains("greet"));
        assert!(output.contains("function"));
        assert!(output.contains("src/main.ts"));
        assert!(output.contains("1-5"));
        assert!(output.contains("[0.95]"));
        assert!(output.contains("```typescript"));

        let output_no_score = format_block(&block, None);
        assert!(!output_no_score.contains("["));
    }

    #[test]
    fn test_format_block_missing_fields() {
        let block = CodeBlock {
            id: 1,
            file_id: 1,
            name: "orphan".to_string(),
            kind: CodeBlockKind::Function,
            start_line: 1,
            end_line: 3,
            content: "fn orphan() {}".to_string(),
            signature: "fn orphan()".to_string(),
            parent_id: None,
            file_path: None,
            language: None,
        };
        let output = format_block(&block, None);
        assert!(
            output.contains("unknown"),
            "missing file_path should show 'unknown'"
        );
        assert!(
            output.contains("```text"),
            "missing language should fallback to 'text'"
        );
    }

    #[test]
    fn test_format_blocks_separator() {
        let blocks = vec![
            (sample_block("a"), Some(0.9)),
            (sample_block("b"), Some(0.8)),
        ];
        let output = format_blocks(&blocks);
        assert!(output.contains("---"));
        let parts: Vec<&str> = output.split("---").collect();
        assert_eq!(parts.len(), 2);
    }
}
