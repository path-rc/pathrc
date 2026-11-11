use crate::collate::handle_collate;
use crate::init::handle_init;
use crate::list::handle_list;
use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

pub mod collate;
pub mod common;
pub mod config;
pub mod files;
pub mod init;
pub mod list;
pub mod shell;

const AFTER_HELP: &str = "\
Configuration lives in one .pathrc file per directory. Lines before any
section marker apply to every shell; a line that is exactly #sh (bash and
zsh), #bash, #zsh or #fish starts a section for those shells.";

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
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Optional Shell override
    #[arg(value_enum, default_value = "env")]
    shell: init::Shell,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Init(init_args) => {
            handle_init(init_args);
            ExitCode::SUCCESS
        }
        Commands::Collate => {
            handle_collate();
            ExitCode::SUCCESS
        }
        Commands::List => handle_list(),
    }
}
