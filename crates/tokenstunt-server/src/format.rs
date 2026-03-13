use tokenstunt_store::CodeBlock;

use crate::render;

#[cfg(test)]
fn format_block(block: &CodeBlock, score: Option<f64>) -> String {
    let file_path = block.file_path.as_deref().unwrap_or("unknown");
    let language = block.language.as_deref().unwrap_or("text");
    let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);

    let mut out = format!(
        "  {}  {:<24} {location}",
        render::kind_label(&block.kind),
        block.name,
    );

    if let Some(s) = score {
        let bar = render::bar(s, 10);
        out.push_str(&format!("    {bar} {s:.2}"));
    }

    out.push_str("\n\n");
    out.push_str(&render::code_block(language, &block.content));
    out
}

pub fn format_summary_row(block: &CodeBlock, score: Option<f64>) -> String {
    let file_path = block.file_path.as_deref().unwrap_or("unknown");
    let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);

    let mut out = format!(
        "  {}  {:<24} {location}",
        render::kind_label(&block.kind),
        block.name,
    );

    if let Some(s) = score {
        let bar = render::bar(s, 10);
        out.push_str(&format!("    {bar} {s:.2}"));
    }

    out
}

pub fn format_blocks(blocks: &[(CodeBlock, Option<f64>)]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let query_hint = "";
    let count = blocks.len();
    let mut out = render::header("Search", query_hint);
    out.push_str(&format!("                                   {count} results\n\n"));

    for (block, score) in blocks {
        out.push_str(&format_summary_row(block, *score));
        out.push('\n');
    }

    out.push('\n');
    out.push_str(&render::separator());
    out.push('\n');

    for (block, _score) in blocks {
        let file_path = block.file_path.as_deref().unwrap_or("unknown");
        let language = block.language.as_deref().unwrap_or("text");
        let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);

        out.push_str(&format!(
            "\n  {}  {:<24} {location}\n\n",
            render::kind_label(&block.kind),
            block.name,
        ));
        out.push_str(&render::code_block(language, &block.content));
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
        let file_path = block.file_path.as_deref().unwrap_or("unknown");
        let language = block.language.as_deref().unwrap_or("text");
        let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);

        out.push_str(&format!(
            "  {}  {:<24} {location}\n\n",
            render::kind_label(&block.kind),
            block.name,
        ));
        out.push_str(&render::code_block(language, &block.content));
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
    fn test_format_block() {
        let block = sample_block("greet");
        let output = format_block(&block, Some(0.95));

        assert!(output.contains("greet"));
        assert!(output.contains("Function"));
        assert!(output.contains("src/main.ts"));
        assert!(output.contains("1-5"));
        assert!(output.contains("0.95"));
        assert!(output.contains("```typescript"));

        let output_no_score = format_block(&block, None);
        assert!(!output_no_score.contains("0.95"));
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
        assert!(output.contains("\u{2500}"));
        assert!(output.contains("2 results"));
    }

    #[test]
    fn test_format_blocks_has_summary_and_code() {
        let blocks = vec![(sample_block("greet"), Some(0.95))];
        let output = format_blocks(&blocks);
        assert!(output.contains("\u{25C6} Search"));
        assert!(output.contains("```typescript"));
    }
}
