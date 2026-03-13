use tokenstunt_store::CodeBlock;

use crate::render;

fn format_block_entry(block: &CodeBlock) -> (String, String, String) {
    let file_path = block.file_path.as_deref().unwrap_or("unknown");
    let language = block.language.as_deref().unwrap_or("text");
    let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);
    (
        format!(
            "  {}  {:<24} {location}",
            render::kind_label(&block.kind),
            block.name,
        ),
        language.to_string(),
        block.content.clone(),
    )
}

pub fn format_blocks(query: &str, blocks: &[(CodeBlock, Option<f64>)]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let count = blocks.len();
    let hint = if query.is_empty() {
        format!("{count} results")
    } else {
        format!("\"{}\"  {} results", query, count)
    };
    let mut out = render::header("Search", &hint);
    out.push('\n');

    for (block, _score) in blocks {
        let (header_line, language, content) = format_block_entry(block);
        out.push_str(&format!("\n{header_line}\n\n"));
        out.push_str(&render::code_block(&language, &content));
        out.push('\n');
    }

    out
}

pub fn format_symbol_blocks(blocks: &[(CodeBlock, Option<f64>)]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (i, (block, _score)) in blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (header_line, language, content) = format_block_entry(block);
        out.push_str(&format!("{header_line}\n\n"));
        out.push_str(&render::code_block(&language, &content));
        out.push('\n');
    }

    out
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
    fn test_format_block_entry_missing_fields() {
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
        let (header_line, language, _content) = format_block_entry(&block);
        assert!(
            header_line.contains("unknown"),
            "missing file_path should show 'unknown'"
        );
        assert_eq!(
            language, "text",
            "missing language should fallback to 'text'"
        );
    }

    #[test]
    fn test_format_blocks_inline_layout() {
        let blocks = vec![
            (sample_block("a"), Some(0.9)),
            (sample_block("b"), Some(0.8)),
        ];
        let output = format_blocks("authenticate", &blocks);
        assert!(output.contains("2 results"));
        assert!(output.contains("\"authenticate\""));
        assert!(output.contains("```typescript"));
        assert!(!output.contains("\u{2500}"), "should not contain separator");
    }

    #[test]
    fn test_format_blocks_has_header_and_code() {
        let blocks = vec![(sample_block("greet"), Some(0.95))];
        let output = format_blocks("greet", &blocks);
        assert!(output.contains("\u{25C6} Search"));
        assert!(output.contains("```typescript"));
        assert!(!output.contains("0.95"), "should not contain score");
    }
}
