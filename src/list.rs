use crate::common::current_dir;
use crate::config::describe;
use crate::config::validate;
use crate::files::{dirs, in_dir, old_in_dir, start_dir, PLAIN};
use crate::shell::Shell;
use crate::trust::{Store, Trust};
use std::fs;
use std::process::ExitCode;

/// Lists the files `collate` would read, in order, with any problem found.
pub fn handle_list() -> ExitCode {
    let setup = Shell::configured().and_then(|s| Ok((s, current_dir()?, Store::load()?)));
    let (shell, cwd, store) = match setup {
        Ok(found) => found,
        Err(message) => {
            eprintln!("pathrc: {message}");
            return ExitCode::FAILURE;
        }
    };
    let scope = dirs(&cwd, start_dir().as_deref());
    let files: Vec<_> = scope.iter().flat_map(|d| in_dir(d)).collect();
    let old: Vec<_> = scope.iter().flat_map(|d| old_in_dir(d)).collect();
    if files.is_empty() && old.is_empty() {
        println!("no {PLAIN} files found in path");
        return ExitCode::SUCCESS;
    }
    println!("pathrc file processing order ({shell}):\n");
    for file in files {
        let mut marks = Vec::new();
        if file.encrypted {
            marks.push("age".to_string());
        }
        match fs::read(&file.path) {
            Err(error) => marks.push(format!("unreadable: {error}")),
            Ok(bytes) => {
                match store.check(&file.path, &bytes) {
                    Trust::Allowed => {}
                    Trust::Untrusted => marks.push("untrusted".to_string()),
                    Trust::Changed => marks.push("changed".to_string()),
                }
                // An encrypted file's structure can't be seen without decrypting it.
                let problems = if file.encrypted {
                    Vec::new()
                } else {
                    validate(&bytes)
                };
                if !problems.is_empty() {
                    marks.push(format!("invalid: {}", describe(&problems)));
                }
            }
        }
        let marks: String = marks.iter().map(|m| format!("  [{m}]")).collect();
        println!("{}{marks}", file.path.display());
    }
    if !old.is_empty() {
        println!("\nnot read, in the old format:\n");
        for path in old {
            println!("{}  [old format: convert it to {PLAIN}]", path.display());
        }
    }
    ExitCode::SUCCESS
}
