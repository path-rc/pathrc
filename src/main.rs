use crate::allow::{handle_allow, handle_deny};
use crate::collate::handle_collate;
use crate::init::handle_init;
use crate::list::handle_list;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

pub mod allow;
pub mod collate;
pub mod common;
pub mod config;
pub mod crypto;
pub mod files;
pub mod init;
pub mod list;
pub mod quote;
pub mod shell;
pub mod state;
pub mod trust;
pub mod undo;

const AFTER_HELP: &str = "\
Configuration lives in one .pathrc file per directory, and optionally an
age-encrypted .pathrc.age beside it, read after it. Lines before any
section marker apply to every shell; a line that is exactly #sh (bash and
zsh), #bash, #zsh or #fish starts a section for those shells.

A file is only evaluated once you have allowed it with `pathrc allow`, and
again after every change to it.";

#[derive(Parser, Debug)]
#[command(version, author, about, long_about, after_help = AFTER_HELP)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialises the shell: add `eval "$(pathrc init)"` to your shell rc file
    Init(InitArgs),
    /// Prints the shell code from every .pathrc in scope (run by the cd hook)
    Collate,
    /// Lists the .pathrc files in scope in processing order, with any problems
    List,
    /// Allows files to be evaluated (default: the .pathrc and .pathrc.age here)
    Allow(FilesArgs),
    /// Stops files being evaluated (default: the .pathrc and .pathrc.age here)
    Deny(FilesArgs),
    /// Used by the init scripts: reads two shell snapshots on stdin and
    /// prints the code undoing the difference
    #[command(name = "_undo", hide = true)]
    Undo {
        #[arg(value_parser = ["bash", "zsh"])]
        shell: String,
        /// The section marker prefix the snapshots use
        mark: String,
    },
}

#[derive(Args, Debug)]
pub struct FilesArgs {
    /// Files to act on
    files: Vec<PathBuf>,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Optional Shell override
    #[arg(value_enum, default_value = "env")]
    shell: init::InitShell,
    /// Print only a shell's hook script (used by the sh script)
    #[arg(long, value_enum, hide = true)]
    script: Option<init::InitShell>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Init(init_args) => handle_init(init_args),
        Commands::Collate => {
            handle_collate();
            ExitCode::SUCCESS
        }
        Commands::List => handle_list(),
        Commands::Allow(args) => handle_allow(&args.files),
        Commands::Deny(args) => handle_deny(&args.files),
        Commands::Undo { shell, mark } => undo::handle_undo(
            shell::Shell::from_name(shell).expect("clap allows only bash and zsh"),
            mark,
        ),
    }
}
