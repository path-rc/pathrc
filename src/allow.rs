//! `pathrc allow` and `pathrc deny`.

use crate::common::current_dir;
use crate::files::{in_dir, ENCRYPTED, PLAIN};
use crate::trust::Store;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

/// The files named, or the configuration files in the current directory.
fn targets(files: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if !files.is_empty() {
        return Ok(files.to_vec());
    }
    let cwd = current_dir()?;
    let found: Vec<_> = in_dir(&cwd).into_iter().map(|f| f.path).collect();
    if found.is_empty() {
        return Err(format!(
            "no {PLAIN} or {ENCRYPTED} file in {}",
            cwd.display()
        ));
    }
    Ok(found)
}

pub fn handle_allow(files: &[PathBuf]) -> ExitCode {
    run(|| {
        let mut store = Store::load()?;
        for file in targets(files)? {
            let bytes = fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            store.allow(&file, &bytes);
            println!("allowed {}", file.display());
        }
        store.save()
    })
}

pub fn handle_deny(files: &[PathBuf]) -> ExitCode {
    run(|| {
        let mut store = Store::load()?;
        for file in targets(files)? {
            if store.deny(&file) {
                println!("denied {}", file.display());
            } else {
                println!("{} was not allowed", file.display());
            }
        }
        store.save()
    })
}

fn run(action: impl FnOnce() -> Result<(), String>) -> ExitCode {
    match action() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("pathrc: {message}");
            ExitCode::FAILURE
        }
    }
}
