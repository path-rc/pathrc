//! Finding the `.pathrc` files that apply to a directory.

use std::fs;
use std::path::{Path, PathBuf};

/// The configuration file name, the same for every shell.
pub const PLAIN: &str = ".pathrc";
/// The same, encrypted with age. Read after `.pathrc` in the same
/// directory, so a secret can override a default.
pub const ENCRYPTED: &str = ".pathrc.age";

/// A configuration file in scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub encrypted: bool,
}

/// The configuration files in one directory, in reading order.
pub fn in_dir(dir: &Path) -> Vec<ConfigFile> {
    [(PLAIN, false), (ENCRYPTED, true)]
        .into_iter()
        .map(|(name, encrypted)| ConfigFile {
            path: dir.join(name),
            encrypted,
        })
        .filter(|file| file.path.is_file())
        .collect()
}

/// The configuration files from `start` (or the filesystem root, when `cwd`
/// is not inside `start`) down to `cwd`: outermost directory first, and
/// within a directory `.pathrc` before `.pathrc.age`.
pub fn find(cwd: &Path, start: Option<&Path>) -> Vec<ConfigFile> {
    let start = start.and_then(|start| fs::canonicalize(start).ok());
    let mut dirs = Vec::new();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        dirs.push(current);
        let reached_start = start
            .as_deref()
            .is_some_and(|start| fs::canonicalize(current).is_ok_and(|c| c == start));
        if reached_start {
            break;
        }
        dir = current.parent();
    }
    dirs.into_iter().rev().flat_map(in_dir).collect()
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
    fn plain_comes_before_encrypted_in_a_directory() {
        let root = tempfile::tempdir().unwrap();
        let inner = root.path().join("a");
        fs::create_dir_all(&inner).unwrap();
        for dir in [root.path(), inner.as_path()] {
            fs::write(dir.join(ENCRYPTED), "").unwrap();
            fs::write(dir.join(PLAIN), "").unwrap();
        }
        let found: Vec<_> = find(&inner, Some(root.path()))
            .into_iter()
            .map(|f| (f.path, f.encrypted))
            .collect();
        assert_eq!(
            found,
            vec![
                (root.path().join(PLAIN), false),
                (root.path().join(ENCRYPTED), true),
                (inner.join(PLAIN), false),
                (inner.join(ENCRYPTED), true),
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
