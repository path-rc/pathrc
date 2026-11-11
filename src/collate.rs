use crate::common::{get_configured_shell, get_path, print_not_initialized};
use crate::init::get_file_name;
use crate::pathrc::PathRC;

/// Prints the shell code for the current directory. Never fails the `cd`
/// hook: problems are reported on stderr and the exit status stays 0.
pub fn handle_collate() {
    let configured_shell = get_configured_shell();
    match configured_shell.as_str() {
        "NONE" => print_not_initialized(),
        _ => {
            let Some(pathrc_filename) = get_file_name(&configured_shell) else {
                eprintln!("pathrc: unknown PATHRC_SHELL '{configured_shell}'; run 'pathrc init'");
                return;
            };
            let dir = match get_path() {
                Ok(dir) => dir,
                Err(error) => {
                    eprintln!("pathrc: cannot read the current directory: {error}");
                    return;
                }
            };
            let mut pathrc = PathRC::new(dir);
            pathrc.search_files(pathrc_filename);
            pathrc.print_files_contents();
        }
    }
}
