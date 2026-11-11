//! `pathrc encrypt`, `decrypt` and `rekey`, so nobody handles age by hand.
//!
//! A file pathrc rewrites stays allowed if the file it came from was
//! allowed; it never becomes trusted just by being rewritten.

use crate::common::current_dir;
use crate::config::validate;
use crate::crypto::{encrypt, recipients_for, Identities};
use crate::files::{ENCRYPTED, PLAIN};
use crate::trust::{Store, Trust};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `file`, or `name` in the current directory.
pub fn default_file(file: Option<&Path>, name: &str) -> Result<PathBuf, String> {
    match file {
        Some(file) => Ok(file.to_path_buf()),
        None => Ok(current_dir()?.join(name)),
    }
}

pub fn dir_of(file: &Path) -> PathBuf {
    match file.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Writes `bytes` to `path` in one rename, with `mode` (or the mode of the
/// file it replaces).
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let mode = fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(mode);
    let mut tmp = tempfile::Builder::new()
        .prefix(".pathrc-write-")
        .tempfile_in(dir_of(path))
        .map_err(fail)?;
    tmp.write_all(bytes).map_err(fail)?;
    tmp.as_file().sync_all().map_err(fail)?;
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode)).map_err(fail)?;
    tmp.persist(path).map_err(|e| fail(e.error))?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Moves trust from `from` to `to` (`to` now holding `bytes`), if `from`
/// was allowed. Returns whether it was.
pub fn carry_trust(
    from: &Path,
    from_bytes: &[u8],
    to: &Path,
    bytes: &[u8],
) -> Result<bool, String> {
    let mut store = Store::load()?;
    let allowed = store.check(from, from_bytes) == Trust::Allowed;
    if from != to {
        store.deny(from);
    }
    if allowed {
        store.allow(to, bytes);
    }
    store.save()?;
    Ok(allowed)
}

fn trust_note(allowed: bool, file: &Path) -> String {
    if allowed {
        String::new()
    } else {
        format!(
            "\n{} is not allowed yet: check it, then run 'pathrc allow'",
            file.display()
        )
    }
}

fn run(action: impl FnOnce() -> Result<String, String>) -> ExitCode {
    match action() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("pathrc: {message}");
            ExitCode::FAILURE
        }
    }
}

pub fn handle_encrypt(file: Option<&Path>, force: bool) -> ExitCode {
    run(|| {
        let plain_path = default_file(file, PLAIN)?;
        let sealed_path = dir_of(&plain_path).join(ENCRYPTED);
        if sealed_path.exists() && !force {
            return Err(format!(
                "{} already exists; edit it with 'pathrc edit', or --force to replace it",
                sealed_path.display()
            ));
        }
        let plain = zeroize::Zeroizing::new(read(&plain_path)?);
        let problems = validate(&plain);
        if !problems.is_empty() {
            let list: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
            return Err(format!("{}: {}", plain_path.display(), list.join("; ")));
        }
        let (recipients, source) = recipients_for(&dir_of(&plain_path))?;
        let sealed = encrypt(&plain, &recipients)?;
        write_atomic(&sealed_path, &sealed, 0o644)?;
        let allowed = carry_trust(&plain_path, &plain, &sealed_path, &sealed)?;
        fs::remove_file(&plain_path).map_err(|e| format!("{}: {e}", plain_path.display()))?;
        Ok(format!(
            "encrypted {} to {} for {source}; removed the plaintext{}",
            plain_path.display(),
            sealed_path.display(),
            trust_note(allowed, &sealed_path)
        ))
    })
}

pub fn handle_decrypt(file: Option<&Path>, force: bool) -> ExitCode {
    run(|| {
        let sealed_path = default_file(file, ENCRYPTED)?;
        let plain_path = dir_of(&sealed_path).join(PLAIN);
        if plain_path.exists() && !force {
            return Err(format!(
                "{} already exists; --force to replace it",
                plain_path.display()
            ));
        }
        let sealed = read(&sealed_path)?;
        let plain = Identities::load()?
            .decrypt(&sealed)
            .map_err(|e| format!("{}: cannot decrypt: {e}", sealed_path.display()))?;
        write_atomic(&plain_path, &plain, 0o600)?;
        let allowed = carry_trust(&sealed_path, &sealed, &plain_path, &plain)?;
        fs::remove_file(&sealed_path).map_err(|e| format!("{}: {e}", sealed_path.display()))?;
        Ok(format!(
            "decrypted {} to {}{}",
            sealed_path.display(),
            plain_path.display(),
            trust_note(allowed, &plain_path)
        ))
    })
}

pub fn handle_rekey(files: &[PathBuf]) -> ExitCode {
    run(|| {
        let files = if files.is_empty() {
            vec![default_file(None, ENCRYPTED)?]
        } else {
            files.to_vec()
        };
        let identities = Identities::load()?;
        let mut report = Vec::new();
        for path in files {
            let sealed = read(&path)?;
            let plain = identities
                .decrypt(&sealed)
                .map_err(|e| format!("{}: cannot decrypt: {e}", path.display()))?;
            let (recipients, source) = recipients_for(&dir_of(&path))?;
            let resealed = encrypt(&plain, &recipients)?;
            write_atomic(&path, &resealed, 0o644)?;
            let allowed = carry_trust(&path, &sealed, &path, &resealed)?;
            report.push(format!(
                "re-encrypted {} for {source}{}",
                path.display(),
                trust_note(allowed, &path)
            ));
        }
        Ok(report.join("\n"))
    })
}
