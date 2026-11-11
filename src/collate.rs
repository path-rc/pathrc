use crate::common::current_dir;
use crate::config::select;
use crate::crypto::Identities;
use crate::files::{find, start_dir, ConfigFile};
use crate::quote::quote;
use crate::shell::Shell;
use crate::state::{self, Entry};
use crate::trust::{hash, Store, Trust};
use std::fs;
use zeroize::Zeroizing;

/// A file in scope that may be evaluated: readable and allowed.
pub struct Candidate {
    pub file: ConfigFile,
    pub bytes: Vec<u8>,
    pub entry: Entry,
}

/// Prints the shell code that brings the shell from what it has loaded
/// (`PATHRC_LOADED`) to what the current directory wants: unloading files
/// that left scope or changed, innermost first, then loading new ones,
/// outermost first. Files already loaded and unchanged are not read again,
/// so an encrypted one is not decrypted again.
///
/// Never fails the `cd` hook: problems are reported on stderr, the file is
/// skipped, and the exit status stays 0. stdout carries only shell code.
pub fn handle_collate() {
    let setup = Shell::configured().and_then(|s| Ok((s, current_dir()?, Store::load()?)));
    let (shell, cwd, store) = match setup {
        Ok(found) => found,
        Err(message) => {
            eprintln!("pathrc: {message}");
            return;
        }
    };
    let loaded = state::parse(&std::env::var("PATHRC_LOADED").unwrap_or_default());
    let wanted = candidates(&find(&cwd, start_dir().as_deref()), &store);
    let wanted_entries: Vec<Entry> = wanted.iter().map(|c| c.entry.clone()).collect();
    let keep = state::common_prefix(&loaded, &wanted_entries);

    let mut out = Zeroizing::new(String::new());
    for index in (keep..loaded.len()).rev() {
        out.push_str(&unload(index, &state::render(&loaded[..index]), shell));
    }
    let mut stack: Vec<Entry> = loaded[..keep].to_vec();
    // Read lazily, so a directory with no encrypted file never touches (or
    // prompts for) the identity, and read at most once per run.
    let mut identities: Option<Result<Identities, String>> = None;
    for candidate in &wanted[keep..] {
        let code = match code_for(candidate, shell, &mut identities) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("pathrc: {}: {error}", candidate.file.path.display());
                continue;
            }
        };
        stack.push(candidate.entry.clone());
        out.push_str(&load(stack.len() - 1, &state::render(&stack), &code, shell));
    }
    print!("{}", *out);
}

/// The files that are readable and allowed, reporting the rest.
pub fn candidates(files: &[ConfigFile], store: &Store) -> Vec<Candidate> {
    let mut out = Vec::new();
    for file in files {
        let path = file.path.display();
        let bytes = match fs::read(&file.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("pathrc: {path}: {error}");
                continue;
            }
        };
        match store.check(&file.path, &bytes) {
            Trust::Allowed => {}
            Trust::Untrusted => {
                eprintln!("pathrc: {path}: not allowed; check it, then run 'pathrc allow'");
                continue;
            }
            Trust::Changed => {
                eprintln!(
                    "pathrc: {path}: changed since it was allowed; check it, then run 'pathrc allow'"
                );
                continue;
            }
        }
        out.push(Candidate {
            entry: state::entry(&hash(&bytes), &file.path),
            file: file.clone(),
            bytes,
        });
    }
    out
}

/// The shell code a file contributes for `shell`, decrypting if needed.
pub fn code_for(
    candidate: &Candidate,
    shell: Shell,
    identities: &mut Option<Result<Identities, String>>,
) -> Result<Zeroizing<String>, String> {
    if !candidate.file.encrypted {
        return Ok(Zeroizing::new(select(
            &String::from_utf8_lossy(&candidate.bytes),
            shell,
        )));
    }
    if std::env::var_os("PATHRC_DEBUG").is_some() {
        eprintln!("pathrc: decrypting {}", candidate.file.path.display());
    }
    let identities = identities
        .get_or_insert_with(Identities::load)
        .as_ref()
        .map_err(|e| format!("cannot decrypt: {e}"))?;
    let plain = identities
        .decrypt(&candidate.bytes)
        .map_err(|e| format!("cannot decrypt: {e}"))?;
    let text = std::str::from_utf8(&plain).map_err(|_| "decrypted, but not UTF-8 text")?;
    Ok(Zeroizing::new(select(text, shell)))
}

/// Unloads the file at `index`, leaving `state` loaded.
fn unload(index: usize, state: &str, shell: Shell) -> String {
    match shell {
        // fish has no snapshots: it loads but cannot unload.
        Shell::Fish => format!("set -g __pathrc_loaded {}\n", quote(state, shell)),
        _ => format!("__pathrc_unload {index} {}\n", quote(state, shell)),
    }
}

/// Loads `code` as the file at `index`, leaving `state` loaded.
fn load(index: usize, state: &str, code: &str, shell: Shell) -> String {
    let state = quote(state, shell);
    match shell {
        Shell::Fish if code.is_empty() => format!("set -g __pathrc_loaded {state}\n"),
        Shell::Fish => format!(
            "eval {}\nset -g __pathrc_loaded {state}\n",
            quote(code, shell)
        ),
        // Nothing for this shell: record it, with nothing to undo.
        _ if code.is_empty() => format!("__pathrc_loaded={state}\n"),
        _ => format!("__pathrc_load {index} {state} {}\n", quote(code, shell)),
    }
}
