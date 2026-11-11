use crate::common::current_dir;
use crate::config::select;
use crate::files::{find, start_dir};
use crate::shell::Shell;
use std::fs;

/// Prints the shell code for the current directory, outermost file first.
/// Never fails the `cd` hook: problems are reported on stderr, the file is
/// skipped, and the exit status stays 0. stdout carries only shell code.
pub fn handle_collate() {
    let (shell, cwd) = match Shell::configured().and_then(|s| Ok((s, current_dir()?))) {
        Ok(found) => found,
        Err(message) => {
            eprintln!("pathrc: {message}");
            return;
        }
    };
    for file in find(&cwd, start_dir().as_deref()) {
        match fs::read_to_string(&file.path) {
            Ok(contents) => {
                let code = select(&contents, shell);
                if !code.is_empty() {
                    println!("# File: {}", file.path.display());
                    print!("{code}");
                }
            }
            Err(error) => eprintln!("pathrc: {}: {error}", file.path.display()),
        }
    }
}
