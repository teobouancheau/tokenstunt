use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Returns the cache directory for a project's index database.
///
/// Path: `~/.cache/tokenstunt/<project-name>-<hash>/index.db`
///
/// The hash is derived from the canonical root path to avoid collisions
/// between projects with the same directory name.
pub fn cache_db_path(root: &Path) -> Result<PathBuf> {
    let project_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let hash = path_hash(root);
    let dir_name = format!("{project_name}-{hash}");

    let cache_dir = cache_root()?.join(dir_name);
    Ok(cache_dir.join("index.db"))
}

fn cache_root() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".cache").join("tokenstunt"))
}

fn path_hash(path: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:012x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_db_path_uses_project_name_and_hash() {
        let path = cache_db_path(Path::new("/Users/dev/myproject")).unwrap();
        let dir = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();

        assert!(dir.starts_with("myproject-"));
        assert!(path.ends_with("index.db"));
        assert!(path.to_str().unwrap().contains(".cache/tokenstunt/"));
    }

    #[test]
    fn different_roots_produce_different_hashes() {
        let a = cache_db_path(Path::new("/a/myproject")).unwrap();
        let b = cache_db_path(Path::new("/b/myproject")).unwrap();

        assert_ne!(a, b);
    }

    #[test]
    fn same_root_produces_same_path() {
        let a = cache_db_path(Path::new("/Users/dev/project")).unwrap();
        let b = cache_db_path(Path::new("/Users/dev/project")).unwrap();

        assert_eq!(a, b);
    }
}
