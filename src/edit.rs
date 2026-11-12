//! `pathrc edit`: create or change a `.pathrc.toml` in your editor.
//!
//! The file is copied to a temporary file, mode 0600 in a 0700 directory,
//! with its `.enc` values shown decrypted; edited; checked; and written
//! back with new and changed `.enc` values encrypted, unchanged ones keeping
//! their ciphertext. A file with `.enc` values is only decrypted under
//! `$XDG_RUNTIME_DIR` (a tmpfs on most systems). The temporary file is
//! removed on every way out: success, abort, an editor error, a panic (it is
//! dropped), and Ctrl-C (pathrc ignores SIGINT while the editor runs, so it
//! lives to clean up).

use crate::config::{describe, validate, validate_revealed, Problem};
use crate::crypto::{recipients_for, seal_value, unbase64, Identities};
use crate::files::PLAIN;
use crate::manage::{default_file, dir_of, read, write_atomic};
use crate::secrets;
use crate::trust::Store;
use std::io::{BufRead, IsTerminal, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use zeroize::Zeroizing;

/// Lines pathrc puts at the top of the file to explain a problem. They are
/// removed again on save.
const NOTE: &str = "# pathrc: ";

const NEW_FILE: &str = "\
# pathrc: a new .pathrc.toml. Tables: [common] for every shell, [sh] for
# pathrc: bash and zsh, then [bash], [zsh] and [fish]. Keys in each table:
# pathrc:   env.NAME = \"value\"        set and export a variable
# pathrc:   var.NAME = \"value\"        set a shell variable, not exported
# pathrc:   alias.NAME = \"command\"    define an alias
# pathrc:   path.prepend = [\"bin\"]    put directories in front of PATH
# pathrc:   unset.env = [\"NAME\"]      remove variables while here (unset.var too)
# pathrc:   unset.alias = [\"NAME\"]    remove aliases while here
# pathrc:   unset.all = true          ignore what outer files set
# pathrc: [common.enc] and the like hold env and var values that are kept
# pathrc: encrypted: type them here in plain text and they are encrypted when
# pathrc: you save (or add one with `pathrc secret NAME`). Values are strings,
# pathrc: used as written. These lines are removed when you save.
";

const SECRETS_NOTE: &str = "\
# pathrc: values in .enc tables are shown decrypted; new and changed ones are
# pathrc: encrypted again when you save. These lines are removed.
";

pub struct EditArgs<'a> {
    pub file: Option<&'a Path>,
    pub tmp_ok: bool,
}

/// A private directory for the temporary copy. `secret`: it will hold
/// decrypted values.
fn temp_dir(secret: bool, tmp_ok: bool) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty());
    let base = match runtime {
        Some(dir) => PathBuf::from(dir),
        None if !secret || tmp_ok => std::env::temp_dir(),
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
    let path = default_file(args.file, PLAIN)?;
    let exists = path.exists();
    let original = if exists { read(&path)? } else { Vec::new() };
    let original = Zeroizing::new(
        String::from_utf8(original).map_err(|_| format!("{}: not UTF-8 text", path.display()))?,
    );
    let mut identities: Option<Result<Identities, String>> = None;
    let (revealed_text, revealed) = secrets::reveal(&original, &mut |sealed| {
        let identities = identities
            .get_or_insert_with(Identities::load)
            .as_ref()
            .map_err(|e| format!("cannot decrypt: {e}"))?;
        identities
            .decrypt(&unbase64(sealed)?)
            .map_err(|e| format!("cannot decrypt: {e}"))
    })
    .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut temp = tempfile::Builder::new()
        .prefix("pathrc-edit-")
        .suffix(".toml")
        .tempfile_in(temp_dir(!revealed.is_empty(), args.tmp_ok)?)
        .map_err(|e| format!("cannot make a temporary file: {e}"))?;
    let mut shown = Zeroizing::new(match (exists, revealed.is_empty()) {
        (false, _) => NEW_FILE.to_string(),
        (true, true) => revealed_text.to_string(),
        (true, false) => format!("{SECRETS_NOTE}{}", *revealed_text),
    });
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
        let problems = validate_revealed(content.as_bytes());
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
                let shifted = Problem {
                    line: if p.line == 0 {
                        0
                    } else {
                        p.line + problems.len() + 1
                    },
                    column: p.column,
                    message: p.message.clone(),
                };
                format!("{NOTE}  {shifted}\n")
            })
            .collect();
        shown = Zeroizing::new(format!("{header}{notes}{}", *content));
    };
    drop(temp); // removes the temporary copy

    if *edited == *revealed_text {
        return Ok(format!("{}: no changes", path.display()));
    }
    if !exists && edited.trim().is_empty() {
        return Ok(format!("{}: empty, not created", path.display()));
    }
    // Recipients only if something needs encrypting.
    let mut recipients: Option<(Vec<Box<dyn age::Recipient + Send>>, String)> = None;
    let (bytes, fresh) = secrets::seal(&edited, &revealed, &mut |plain| {
        if recipients.is_none() {
            recipients = Some(recipients_for(&dir_of(&path))?);
        }
        let (list, _) = recipients.as_ref().expect("just set");
        seal_value(plain.as_bytes(), list)
    })
    .map_err(|e| format!("{}: {e}; nothing saved", path.display()))?;
    let problems = validate(bytes.as_bytes());
    if !problems.is_empty() {
        return Err(format!(
            "{}: {}; nothing saved",
            path.display(),
            describe(&problems)
        ));
    }
    write_atomic(&path, bytes.as_bytes(), 0o644)?;
    // You have just read and written it: it is allowed.
    let mut store = Store::load()?;
    store.allow(&path, bytes.as_bytes());
    store.save()?;
    Ok(match recipients {
        Some((_, source)) => format!(
            "saved {} ({fresh} value{} encrypted for {source})",
            path.display(),
            if fresh == 1 { "" } else { "s" }
        ),
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
