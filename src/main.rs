use crate::collate::handle_collate;
use crate::init::handle_init;
use crate::list::handle_list;
use clap::{ArgAction, Args, Parser, Subcommand};
use std::process::ExitCode;

pub mod collate;
pub mod common;
pub mod init;
pub mod list;
mod pathrc;

#[derive(Parser, Debug)]
#[command(version, author, about, long_about)]
#[command(propagate_version = true)]

struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialises the shell
    Init(InitArgs),
    /// Collates the pathrc files
    Collate,
    /// Lists configuration files in processing order
    List,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Optional Shell override
    #[arg(value_enum, default_value = "env")]
    shell: init::Shell,
    /// Use shared config for compatible shells
    #[clap(long, short='s', action = ArgAction::SetTrue)]
    enable_shared_config: Option<bool>,
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
