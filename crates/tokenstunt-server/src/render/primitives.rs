use std::fmt::Write;

use tokenstunt_store::CodeBlockKind;

pub fn header(title: &str, subtitle: &str) -> String {
    if subtitle.is_empty() {
        format!("\u{25C6} {title}")
    } else {
        format!("\u{25C6} {title}  {subtitle}")
    }
}

pub fn separator() -> String {
    "\u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500} \u{2500}".to_string()
}

pub fn notice(message: &str) -> String {
    format!("\u{2591}\u{2591}\u{2591} {message}")
}

pub fn kv(label: &str, value: &str, width: usize) -> String {
    format!("{label:>width$}  {value}")
}

pub fn kind_label(kind: &CodeBlockKind) -> &'static str {
    match kind {
        CodeBlockKind::Function => "Function ",
        CodeBlockKind::Method => "Method   ",
        CodeBlockKind::Class => "Class    ",
        CodeBlockKind::Struct => "Struct   ",
        CodeBlockKind::Enum => "Enum     ",
        CodeBlockKind::Interface => "Interface",
        CodeBlockKind::TypeAlias => "Type     ",
        CodeBlockKind::Constant => "Constant ",
        CodeBlockKind::Variable => "Variable ",
        CodeBlockKind::Module => "Module   ",
        CodeBlockKind::Trait => "Trait    ",
        CodeBlockKind::Impl => "Impl     ",
    }
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

pub fn code_block(language: &str, content: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "```{language}");
    let _ = write!(out, "{content}");
    if !content.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header() {
        assert_eq!(header("Search", "\"auth\""), "\u{25C6} Search  \"auth\"");
        assert_eq!(header("Setup", ""), "\u{25C6} Setup");
    }

    #[test]
    fn test_kv() {
        let line = kv("Root", "/test", 10);
        assert!(line.contains("Root"));
        assert!(line.contains("/test"));
    }

    #[test]
    fn test_kind_label() {
        assert_eq!(kind_label(&CodeBlockKind::Function).trim(), "Function");
        assert_eq!(kind_label(&CodeBlockKind::Interface).trim(), "Interface");
    }

    #[test]
    fn test_notice() {
        assert!(notice("Warning").contains("\u{2591}"));
    }

    #[test]
    fn test_code_block() {
        let cb = code_block("typescript", "const x = 1;");
        assert!(cb.starts_with("```typescript"));
        assert!(cb.ends_with("```"));
    }
}
