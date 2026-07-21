use crate::common::{print_not_initialized,get_configured_shell,get_path};
use crate::pathrc::{PathRC};

pub fn handle_list() {
    let configured_shell = get_configured_shell();
    match configured_shell.as_str() {
        "NONE" => print_not_initialized(),
        _ => {
            let pathrc_filename = crate::init::get_file_name(configured_shell);
             println!("searching for files: {pathrc_filename}");
            let dir = get_path().unwrap();
            let mut pathrc = PathRC::new(dir);
            pathrc.search_files(pathrc_filename);
            pathrc.print_file_names();
        }
    }
}
