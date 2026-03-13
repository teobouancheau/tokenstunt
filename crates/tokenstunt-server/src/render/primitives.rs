use std::fmt::Write;

use tokenstunt_store::CodeBlockKind;

pub const LABEL_WIDTH: usize = 12;
pub const KIND_WIDTH: usize = 9;

pub fn header(title: &str, subtitle: &str) -> String {
    if subtitle.is_empty() {
        format!("\u{25C6} {title}")
    } else {
        format!("\u{25C6} {title}  {subtitle}")
    }
}

pub fn notice(message: &str) -> String {
    format!("\u{25C6} {message}")
}

pub fn kv(label: &str, value: &str, width: usize) -> String {
    format!("{label:>width$}  {value}")
}

pub fn kv_line(label: &str, value: &str) -> String {
    kv(label, value, LABEL_WIDTH)
}

pub fn kind_label(kind: &CodeBlockKind) -> String {
    let name = match kind {
        CodeBlockKind::Function => "Function",
        CodeBlockKind::Method => "Method",
        CodeBlockKind::Class => "Class",
        CodeBlockKind::Struct => "Struct",
        CodeBlockKind::Enum => "Enum",
        CodeBlockKind::Interface => "Interface",
        CodeBlockKind::TypeAlias => "Type",
        CodeBlockKind::Constant => "Constant",
        CodeBlockKind::Variable => "Variable",
        CodeBlockKind::Module => "Module",
        CodeBlockKind::Trait => "Trait",
        CodeBlockKind::Impl => "Impl",
    };
    format!("{name:<KIND_WIDTH$}")
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
        let func = kind_label(&CodeBlockKind::Function);
        assert_eq!(func.trim(), "Function");
        assert_eq!(func.len(), KIND_WIDTH);
        let iface = kind_label(&CodeBlockKind::Interface);
        assert_eq!(iface.trim(), "Interface");
        assert_eq!(iface.len(), KIND_WIDTH);
    }

    #[test]
    fn test_notice() {
        assert!(notice("Warning").contains("\u{25C6}"));
    }

    #[test]
    fn test_code_block() {
        let cb = code_block("typescript", "const x = 1;");
        assert!(cb.starts_with("```typescript"));
        assert!(cb.ends_with("```"));
    }
}
