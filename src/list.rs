use crate::common::current_dir;
use crate::config::validate;
use crate::files::{find, start_dir, PLAIN};
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
    let files = find(&cwd, start_dir().as_deref());
    if files.is_empty() {
        println!("no {PLAIN} files found in path");
        return ExitCode::SUCCESS;
    }
    println!("pathrc file processing order ({shell}):\n");
    for file in files {
        let mut marks = Vec::new();
        match fs::read(&file.path) {
            Err(error) => marks.push(format!("unreadable: {error}")),
            Ok(bytes) => {
                match store.check(&file.path, &bytes) {
                    Trust::Allowed => {}
                    Trust::Untrusted => marks.push("untrusted".to_string()),
                    Trust::Changed => marks.push("changed".to_string()),
                }
                let problems: Vec<_> = validate(&bytes).iter().map(|p| p.to_string()).collect();
                if !problems.is_empty() {
                    marks.push(format!("invalid: {}", problems.join("; ")));
                }
            }
        }
        let marks: String = marks.iter().map(|m| format!("  [{m}]")).collect();
        println!("{}{marks}", file.path.display());
    }
    ExitCode::SUCCESS
}
