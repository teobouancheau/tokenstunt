use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;
use tokenstunt_parser::Language;
use tracing::warn;

pub struct FileEntry {
    pub path: PathBuf,
    pub language: Language,
}

/// Walk a directory tree and return all files with recognized language extensions.
pub fn walk_directory(root: &Path) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read directory entry");
                continue;
            }
        };

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path().to_path_buf();
        if let Some(language) = Language::from_path(&path) {
            entries.push(FileEntry { path, language });
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_walk_directory() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(src.join("app.ts"), "const x = 1;").unwrap();
        std::fs::write(src.join("utils.py"), "x = 1").unwrap();
        std::fs::write(src.join("readme.md"), "# Hello").unwrap();
        std::fs::write(src.join("data.json"), "{}").unwrap();

        let entries = walk_directory(dir.path()).unwrap();

        let extensions: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.path.extension().and_then(|ext| ext.to_str()))
            .collect();

        assert!(extensions.contains(&"ts"));
        assert!(extensions.contains(&"py"));
        assert!(!extensions.contains(&"md"));
        assert!(!extensions.contains(&"json"));
    }
}
