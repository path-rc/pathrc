//! Finding the `.pathrc.toml` files that apply to a directory.

use std::fs;
use std::path::{Path, PathBuf};

/// The configuration file name, the same for every shell. Secrets live in
/// it too, in `.enc` tables (RFC 0003).
pub const PLAIN: &str = ".pathrc.toml";
/// Files of older pathrc versions, no longer read: the marker-section
/// files of 0.1 (RFC 0001) and the whole-file `.pathrc.toml.age` of 0.3 to
/// 0.5 (RFC 0002). `list` marks them, and `pathrc migrate` converts them.
pub const OLD: [&str; 3] = [".pathrc", ".pathrc.age", ".pathrc.toml.age"];

/// A configuration file in scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: PathBuf,
}

/// The configuration file in one directory, if there is one.
pub fn in_dir(dir: &Path) -> Vec<ConfigFile> {
    let path = dir.join(PLAIN);
    if path.is_file() {
        vec![ConfigFile { path }]
    } else {
        Vec::new()
    }
}

/// Old-format files in one directory.
pub fn old_in_dir(dir: &Path) -> Vec<PathBuf> {
    OLD.iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

/// The configuration files from `start` (or the filesystem root, when `cwd`
/// is not inside `start`) down to `cwd`: outermost directory first, and
/// one file per directory.
pub fn find(cwd: &Path, start: Option<&Path>) -> Vec<ConfigFile> {
    dirs(cwd, start).iter().flat_map(|d| in_dir(d)).collect()
}

/// The directories in scope, outermost first.
pub fn dirs(cwd: &Path, start: Option<&Path>) -> Vec<PathBuf> {
    let start = start.and_then(|start| fs::canonicalize(start).ok());
    let mut dirs = Vec::new();
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        dirs.push(current.to_path_buf());
        let reached_start = start
            .as_deref()
            .is_some_and(|start| fs::canonicalize(current).is_ok_and(|c| c == start));
        if reached_start {
            break;
        }
        dir = current.parent();
    }
    dirs.reverse();
    dirs
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
    fn ignores_old_files_but_can_list_them() {
        let root = tempfile::tempdir().unwrap();
        for name in [".pathrc-bash", ".pathrc", ".pathrc.age", ".pathrc.toml.age"] {
            fs::write(root.path().join(name), "").unwrap();
        }
        assert!(find(root.path(), Some(root.path())).is_empty());
        assert_eq!(
            old_in_dir(root.path()),
            vec![
                root.path().join(".pathrc"),
                root.path().join(".pathrc.age"),
                root.path().join(".pathrc.toml.age")
            ]
        );
    }
}
