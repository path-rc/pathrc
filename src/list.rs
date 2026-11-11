use crate::common::{get_configured_shell, get_path, print_not_initialized};
use crate::init::get_file_name;
use crate::pathrc::PathRC;
use std::process::ExitCode;

pub fn handle_list() -> ExitCode {
    let configured_shell = get_configured_shell();
    if configured_shell == "NONE" {
        print_not_initialized();
        return ExitCode::FAILURE;
    }
    let Some(pathrc_filename) = get_file_name(&configured_shell) else {
        eprintln!("pathrc: unknown PATHRC_SHELL '{configured_shell}'; run 'pathrc init'");
        return ExitCode::FAILURE;
    };
    println!("searching for files: {pathrc_filename}");
    let dir = match get_path() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("pathrc: cannot read the current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut pathrc = PathRC::new(dir);
    pathrc.search_files(pathrc_filename);
    pathrc.print_file_names();
    ExitCode::SUCCESS
}
