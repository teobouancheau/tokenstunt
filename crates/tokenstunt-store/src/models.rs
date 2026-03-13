use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeBlockKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    TypeAlias,
    Constant,
    Variable,
    Module,
    Trait,
    Impl,
}

impl CodeBlockKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::TypeAlias => "type_alias",
            Self::Constant => "constant",
            Self::Variable => "variable",
            Self::Module => "module",
            Self::Trait => "trait",
            Self::Impl => "impl",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

impl std::str::FromStr for CodeBlockKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            "class" => Ok(Self::Class),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "interface" => Ok(Self::Interface),
            "type_alias" => Ok(Self::TypeAlias),
            "constant" => Ok(Self::Constant),
            "variable" => Ok(Self::Variable),
            "module" => Ok(Self::Module),
            "trait" => Ok(Self::Trait),
            "impl" => Ok(Self::Impl),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for CodeBlockKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: CodeBlockKind,
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    pub signature: String,
    pub docstring: String,
    pub parent_id: Option<i64>,
    pub file_path: Option<String>,
    pub language: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_block_kind_all_variants() {
        let variants = [
            (CodeBlockKind::Function, "function"),
            (CodeBlockKind::Method, "method"),
            (CodeBlockKind::Class, "class"),
            (CodeBlockKind::Struct, "struct"),
            (CodeBlockKind::Enum, "enum"),
            (CodeBlockKind::Interface, "interface"),
            (CodeBlockKind::TypeAlias, "type_alias"),
            (CodeBlockKind::Constant, "constant"),
            (CodeBlockKind::Variable, "variable"),
            (CodeBlockKind::Module, "module"),
            (CodeBlockKind::Trait, "trait"),
            (CodeBlockKind::Impl, "impl"),
        ];

        for (kind, expected_str) in &variants {
            assert_eq!(kind.as_str(), *expected_str);
            assert_eq!(CodeBlockKind::from_str(expected_str), Some(*kind));
            assert_eq!(format!("{kind}"), *expected_str);
        }

        assert_eq!(CodeBlockKind::from_str("unknown"), None);
    }
}
