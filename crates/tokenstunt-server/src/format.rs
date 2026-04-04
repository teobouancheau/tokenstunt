use tokenstunt_search::SearchSource;
use tokenstunt_store::CodeBlock;

use crate::render;

pub struct SearchBlock {
    pub block: CodeBlock,
    pub source: Option<SearchSource>,
}

fn format_block_entry(block: &CodeBlock, source: Option<SearchSource>) -> (String, String, String) {
    let file_path = block.file_path.as_deref().unwrap_or("unknown");
    let language = block.language.as_deref().unwrap_or("text");
    let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);
    let source_tag = match source {
        Some(SearchSource::Bm25) => "  [keyword]",
        Some(SearchSource::Semantic) => "  [semantic]",
        Some(SearchSource::Hybrid) => "  [hybrid]",
        None => "",
    };
    (
        format!(
            "  {}  {:<24} {location}{source_tag}",
            render::kind_label(&block.kind),
            block.name,
        ),
        language.to_string(),
        block.content.clone(),
    )
}

pub fn format_blocks(query: &str, blocks: &[SearchBlock]) -> String {
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

    for sb in blocks {
        let (header_line, language, content) = format_block_entry(&sb.block, sb.source);
        out.push_str(&format!("\n{header_line}\n\n"));
        out.push_str(&render::code_block(&language, &content));
        out.push('\n');
    }

    out
}

pub fn format_file_blocks(path: &str, blocks: &[CodeBlock]) -> String {
    if blocks.is_empty() {
        return format!("No symbols found in '{path}'.");
    }

    let mut out = render::header("File", &format!("{path}  {} symbols", blocks.len()));
    out.push('\n');

    for block in blocks {
        let language = block.language.as_deref().unwrap_or("text");
        out.push_str(&format!(
            "\n  {}  {:<24} lines {}-{}\n",
            render::kind_label(&block.kind),
            block.name,
            block.start_line,
            block.end_line,
        ));
        if !block.signature.is_empty() {
            out.push_str(&format!("  {}\n", block.signature));
        }
        out.push('\n');
        out.push_str(&render::code_block(language, &block.content));
        out.push('\n');
    }

    out
}

pub fn format_usages(symbol: &str, usages: &[(CodeBlock, String)]) -> String {
    if usages.is_empty() {
        return format!("No usages found for '{symbol}'.");
    }

    let mut out = render::header("Usages", &format!("{symbol}  {} call sites", usages.len()));
    out.push('\n');

    for (block, dep_kind) in usages {
        let file_path = block.file_path.as_deref().unwrap_or("unknown");
        let language = block.language.as_deref().unwrap_or("text");
        let location = format!("{file_path}:{}-{}", block.start_line, block.end_line);
        out.push_str(&format!(
            "\n  {}  {:<24} {:<28} {}\n\n",
            render::kind_label(&block.kind),
            block.name,
            location,
            render::capitalize(dep_kind),
        ));
        out.push_str(&render::code_block(language, &block.content));
        out.push('\n');
    }

    out
}

pub fn format_symbol_blocks(blocks: &[CodeBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (header_line, language, content) = format_block_entry(block, None);
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
            docstring: String::new(),
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
            docstring: String::new(),
            parent_id: None,
            file_path: None,
            language: None,
        };
        let (header_line, language, _content) = format_block_entry(&block, None);
        assert!(
            header_line.contains("unknown"),
            "missing file_path should show 'unknown'"
        );
        assert_eq!(
            language, "text",
            "missing language should fallback to 'text'"
        );
    }

    fn search_block(name: &str, source: Option<SearchSource>) -> SearchBlock {
        SearchBlock {
            block: sample_block(name),
            source,
        }
    }

    #[test]
    fn test_format_blocks_inline_layout() {
        let blocks = vec![
            search_block("a", Some(SearchSource::Bm25)),
            search_block("b", Some(SearchSource::Semantic)),
        ];
        let output = format_blocks("authenticate", &blocks);
        assert!(output.contains("2 results"));
        assert!(output.contains("\"authenticate\""));
        assert!(output.contains("```typescript"));
        assert!(output.contains("[keyword]"));
        assert!(output.contains("[semantic]"));
        assert!(!output.contains("\u{2500}"), "should not contain separator");
    }

    #[test]
    fn test_format_blocks_empty() {
        let output = format_blocks("test", &[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_format_blocks_empty_query() {
        let blocks = vec![search_block("greet", None)];
        let output = format_blocks("", &blocks);
        assert!(output.contains("1 results"));
        assert!(
            !output.contains("\"\""),
            "should not show empty quoted query"
        );
    }

    #[test]
    fn test_format_file_blocks_empty() {
        let output = format_file_blocks("src/empty.ts", &[]);
        assert!(output.contains("No symbols found"));
    }

    #[test]
    fn test_format_file_blocks_with_symbols() {
        let blocks = vec![sample_block("greet")];
        let output = format_file_blocks("src/main.ts", &blocks);
        assert!(output.contains("File"));
        assert!(output.contains("1 symbols"));
        assert!(output.contains("greet"));
        assert!(output.contains("lines 1-5"));
    }

    #[test]
    fn test_format_usages_empty() {
        let output = format_usages("myFunc", &[]);
        assert!(output.contains("No usages found"));
    }

    #[test]
    fn test_format_usages_with_results() {
        let usages = vec![(sample_block("caller"), "call".to_string())];
        let output = format_usages("myFunc", &usages);
        assert!(output.contains("Usages"));
        assert!(output.contains("1 call sites"));
        assert!(output.contains("caller"));
        assert!(output.contains("Call"));
    }

    #[test]
    fn test_format_symbol_blocks_empty() {
        let output = format_symbol_blocks(&[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_format_symbol_blocks_single() {
        let blocks = vec![sample_block("greet")];
        let output = format_symbol_blocks(&blocks);
        assert!(output.contains("greet"));
        assert!(output.contains("```typescript"));
    }

    #[test]
    fn test_format_symbol_blocks_multiple() {
        let blocks = vec![sample_block("greet"), sample_block("farewell")];
        let output = format_symbol_blocks(&blocks);
        assert!(output.contains("greet"));
        assert!(output.contains("farewell"));
    }

    #[test]
    fn test_format_blocks_has_header_and_code() {
        let blocks = vec![search_block("greet", Some(SearchSource::Hybrid))];
        let output = format_blocks("greet", &blocks);
        assert!(output.contains("\u{25C6} Search"));
        assert!(output.contains("```typescript"));
        assert!(output.contains("[hybrid]"));
        assert!(!output.contains("0.9"), "should not contain score");
    }
}
