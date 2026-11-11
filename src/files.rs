//! Finding the `.pathrc` files that apply to a directory.

use std::fs;
use std::path::{Path, PathBuf};

/// The configuration file name, the same for every shell.
pub const PLAIN: &str = ".pathrc";

/// A configuration file in scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: PathBuf,
}

/// The `.pathrc` files from `start` (or the filesystem root, when `cwd` is
/// not inside `start`) down to `cwd`, outermost first.
pub fn find(cwd: &Path, start: Option<&Path>) -> Vec<ConfigFile> {
    let start = start.and_then(|start| fs::canonicalize(start).ok());
    let mut found = Vec::new();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let candidate = current.join(PLAIN);
        if candidate.is_file() {
            found.push(ConfigFile { path: candidate });
        }
        let reached_start = start
            .as_deref()
            .is_some_and(|start| fs::canonicalize(current).is_ok_and(|c| c == start));
        if reached_start {
            break;
        }
        dir = current.parent();
    }
    found.reverse();
    found
}

/// `PATHRC_START`, the shell's starting directory, if set.
pub fn start_dir() -> Option<PathBuf> {
    std::env::var_os("PATHRC_START")
        .filter(|start| !start.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_outermost_first_and_stops_at_start() {
        let root = tempfile::tempdir().unwrap();
        let start = root.path().join("start");
        let inner = start.join("a").join("b");
        fs::create_dir_all(&inner).unwrap();
        for dir in [root.path(), start.as_path(), &start.join("a"), &inner] {
            fs::write(dir.join(PLAIN), "").unwrap();
        }
        let found: Vec<_> = find(&inner, Some(&start))
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(
            found,
            vec![
                start.join(PLAIN),
                start.join("a").join(PLAIN),
                inner.join(PLAIN)
            ]
        );
    }

    #[test]
    fn ignores_the_old_per_shell_files() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".pathrc-bash"), "").unwrap();
        assert!(find(root.path(), Some(root.path())).is_empty());
    }
}
