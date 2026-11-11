use crate::quote::quote;
use crate::shell::Shell;
use crate::InitArgs;
use clap::ValueEnum;
use rust_embed::RustEmbed;
use std::process::ExitCode;

#[derive(RustEmbed)]
#[folder = "init/"]
struct Asset;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum InitShell {
    /// Bash or Zsh, reading only the common and #sh sections
    Sh,
    /// Bash Shell
    Bash,
    /// Fish shell (loads files but does not unload them)
    Fish,
    /// Z Shell
    Zsh,
    /// Default selected from the SHELL environment variable
    Env,
}

impl InitShell {
    fn resolve(self) -> Result<Shell, String> {
        match self {
            InitShell::Sh => Ok(Shell::Sh),
            InitShell::Bash => Ok(Shell::Bash),
            InitShell::Fish => Ok(Shell::Fish),
            InitShell::Zsh => Ok(Shell::Zsh),
            InitShell::Env => {
                let shell = std::env::var("SHELL").unwrap_or_default();
                let name = shell.rsplit('/').next().unwrap_or_default();
                match Shell::from_name(name) {
                    Some(Shell::Sh) | None => Err(format!(
                        "cannot tell the shell from SHELL='{shell}'; \
                         name it: pathrc init bash|zsh|fish|sh"
                    )),
                    Some(found) => Ok(found),
                }
            }
        }
    }
}

/// The hook script for a shell, embedded from `init/`.
fn script(shell: Shell) -> String {
    let asset = Asset::get(&format!("pathrc.{shell}.cfg")).expect("every shell has a script");
    String::from_utf8_lossy(&asset.data).into_owned()
}

pub fn handle_init(args: &InitArgs) -> ExitCode {
    if let Some(shell) = args.script {
        // Used by the sh script, which picks bash's or zsh's at runtime.
        match shell.resolve() {
            Ok(shell) => print!("{}", script(shell)),
            Err(message) => {
                eprintln!("pathrc: {message}");
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }
    let shell = match args.shell.resolve() {
        Ok(shell) => shell,
        Err(message) => {
            eprintln!("pathrc: {message}");
            return ExitCode::FAILURE;
        }
    };
    let start = match std::env::current_dir() {
        Ok(start) => start.to_string_lossy().into_owned(),
        Err(error) => {
            eprintln!("pathrc: cannot read the current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (start, name) = (quote(&start, shell), quote(shell.name(), shell));
    match shell {
        Shell::Fish => {
            println!("set -gx PATHRC_START {start}");
            println!("set -gx PATHRC_SHELL {name}");
        }
        _ => {
            println!("export PATHRC_START={start}");
            println!("export PATHRC_SHELL={name}");
        }
    }
    print!("{}", script(shell));
    ExitCode::SUCCESS
}
