//! `pathrc secret` and `rekey`, so nobody handles age by hand, and the file
//! helpers `edit` shares.
//!
//! A file pathrc rewrites stays allowed if it was allowed; it never becomes
//! trusted just by being rewritten.

use crate::common::current_dir;
use crate::config::{describe, validate};
use crate::crypto::{recipients_for, seal_value, unbase64, Identities};
use crate::files::PLAIN;
use crate::secrets;
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

pub struct SecretArgs<'a> {
    pub name: &'a str,
    pub var: bool,
    pub table: &'a str,
    pub file: Option<&'a Path>,
}

/// The value to store: typed without echo at a terminal (pinentry if there
/// is one), else everything on stdin less one final newline.
fn read_value(name: &str) -> Result<zeroize::Zeroizing<String>, String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        use age::secrecy::ExposeSecret;
        let secret =
            age::cli_common::read_secret(&format!("pathrc: the value for {name}"), name, None)
                .map_err(|e| format!("cannot read the value: {e}"))?;
        return Ok(zeroize::Zeroizing::new(secret.expose_secret().to_string()));
    }
    let mut value = zeroize::Zeroizing::new(String::new());
    std::io::stdin()
        .read_to_string(&mut value)
        .map_err(|e| format!("cannot read the value from stdin: {e}"))?;
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    Ok(value)
}

pub fn handle_secret(args: SecretArgs) -> ExitCode {
    run(|| {
        if !secrets::TABLES.contains(&args.table) {
            return Err(format!(
                "no table [{}]; tables are {}",
                args.table,
                secrets::TABLES.join(", ")
            ));
        }
        let path = default_file(args.file, PLAIN)?;
        let existed = path.exists();
        let before = if existed { read(&path)? } else { Vec::new() };
        let text = String::from_utf8(before.clone())
            .map_err(|_| format!("{}: not UTF-8 text", path.display()))?;
        let (recipients, source) = recipients_for(&dir_of(&path))?;
        let value = read_value(args.name)?;
        let sealed = seal_value(value.as_bytes(), &recipients)?;
        let kind = if args.var { "var" } else { "env" };
        let after = secrets::set(&text, args.table, kind, args.name, &sealed)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let problems = validate(after.as_bytes());
        if !problems.is_empty() {
            return Err(format!("{}: {}", path.display(), describe(&problems)));
        }
        write_atomic(&path, after.as_bytes(), 0o644)?;
        // A new file is yours; an existing one stays as trusted as it was.
        let allowed = if existed {
            carry_trust(&path, &before, &path, after.as_bytes())?
        } else {
            let mut store = Store::load()?;
            store.allow(&path, after.as_bytes());
            store.save()?;
            true
        };
        Ok(format!(
            "set [{}.enc] {kind}.{} in {} (encrypted for {source}){}",
            args.table,
            args.name,
            path.display(),
            trust_note(allowed, &path)
        ))
    })
}

pub fn handle_rekey(files: &[PathBuf]) -> ExitCode {
    run(|| {
        let files = if files.is_empty() {
            vec![default_file(None, PLAIN)?]
        } else {
            files.to_vec()
        };
        let identities = Identities::load()?;
        let mut report = Vec::new();
        for path in files {
            let before = read(&path)?;
            let text = String::from_utf8(before.clone())
                .map_err(|_| format!("{}: not UTF-8 text", path.display()))?;
            let (recipients, source) = recipients_for(&dir_of(&path))?;
            let (after, count) = secrets::rekey(
                &text,
                &mut |sealed| {
                    identities
                        .decrypt(&unbase64(sealed)?)
                        .map_err(|e| format!("cannot decrypt: {e}"))
                },
                &mut |plain| seal_value(plain, &recipients),
            )
            .map_err(|e| format!("{}: {e}", path.display()))?;
            if count == 0 {
                report.push(format!("{}: no encrypted values", path.display()));
                continue;
            }
            write_atomic(&path, after.as_bytes(), 0o644)?;
            let allowed = carry_trust(&path, &before, &path, after.as_bytes())?;
            report.push(format!(
                "re-encrypted {count} value{} in {} for {source}{}",
                if count == 1 { "" } else { "s" },
                path.display(),
                trust_note(allowed, &path)
            ));
        }
        Ok(report.join("\n"))
    })
}
