use crate::common::current_dir;
use crate::config::{describe, parse, Layer};
use crate::crypto::Identities;
use crate::files::{find, start_dir, ConfigFile};
use crate::quote::quote;
use crate::settings::Wanted;
use crate::shell::Shell;
use crate::state::{self, Entry};
use crate::trust::{hash, Store, Trust};
use std::fs;
use zeroize::Zeroizing;

/// A file in scope that may be read: readable and allowed.
pub struct Candidate {
    pub file: ConfigFile,
    pub bytes: Vec<u8>,
    pub entry: Entry,
}

/// Prints the shell code that brings the shell from what it has loaded
/// (`PATHRC_LOADED`) to the settings the current directory wants. When the
/// files in scope are the same, with the same hashes, it prints nothing and
/// reads nothing, so an encrypted file is not decrypted again. Otherwise the
/// whole cascade is folded into one set of settings, loaded as one level
/// that the shell's snapshot can undo.
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
    let candidates = candidates(&find(&cwd, start_dir().as_deref()), &store);
    let wanted_entries: Vec<Entry> = candidates.iter().map(|c| c.entry.clone()).collect();
    if loaded == wanted_entries {
        return;
    }

    let mut out = Zeroizing::new(String::new());
    for index in (0..loaded.len()).rev() {
        out.push_str(&unload(index, &state::render(&loaded[..index]), shell));
    }
    let (wanted, applied, _) = fold(&candidates, shell);
    if !applied.is_empty() {
        let code = wanted.render(shell);
        out.push_str(&load(0, &state::render(&applied), &code, shell));
    }
    print!("{}", *out);
}

/// The settings `candidates` add up to for `shell`, outermost first, with
/// the entries of the files that were applied and whether all were. Files
/// that cannot be decrypted or parsed are reported on stderr and skipped.
pub fn fold(candidates: &[Candidate], shell: Shell) -> (Wanted, Vec<Entry>, bool) {
    // Read lazily, so a scope with no encrypted file never touches (or
    // prompts for) the identity, and read at most once per run.
    let mut identities: Option<Result<Identities, String>> = None;
    let mut wanted = Wanted::default();
    let mut applied = Vec::new();
    let mut all = true;
    for candidate in candidates {
        match layer_for(candidate, shell, &mut identities) {
            Ok(layer) => {
                wanted.apply(&layer);
                applied.push(candidate.entry.clone());
            }
            Err(error) => {
                eprintln!("pathrc: {}: {error}", candidate.file.path.display());
                all = false;
            }
        }
    }
    (wanted, applied, all)
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

/// Decrypts with the identity, loading it on first use: a scope with no
/// encrypted file or value never touches (or prompts for) it, and it is
/// loaded at most once per run.
fn decrypt_with(
    identities: &mut Option<Result<Identities, String>>,
    ciphertext: &[u8],
) -> Result<crate::crypto::Plain, String> {
    let identities = identities
        .get_or_insert_with(Identities::load)
        .as_ref()
        .map_err(|e| format!("cannot decrypt: {e}"))?;
    identities
        .decrypt(ciphertext)
        .map_err(|e| format!("cannot decrypt: {e}"))
}

/// The settings a file contributes for `shell`, decrypting the file or its
/// `.enc` values if needed.
pub fn layer_for(
    candidate: &Candidate,
    shell: Shell,
    identities: &mut Option<Result<Identities, String>>,
) -> Result<Layer, String> {
    let debug = std::env::var_os("PATHRC_DEBUG").is_some();
    let path = candidate.file.path.display();
    let plain;
    let bytes: &[u8] = if candidate.file.encrypted {
        if debug {
            eprintln!("pathrc: decrypting {path}");
        }
        plain = decrypt_with(identities, &candidate.bytes)?;
        &plain
    } else {
        &candidate.bytes
    };
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string())?;
    let dir = candidate
        .file
        .path
        .parent()
        .unwrap_or(std::path::Path::new("/"));
    let config = parse(text, dir).map_err(|problems| describe(&problems))?;
    config.layer(shell, &mut |ciphertext| {
        if debug {
            eprintln!("pathrc: decrypting a value in {path}");
        }
        decrypt_with(identities, ciphertext)
    })
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
