use crate::common::current_dir;
use crate::config::select;
use crate::crypto::Identities;
use crate::files::{find, start_dir};
use crate::shell::Shell;
use crate::trust::{Store, Trust};
use std::fs;

/// Prints the shell code for the current directory, outermost file first.
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
    // Read lazily, so a directory with no encrypted file never touches (or
    // prompts for) the identity, and read at most once per run.
    let mut identities: Option<Result<Identities, String>> = None;
    for file in find(&cwd, start_dir().as_deref()) {
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
        let code = if file.encrypted {
            let identities = identities.get_or_insert_with(Identities::load);
            let plain = match identities {
                Ok(identities) => identities.decrypt(&bytes),
                Err(error) => Err(error.clone()),
            };
            match plain.and_then(|p| {
                std::str::from_utf8(&p)
                    .map(|text| select(text, shell))
                    .map_err(|_| "decrypted, but not UTF-8 text".to_string())
            }) {
                Ok(code) => zeroize::Zeroizing::new(code),
                Err(error) => {
                    eprintln!("pathrc: {path}: cannot decrypt: {error}");
                    continue;
                }
            }
        } else {
            zeroize::Zeroizing::new(select(&String::from_utf8_lossy(&bytes), shell))
        };
        if !code.is_empty() {
            println!("# File: {path}");
            print!("{}", *code);
        }
    }
}
