//! `pathrc show`: the whole configuration in scope, decrypted, as plain
//! shell code you can read or evaluate.

use crate::collate::{candidates, code_for};
use crate::common::current_dir;
use crate::files::{find, start_dir};
use crate::shell::Shell;
use crate::trust::Store;
use std::io::Write;
use std::process::ExitCode;

/// Prints every allowed file in scope for `shell`, outermost first, each
/// under a `# File:` comment. Untrusted, unreadable or undecryptable files
/// are reported on stderr and make the exit status 1; the rest still print.
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
    let mut ok = wanted.len() == files.len();
    let mut identities = None;
    let mut stdout = std::io::stdout().lock();
    for candidate in &wanted {
        match code_for(candidate, shell, &mut identities) {
            Ok(code) if code.is_empty() => {}
            Ok(code) => {
                let _ = writeln!(stdout, "# File: {}", candidate.file.path.display());
                let _ = stdout.write_all(code.as_bytes());
            }
            Err(error) => {
                eprintln!("pathrc: {}: {error}", candidate.file.path.display());
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
