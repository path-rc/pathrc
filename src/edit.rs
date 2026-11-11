//! `pathrc edit`: create or change a `.pathrc` or `.pathrc.age` in your editor.
//!
//! The file is decrypted into a temporary file, mode 0600 in a 0700
//! directory under `$XDG_RUNTIME_DIR` (a tmpfs on most systems), edited,
//! checked, and written back, encrypted again for `.age`. The temporary file
//! is removed on every way out: success, abort, an editor error, a panic
//! (it is dropped), and Ctrl-C (pathrc ignores SIGINT while the editor runs,
//! so it lives to clean up).

use crate::config::validate;
use crate::crypto::{encrypt, recipients_for, Identities};
use crate::files::{ENCRYPTED, PLAIN};
use crate::manage::{default_file, dir_of, read, write_atomic};
use crate::trust::Store;
use std::io::{BufRead, IsTerminal, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use zeroize::Zeroizing;

/// Lines pathrc puts at the top of the file to explain a problem. They are
/// removed again on save.
const NOTE: &str = "# pathrc: ";

const NEW_FILE: &str = "\
# pathrc: a new configuration file. Lines before any marker apply to every
# pathrc: shell; #sh (bash and zsh), #bash, #zsh and #fish start a section.
# pathrc: These lines are removed when you save.
";

pub struct EditArgs<'a> {
    pub file: Option<&'a Path>,
    pub encrypt: bool,
    pub tmp_ok: bool,
}

/// Which file to edit: as named; else `.pathrc.age` here if it exists (or
/// `--encrypt`); else `.pathrc`.
fn target(args: &EditArgs) -> Result<PathBuf, String> {
    if let Some(file) = args.file {
        return Ok(file.to_path_buf());
    }
    let sealed = default_file(None, ENCRYPTED)?;
    if args.encrypt || sealed.exists() {
        Ok(sealed)
    } else {
        default_file(None, PLAIN)
    }
}

/// A private directory for the temporary plaintext.
fn temp_dir(encrypted: bool, tmp_ok: bool) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty());
    let base = match runtime {
        Some(dir) => PathBuf::from(dir),
        None if !encrypted || tmp_ok => std::env::temp_dir(),
        None => {
            return Err(
                "XDG_RUNTIME_DIR is not set, so there is no RAM-backed place for the \
                 decrypted text; pass --tmp-ok to use the system temp directory"
                    .to_string(),
            )
        }
    };
    let dir = base.join("pathrc");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The editor command: `$VISUAL`, `$EDITOR`, else `vi`. Run through `sh`
/// so a value with arguments (`code --wait`) works.
fn run_editor(file: &Path) -> Result<(), String> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|e| !e.trim().is_empty()))
        .unwrap_or_else(|| "vi".to_string());
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(file)
        .status()
        .map_err(|e| format!("cannot run the editor '{editor}': {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the editor '{editor}' failed ({status}); nothing saved"
        ))
    }
}

/// `text` without the lines pathrc added at the top.
fn strip_notes(text: &str) -> String {
    let mut lines = text.split_inclusive('\n').peekable();
    while lines.peek().is_some_and(|l| l.starts_with(NOTE)) {
        lines.next();
    }
    lines.collect()
}

/// Asks whether to go back to the editor. Without a terminal, no.
fn edit_again() -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("pathrc: [e]dit again or [a]bort? ");
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    !answer.trim().to_lowercase().starts_with('a')
}

pub fn handle_edit(args: EditArgs) -> ExitCode {
    // Ctrl-C reaches the editor; pathrc stays alive to remove the temp file.
    let _ = ctrlc::set_handler(|| {});
    match edit(&args) {
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

fn edit(args: &EditArgs) -> Result<String, String> {
    let path = target(args)?;
    let encrypted = path.extension().is_some_and(|e| e == "age");
    let exists = path.exists();
    let original: Zeroizing<Vec<u8>> = match (exists, encrypted) {
        (false, _) => Zeroizing::new(Vec::new()),
        (true, false) => Zeroizing::new(read(&path)?),
        (true, true) => Identities::load()?
            .decrypt(&read(&path)?)
            .map_err(|e| format!("{}: cannot decrypt: {e}", path.display()))?,
    };
    let original = Zeroizing::new(
        String::from_utf8(original.to_vec())
            .map_err(|_| format!("{}: not UTF-8 text", path.display()))?,
    );
    // Check recipients before the user spends time editing.
    let recipients = if encrypted {
        Some(recipients_for(&dir_of(&path))?)
    } else {
        None
    };

    let mut temp = tempfile::Builder::new()
        .prefix("pathrc-edit-")
        .suffix(".sh")
        .tempfile_in(temp_dir(encrypted, args.tmp_ok)?)
        .map_err(|e| format!("cannot make a temporary file: {e}"))?;
    let mut shown = if exists {
        original.to_string()
    } else {
        format!("{NEW_FILE}{}", *original)
    };
    let edited = loop {
        let file = temp.as_file_mut();
        file.set_len(0)
            .and_then(|_| file.rewind())
            .and_then(|_| file.write_all(shown.as_bytes()))
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("cannot write the temporary file: {e}"))?;
        run_editor(temp.path())?;
        let text = Zeroizing::new(
            std::fs::read_to_string(temp.path())
                .map_err(|e| format!("cannot read the temporary file back: {e}"))?,
        );
        let content = Zeroizing::new(strip_notes(&text));
        let problems = validate(content.as_bytes());
        if problems.is_empty() {
            break content;
        }
        for problem in &problems {
            eprintln!("pathrc: {}: {problem}", path.display());
        }
        if !edit_again() {
            return Err("not saved".to_string());
        }
        let header = format!("{NOTE}fix these, then save (these lines are removed):\n");
        let notes: String = problems
            .iter()
            .map(|p| {
                let line = p.line + problems.len() + 1;
                format!("{NOTE}  line {line}: {}\n", p.message)
            })
            .collect();
        shown = format!("{header}{notes}{}", *content);
    };
    drop(temp); // removes the temporary plaintext

    if *edited == *original {
        return Ok(format!("{}: no changes", path.display()));
    }
    if !exists && edited.trim().is_empty() {
        return Ok(format!("{}: empty, not created", path.display()));
    }
    let bytes = match &recipients {
        Some((recipients, _)) => encrypt(edited.as_bytes(), recipients)?,
        None => edited.as_bytes().to_vec(),
    };
    write_atomic(&path, &bytes, 0o644)?;
    // You have just read and written it: it is allowed.
    let mut store = Store::load()?;
    store.allow(&path, &bytes);
    store.save()?;
    Ok(match recipients {
        Some((_, source)) => format!("saved {} (encrypted for {source})", path.display()),
        None => format!("saved {}", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_stripped_only_from_the_top() {
        let text = "# pathrc: a\n# pathrc: b\nexport A=1\n# pathrc: kept\n";
        assert_eq!(strip_notes(text), "export A=1\n# pathrc: kept\n");
        assert_eq!(strip_notes(NEW_FILE), "");
    }
}
