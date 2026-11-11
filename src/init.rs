use crate::InitArgs;
use clap::ValueEnum;
use envmnt;
use rust_embed::RustEmbed;
use std::{env, str};

#[derive(RustEmbed)]
#[folder = "init/"]
struct Asset;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum Shell {
    /// Shared Bash and Zsh compatible Bourne shells
    Sh,
    /// Bash Shell
    Bash,
    /// Fish shell (experimental)
    Fish,
    /// Z Shell
    Zsh,
    /// Default selected from environment
    Env,
}

pub fn get_shell_name(shell: Shell) -> String {
    match shell {
        Shell::Sh => String::from("sh"),
        Shell::Bash => String::from("bash"),
        Shell::Fish => String::from("fish"),
        Shell::Zsh => String::from("zsh"),
        Shell::Env => String::from("env"),
    }
}

/// The configuration file name for a shell, or `None` for a shell pathrc
/// doesn't know (a hand-set or stale `PATHRC_SHELL`).
pub fn get_file_name(shell_name: &str) -> Option<String> {
    match shell_name {
        "sh" | "bash" | "zsh" | "fish" => Some(format!(".pathrc-{shell_name}")),
        _ => None,
    }
}

pub fn print_init_script(shell: String) {
    let base: &str = "pathrc.";
    let filename = format!("{base}{shell}.cfg");
    let script_name: &str = &*filename;
    let script = Asset::get(script_name).unwrap();
    let string_data = str::from_utf8(script.data.as_ref());
    print!("{}", string_data.unwrap());
}

pub fn get_shell_from_env() -> String {
    let shell_env = envmnt::get_or("SHELL", "NONE");
    let (_, shell_name) = shell_env.rsplit_once('/').unwrap();
    match shell_name {
        "bash" => "bash".to_string(),
        "fish" => "fish".to_string(),
        "zsh" => "zsh".to_string(),
        _ => panic!(
            "ISSUE: Unable to determine shell from environment \
            or shell is not currently supported '{}'. If this is a \
            supported shell then add as argument",
            shell_env
        ),
    }
}

pub fn handle_init(init_args: &InitArgs) {
    let shell = get_shell_name(init_args.shell);
    let start_dir = env::current_dir()
        .expect("ISSUE: Unable to determine the current working directory during init");
    let start_dir = start_dir.to_string_lossy();
    match shell.as_str() {
        "env" => {
            let env_shell: String = get_shell_from_env();
            println!("export PATHRC_START={start_dir:?}");
            println!("export PATHRC_SHELL={:?}", env_shell);
            print_init_script(env_shell);
        }
        _ => {
            println!("export PATHRC_START={start_dir:?}");
            println!("export PATHRC_SHELL={:?}", shell);
            print_init_script(shell);
        }
    }
}
