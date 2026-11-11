//! Which files the user has allowed pathrc to evaluate.
//!
//! pathrc evaluates whatever it finds on `cd`, so a cloned repository with a
//! `.pathrc` would run its code the moment you enter it. Nothing is evaluated
//! until `pathrc allow` has recorded the file's path and the SHA-256 of its
//! bytes in `$XDG_DATA_HOME/pathrc/allowed`. For an encrypted file that is
//! the hash of the ciphertext, so checking trust never needs decryption.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    Allowed,
    /// Never allowed, or denied.
    Untrusted,
    /// Allowed once, but its contents have changed since.
    Changed,
}

/// Hex SHA-256 of `bytes`.
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The allow list's location.
pub fn store_path() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("pathrc").join("allowed"))
}

/// The allow list: `(hash, absolute path)` pairs.
pub struct Store {
    path: PathBuf,
    entries: Vec<(String, PathBuf)>,
}

impl Store {
    pub fn load() -> Result<Store, String> {
        let path = store_path().ok_or("neither XDG_DATA_HOME nor HOME is set")?;
        let entries = match fs::read_to_string(&path) {
            Ok(text) => text
                .lines()
                .filter_map(|line| {
                    let (hash, file) = line.split_once("  ")?;
                    Some((hash.to_string(), PathBuf::from(file)))
                })
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        Ok(Store { path, entries })
    }

    /// The trust state of `file`, whose current bytes are `bytes`.
    pub fn check(&self, file: &Path, bytes: &[u8]) -> Trust {
        let file = absolute(file);
        match self.entries.iter().find(|(_, path)| *path == file) {
            None => Trust::Untrusted,
            Some((recorded, _)) if *recorded == hash(bytes) => Trust::Allowed,
            Some(_) => Trust::Changed,
        }
    }

    /// Records `file` as allowed with its current contents.
    pub fn allow(&mut self, file: &Path, bytes: &[u8]) {
        let file = absolute(file);
        self.entries.retain(|(_, path)| *path != file);
        self.entries.push((hash(bytes), file));
    }

    /// Forgets `file`. Returns whether it was there.
    pub fn deny(&mut self, file: &Path) -> bool {
        let file = absolute(file);
        let before = self.entries.len();
        self.entries.retain(|(_, path)| *path != file);
        self.entries.len() != before
    }

    /// Writes the list back, replacing the old one in a single rename.
    pub fn save(&self) -> Result<(), String> {
        let fail = |error: std::io::Error| format!("{}: {error}", self.path.display());
        let dir = self.path.parent().expect("the store has a directory");
        create_private_dir(dir).map_err(fail)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(fail)?;
        for (hash, file) in &self.entries {
            writeln!(tmp, "{hash}  {}", file.display()).map_err(fail)?;
        }
        tmp.persist(&self.path).map_err(|e| fail(e.error))?;
        Ok(())
    }
}

/// Saves a file pathrc has just written itself as allowed.
pub fn allow_written(file: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut store = Store::load()?;
    store.allow(file, bytes);
    store.save()
}

/// `file` as an absolute path with symlinks resolved, or as given (made
/// absolute) when it does not exist.
fn absolute(file: &Path) -> PathBuf {
    fs::canonicalize(file).unwrap_or_else(|_| std::path::absolute(file).unwrap_or(file.into()))
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &Path) -> Store {
        Store {
            path: dir.join("pathrc").join("allowed"),
            entries: Vec::new(),
        }
    }

    #[test]
    fn allow_then_change_then_deny() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".pathrc");
        fs::write(&file, "export A=1\n").unwrap();
        let mut store = store_in(dir.path());

        assert_eq!(store.check(&file, b"export A=1\n"), Trust::Untrusted);
        store.allow(&file, b"export A=1\n");
        assert_eq!(store.check(&file, b"export A=1\n"), Trust::Allowed);
        assert_eq!(store.check(&file, b"export A=2\n"), Trust::Changed);
        assert!(store.deny(&file));
        assert!(!store.deny(&file));
        assert_eq!(store.check(&file, b"export A=1\n"), Trust::Untrusted);
    }

    #[test]
    fn saves_and_reloads_one_entry_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("with space").join(".pathrc");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "x").unwrap();
        let mut store = store_in(dir.path());
        store.allow(&file, b"old");
        store.allow(&file, b"x");
        store.save().unwrap();

        let text = fs::read_to_string(&store.path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let (hash_part, path_part) = text.trim_end().split_once("  ").unwrap();
        assert_eq!(hash_part, hash(b"x"));
        assert_eq!(PathBuf::from(path_part), fs::canonicalize(&file).unwrap());
    }
}
