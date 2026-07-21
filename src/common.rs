use std::env;
use std::path::PathBuf;



pub fn print_not_initialized() {
    println!("pathrc has not been initialised for this shell");
    println!("add 'pathrc init' to your shell rc file");
    println!("see 'pathrc init -h' for options");
}

pub fn get_configured_shell() -> String {
    let configured_shell  = envmnt::get_or("PATHRC_SHELL", "NONE");
    return configured_shell;
}

pub fn get_path() -> std::io::Result<PathBuf> {
    let path = env::current_dir()?;
    Ok(path)
}
