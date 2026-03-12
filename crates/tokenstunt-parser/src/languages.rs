use anyhow::{bail, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    Ruby,
    Swift,
    Kotlin,
    Dart,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Java => "java",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Ruby => "ruby",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Dart => "dart",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "mjs" | "cjs" => Some(Self::JavaScript),
            "jsx" => Some(Self::Tsx),
            "py" | "pyi" => Some(Self::Python),
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(Self::Cpp),
            "rb" => Some(Self::Ruby),
            "swift" => Some(Self::Swift),
            "kt" | "kts" => Some(Self::Kotlin),
            "dart" => Some(Self::Dart),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }
}

pub struct LanguageRegistry {
    ts_typescript: tree_sitter::Language,
    ts_tsx: tree_sitter::Language,
    ts_python: tree_sitter::Language,
    ts_rust: tree_sitter::Language,
    ts_go: tree_sitter::Language,
    ts_java: tree_sitter::Language,
}

impl LanguageRegistry {
    pub fn new() -> Result<Self> {
        Ok(Self {
            ts_typescript: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            ts_tsx: tree_sitter_typescript::LANGUAGE_TSX.into(),
            ts_python: tree_sitter_python::LANGUAGE.into(),
            ts_rust: tree_sitter_rust::LANGUAGE.into(),
            ts_go: tree_sitter_go::LANGUAGE.into(),
            ts_java: tree_sitter_java::LANGUAGE.into(),
        })
    }

    pub fn get_ts_language(&self, lang: Language) -> Result<tree_sitter::Language> {
        match lang {
            Language::TypeScript => Ok(self.ts_typescript.clone()),
            Language::Tsx | Language::JavaScript => Ok(self.ts_tsx.clone()),
            Language::Python => Ok(self.ts_python.clone()),
            Language::Rust => Ok(self.ts_rust.clone()),
            Language::Go => Ok(self.ts_go.clone()),
            Language::Java => Ok(self.ts_java.clone()),
            other => bail!("language {:?} not yet supported", other),
        }
    }

    pub fn is_supported(&self, lang: Language) -> bool {
        matches!(
            lang,
            Language::TypeScript
                | Language::Tsx
                | Language::JavaScript
                | Language::Python
                | Language::Rust
                | Language::Go
                | Language::Java
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_extension_all() {
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("mjs"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("cjs"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("pyi"), Some(Language::Python));
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("java"), Some(Language::Java));
        assert_eq!(Language::from_extension("c"), Some(Language::C));
        assert_eq!(Language::from_extension("h"), Some(Language::C));
        assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cc"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("cxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("hxx"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("rb"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("swift"), Some(Language::Swift));
        assert_eq!(Language::from_extension("kt"), Some(Language::Kotlin));
        assert_eq!(Language::from_extension("kts"), Some(Language::Kotlin));
        assert_eq!(Language::from_extension("dart"), Some(Language::Dart));
        assert_eq!(Language::from_extension("txt"), None);
        assert_eq!(Language::from_extension("md"), None);
    }

    #[test]
    fn test_from_path() {
        assert_eq!(
            Language::from_path(Path::new("src/main.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::from_path(Path::new("lib/utils.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("README.md")), None);
        assert_eq!(Language::from_path(Path::new("Makefile")), None);
    }

    #[test]
    fn test_as_str_roundtrip() {
        let all = [
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Rust,
            Language::Go,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Swift,
            Language::Kotlin,
            Language::Dart,
        ];

        for lang in &all {
            let s = lang.as_str();
            assert!(!s.is_empty(), "as_str returned empty for {lang:?}");
        }
    }

    #[test]
    fn test_get_ts_language() {
        let registry = LanguageRegistry::new().unwrap();
        assert!(registry.get_ts_language(Language::TypeScript).is_ok());
        assert!(registry.get_ts_language(Language::Tsx).is_ok());
        assert!(registry.get_ts_language(Language::JavaScript).is_ok());
        assert!(registry.get_ts_language(Language::Python).is_ok());
        assert!(registry.get_ts_language(Language::Rust).is_ok());
        assert!(registry.get_ts_language(Language::Go).is_ok());
        assert!(registry.get_ts_language(Language::Java).is_ok());
    }

    #[test]
    fn test_is_supported() {
        let registry = LanguageRegistry::new().unwrap();
        assert!(registry.is_supported(Language::TypeScript));
        assert!(registry.is_supported(Language::Tsx));
        assert!(registry.is_supported(Language::JavaScript));
        assert!(registry.is_supported(Language::Python));
        assert!(registry.is_supported(Language::Rust));
        assert!(registry.is_supported(Language::Go));
        assert!(registry.is_supported(Language::Java));
    }
}
