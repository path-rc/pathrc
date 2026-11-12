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
pub mod edit;
pub mod files;
pub mod init;
pub mod list;
pub mod manage;
pub mod quote;
pub mod settings;
pub mod shell;
pub mod show;
pub mod state;
pub mod trust;
pub mod undo;

const AFTER_HELP: &str = "\
Settings live in one .pathrc.toml file per directory, and optionally an
age-encrypted .pathrc.toml.age beside it, read after it. [common] applies to
every shell, [sh] to bash and zsh, and [bash], [zsh] and [fish] to one shell,
each with env.NAME, alias.NAME, path.prepend and unset.env, unset.alias and
unset.all. A shell's table overrides [common], and a deeper directory's file
overrides an outer one. In bash and zsh, leaving a directory undoes what its
files set.

A file is only read once you have allowed it with `pathrc allow`, and again
after every change to it.";

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
    /// Prints the code that applies and undoes .pathrc.toml settings after a cd (run by the hook)
    Collate,
    /// Lists the .pathrc.toml files in scope in processing order, with any problems
    List,
    /// Prints the settings in effect here for a shell, decrypted, as shell
    /// code you can read or `eval`
    Show {
        /// The shell whose tables to read [default: PATHRC_SHELL]
        #[arg(long, short, value_enum)]
        shell: Option<ShowShell>,
    },
    /// Creates or changes a .pathrc.toml or .pathrc.toml.age in $VISUAL/$EDITOR,
    /// decrypting and re-encrypting as needed
    #[command(
        long_about = "Creates or changes a .pathrc.toml or .pathrc.toml.age in $VISUAL, $EDITOR or vi.\n\n\
        With no file: .pathrc.toml.age in this directory if it exists (or with --encrypt), else \
        .pathrc.toml. \
        An encrypted file is decrypted into a private temporary file under $XDG_RUNTIME_DIR, \
        removed afterwards, and encrypted again on save for the nearest .pathrc-recipients \
        (else your own identity). The file is checked on save (TOML, tables, keys and names): \
        problems reopen the editor with notes at the top. The saved file is allowed."
    )]
    Edit {
        /// The file to edit
        file: Option<PathBuf>,
        /// Make a new file encrypted (.pathrc.toml.age)
        #[arg(long)]
        encrypt: bool,
        /// Allow the system temp directory when XDG_RUNTIME_DIR is not set
        #[arg(long)]
        tmp_ok: bool,
    },
    /// Encrypts .pathrc.toml to .pathrc.toml.age for the nearest .pathrc-recipients (or
    /// your identity), then removes the plaintext
    Encrypt {
        /// The file [default: .pathrc.toml here]
        file: Option<PathBuf>,
        /// Replace an existing .pathrc.toml.age
        #[arg(long)]
        force: bool,
    },
    /// Decrypts .pathrc.toml.age back to a plain .pathrc.toml, then removes the .age
    Decrypt {
        /// The file [default: .pathrc.toml.age here]
        file: Option<PathBuf>,
        /// Replace an existing .pathrc.toml
        #[arg(long)]
        force: bool,
    },
    /// Re-encrypts .pathrc.toml.age files for the current recipients
    Rekey {
        /// Files [default: .pathrc.toml.age here]
        files: Vec<PathBuf>,
    },
    /// Allows files to be read (default: the .pathrc.toml and .pathrc.toml.age here)
    Allow(FilesArgs),
    /// Stops files being read (default: the .pathrc.toml and .pathrc.toml.age here)
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

#[derive(Copy, Clone, Debug, clap::ValueEnum)]
enum ShowShell {
    Sh,
    Bash,
    Zsh,
    Fish,
}

impl From<ShowShell> for shell::Shell {
    fn from(shell: ShowShell) -> Self {
        match shell {
            ShowShell::Sh => shell::Shell::Sh,
            ShowShell::Bash => shell::Shell::Bash,
            ShowShell::Zsh => shell::Shell::Zsh,
            ShowShell::Fish => shell::Shell::Fish,
        }
    }
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
        Commands::Show { shell } => show::handle_show(shell.map(Into::into)),
        Commands::Edit {
            file,
            encrypt,
            tmp_ok,
        } => edit::handle_edit(edit::EditArgs {
            file: file.as_deref(),
            encrypt: *encrypt,
            tmp_ok: *tmp_ok,
        }),
        Commands::Encrypt { file, force } => manage::handle_encrypt(file.as_deref(), *force),
        Commands::Decrypt { file, force } => manage::handle_decrypt(file.as_deref(), *force),
        Commands::Rekey { files } => manage::handle_rekey(files),
        Commands::Allow(args) => handle_allow(&args.files),
        Commands::Deny(args) => handle_deny(&args.files),
        Commands::Undo { shell, mark } => undo::handle_undo(
            shell::Shell::from_name(shell).expect("clap allows only bash and zsh"),
            mark,
        ),
    }
}
