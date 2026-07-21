use crate::common::{get_configured_shell, get_path, print_not_initialized};
use crate::init::get_file_name;
use crate::pathrc::{PathRC};


pub fn handle_collate() {
    let configured_shell = get_configured_shell();
    match configured_shell.as_str() {
        "NONE" => print_not_initialized(),
        _ => {
            let pathrc_filename = get_file_name(configured_shell);
            let dir = get_path().unwrap();
            let mut pathrc = PathRC::new(dir);
            pathrc.search_files(pathrc_filename);
            pathrc.print_files_contents();



        }
    }
}
