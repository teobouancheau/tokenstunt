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

    #[cfg(unix)]
    #[test]
    fn test_walk_directory_unreadable_subdir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.ts"), "const x = 1;").unwrap();

        // Create a subdirectory and make it unreadable so the walker
        // produces an error for its entries (covers lines 27-29)
        let bad = dir.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("secret.ts"), "const s = 1;").unwrap();
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();

        let entries = walk_directory(dir.path()).unwrap();

        // Restore permissions for cleanup
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The readable file should still be found
        let has_app = entries.iter().any(|e| {
            e.path
                .file_name()
                .is_some_and(|n| n.to_str() == Some("app.ts"))
        });
        assert!(has_app, "readable files should still be returned");
    }
}
