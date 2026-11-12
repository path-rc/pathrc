use crate::allow::{handle_allow, handle_deny};
use crate::collate::handle_collate;
use crate::init::handle_init;
use crate::list::handle_list;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

pub mod allow;
pub mod apply;
pub mod collate;
pub mod common;
pub mod config;
pub mod crypto;
pub mod edit;
pub mod files;
pub mod init;
pub mod list;
pub mod manage;
pub mod migrate;
pub mod quote;
pub mod secrets;
pub mod settings;
pub mod shell;
pub mod show;
pub mod state;
pub mod trust;

const AFTER_HELP: &str = "\
Settings live in one .pathrc.toml file per directory. [common] applies to
every shell, [sh] to bash and zsh, and [bash], [zsh] and [fish] to one shell,
each with env.NAME (exported), var.NAME (not exported), alias.NAME,
path.prepend and unset.env, unset.var, unset.alias and unset.all. Each table
can have an .enc table ([common.enc] ...) of env and var values encrypted with
age, one line of base64 each. A shell's table overrides [common], and a deeper
directory's file overrides an outer one. Leaving a directory undoes what its
files set, and puts back any value of yours they replaced.

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
    /// Creates or changes a .pathrc.toml in $VISUAL/$EDITOR, showing its
    /// encrypted values decrypted
    #[command(
        long_about = "Creates or changes a .pathrc.toml in $VISUAL, $EDITOR or vi.\n\n\
        With no file: .pathrc.toml in this directory. Values in .enc tables are shown \
        decrypted in a private temporary file (under $XDG_RUNTIME_DIR when the file has any), \
        removed afterwards. On save, new and changed .enc values are encrypted for the nearest \
        .pathrc-recipients (else your own identity); unchanged ones keep their ciphertext, so \
        a diff shows only what changed. The file is checked on save (TOML, tables, keys and \
        names): problems reopen the editor with notes at the top. The saved file is allowed."
    )]
    Edit {
        /// The file to edit
        file: Option<PathBuf>,
        /// Allow the system temp directory when XDG_RUNTIME_DIR is not set
        #[arg(long)]
        tmp_ok: bool,
    },
    /// Adds or replaces an encrypted value in a .enc table, reading it
    /// without echo (or from stdin)
    #[command(
        long_about = "Adds or replaces an encrypted value in a .enc table.\n\n\
        The value is typed without echo at a terminal, or read from stdin (less one final \
        newline), so it never appears in your shell history. It is encrypted for the nearest \
        .pathrc-recipients (else your own identity) and set as env.NAME, or var.NAME with \
        --var, in [common.enc] or the table you name. A new file is allowed; an existing one \
        stays allowed only if it was."
    )]
    Secret {
        /// The variable's name
        name: String,
        /// Set it as a shell variable, not exported (var.NAME)
        #[arg(long)]
        var: bool,
        /// The table: common, sh, bash, zsh or fish
        #[arg(long, default_value = "common")]
        table: String,
        /// The file [default: .pathrc.toml here]
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Converts older pathrc files here (.pathrc, .pathrc.age,
    /// .pathrc.toml.age) into the .pathrc.toml
    #[command(
        long_about = "Converts older pathrc files into the directory's .pathrc.toml.\n\n\
        .pathrc and .pathrc.age (pathrc 0.1, shell code in #sh, #bash, #zsh and #fish sections) \
        are read line by line: export, plain assignments, alias, unset, unalias, prepending to \
        PATH, and fish's set, alias and functions -e. Anything else (functions, conditions, \
        commands, $ expansion) stops that directory's conversion and is listed with its line. \
        .pathrc.toml.age (pathrc 0.3 to 0.5) is read as it is. Variables from an encrypted file \
        go into .enc tables, each encrypted for the nearest .pathrc-recipients. A key already in \
        .pathrc.toml stops the conversion. The result is allowed only if every file it came \
        from was; the old files are left for you to delete."
    )]
    Migrate {
        /// Directories [default: this one]
        dirs: Vec<PathBuf>,
    },
    /// Re-encrypts every .enc value for the current recipients
    Rekey {
        /// Files [default: .pathrc.toml here]
        files: Vec<PathBuf>,
    },
    /// Allows files to be read (default: the .pathrc.toml here)
    Allow(FilesArgs),
    /// Stops files being read (default: the .pathrc.toml here)
    Deny(FilesArgs),
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
        Commands::Edit { file, tmp_ok } => edit::handle_edit(edit::EditArgs {
            file: file.as_deref(),
            tmp_ok: *tmp_ok,
        }),
        Commands::Secret {
            name,
            var,
            table,
            file,
        } => manage::handle_secret(manage::SecretArgs {
            name,
            var: *var,
            table,
            file: file.as_deref(),
        }),
        Commands::Rekey { files } => manage::handle_rekey(files),
        Commands::Migrate { dirs } => migrate::handle_migrate(dirs),
        Commands::Allow(args) => handle_allow(&args.files),
        Commands::Deny(args) => handle_deny(&args.files),
    }
}
