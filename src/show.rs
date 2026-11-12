//! `pathrc show`: the settings in effect here, decrypted, as plain shell
//! code you can read or evaluate.

use crate::collate::{candidates, fold};
use crate::common::current_dir;
use crate::files::{find, start_dir};
use crate::shell::Shell;
use crate::trust::Store;
use std::io::Write;
use std::process::ExitCode;

/// Prints the settings the allowed files in scope add up to for `shell`,
/// as the code that would apply them, under a `# File:` line for each file
/// that went into them. Untrusted, unreadable, undecryptable or invalid
/// files are reported on stderr and make the exit status 1; the rest still
/// count.
pub fn handle_show(shell: Option<Shell>) -> ExitCode {
    let setup = shell
        .map(Ok)
        .unwrap_or_else(Shell::configured)
        .and_then(|s| Ok((s, current_dir()?, Store::load()?)));
    let (shell, cwd, store) = match setup {
        Ok(found) => found,
        Err(message) => {
            eprintln!("pathrc: {message}");
            return ExitCode::FAILURE;
        }
    };
    let files = find(&cwd, start_dir().as_deref());
    let wanted = candidates(&files, &store);
    let (settings, applied, all_applied) = fold(&wanted, shell);
    let mut stdout = std::io::stdout().lock();
    if !settings.is_empty() {
        for entry in &applied {
            // An entry is `<sha256> <path>`.
            let path = entry.split_once(' ').map_or(entry.as_str(), |(_, p)| p);
            let _ = writeln!(stdout, "# File: {path}");
        }
        let _ = stdout.write_all(settings.render(shell).as_bytes());
    }
    if all_applied && wanted.len() == files.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
