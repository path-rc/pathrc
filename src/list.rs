use crate::common::current_dir;
use crate::config::validate;
use crate::files::{find, start_dir, PLAIN};
use crate::shell::Shell;
use std::fs;
use std::process::ExitCode;

/// Lists the files `collate` would read, in order, with any problem found.
pub fn handle_list() -> ExitCode {
    let (shell, cwd) = match Shell::configured().and_then(|s| Ok((s, current_dir()?))) {
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
        let path = file.path.display();
        match fs::read(&file.path) {
            Err(error) => println!("{path}  [unreadable: {error}]"),
            Ok(bytes) => {
                let problems = validate(&bytes);
                if problems.is_empty() {
                    println!("{path}");
                } else {
                    let problems: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
                    println!("{path}  [invalid: {}]", problems.join("; "));
                }
            }
        }
    }
    ExitCode::SUCCESS
}
