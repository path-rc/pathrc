use std::path::PathBuf;

/// The working directory, or a message for stderr.
pub fn current_dir() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("cannot read the current directory: {error}"))
}
