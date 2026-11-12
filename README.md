# PATHRC

## Logical project partitioning and more

### pathrc is a  command line utility providing per path configurations for

* environment variables
* aliases
* `PATH` entries

Currently supported shells are:

* `Bash` and `Zsh`

Additional experimental testing with:

* `Fish`

Other `Bourne` shell derivates may also work if initialised as `Bash`.

### Installation

Currently there are no automated releases.

Install Rust:(<https://rust-lang.org/tools/install/>)

From the repository root directory run:

`cargo build --release`

Either copy the target/release/pathrc executable
to your path or from the repository
root run:

`cargo install --path .`

### Configuration

Add

```shell
eval "$(pathrc init)"
```

to your shell init file (fish: `pathrc init fish | source` in
`config.fish`).

Each directory holds one `.pathrc.toml` file of settings, not shell code. It
has a `[common]` table for every shell and a table per shell:

| Table      | Read by                                       |
|------------|-----------------------------------------------|
| `[common]` | every shell                                   |
| `[sh]`     | bash and zsh (and `pathrc init sh`)           |
| `[bash]`   | bash                                          |
| `[zsh]`    | zsh                                           |
| `[fish]`   | fish                                          |

```toml
# ~/work/aws-project/.pathrc.toml
[common]
env.AWS_REGION = "eu-west-2"
env.TF_VAR_project = "billing"
path.prepend = ["bin"]           # relative to this file's directory
alias.tfp = "terraform plan"

[bash]
alias.ll = "ls -l --color=auto"
```

```toml
# ~/work/aws-project/prod/.pathrc.toml
[common]
env.AWS_ACCOUNT_ID = "111111111111"
env.AWS_REGION = "eu-west-1"     # only prod differs
unset.env = ["AWS_PROFILE"]
```

Each table can have:

| Key            | Meaning                                                  |
|----------------|----------------------------------------------------------|
| `env.NAME`     | set and export `NAME`                                    |
| `alias.NAME`   | define alias `NAME`                                      |
| `path.prepend` | put these directories at the front of `PATH`             |
| `unset.env`    | remove these variables while here, even ones you set     |
| `unset.alias`  | remove these aliases while here                          |
| `unset.all`    | ignore everything the outer directories' files set       |

Values are strings, used exactly as written: `"$HOME/bin"` is not expanded,
and a number such as an account ID must be quoted. A key in a shell's table
overrides the same key in `[common]`, and a deeper directory's file overrides
an outer one. Within a file, `unset` applies first, then `env`, `path` and
`alias`. Shell functions, completions and options belong in your shell's own
rc files.

`pathrc list` and `pathrc edit` report a mistake with its line and column:
an unknown table or key, a value that is not a string, or a name that is not
a valid variable or alias name.

The marker-section `.pathrc` files of pathrc 0.1 are no longer read;
`pathrc list` marks them `[old format: convert it to .pathrc.toml]`. Move their `export` and `alias`
lines into a `.pathrc.toml`.

The search traverses parent directories up to and including the directory
specified in the `PATHRC_START` environment variable.

### Encrypted files

A directory may also hold `.pathrc.toml.age`: a `.pathrc.toml` encrypted
with [age](https://age-encryption.org). It is read after the plain
`.pathrc.toml` in the same directory, so a secret can override a default.
pathrc decrypts it in memory; the plaintext is never written to disk.

pathrc uses the first identity it finds:

1. the file named in `PATHRC_IDENTITY`;
2. `~/.config/pathrc/identity` (`$XDG_CONFIG_HOME/pathrc/identity`);
3. `~/.ssh/id_ed25519`.

An age identity (`age-keygen -o ~/.config/pathrc/identity`) or an
unencrypted ssh-ed25519 or ssh-rsa key works. A passphrase-protected identity
prompts on the terminal; plugin identities such as `age-plugin-yubikey` work
if the plugin is on your `PATH`. A file that can't be decrypted is reported
on stderr and skipped; the other files still load. `pathrc list` marks
encrypted files `[age]` without decrypting them.

#### Working with encrypted files

```shell
pathrc edit --encrypt       # create .pathrc.toml.age here in $EDITOR
pathrc edit                 # edit .pathrc.toml.age (or .pathrc.toml) here
pathrc encrypt              # .pathrc.toml -> .pathrc.toml.age, removing the plaintext
pathrc decrypt              # .pathrc.toml.age -> .pathrc.toml
pathrc rekey                # re-encrypt after the recipients change
```

`edit` decrypts into a private temporary file (mode 0600 in
`$XDG_RUNTIME_DIR/pathrc`, normally RAM-backed), opens `$VISUAL`, `$EDITOR`
or `vi`, checks the file when you save, and encrypts the result again. If
something is wrong it reopens the editor with the problems noted at the top. The temporary file is removed however the edit ends, Ctrl-C
included. Without `XDG_RUNTIME_DIR` it refuses to put decrypted text on disk
unless you pass `--tmp-ok`.

Files are encrypted for the recipients in the nearest `.pathrc-recipients`
up the directory tree, one age or SSH public key per line, so a team
project can list everyone's keys:

```shell
# ~/work/client/.pathrc-recipients
age1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs3290gq   # alice
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyForBob bob@laptop
```

With no recipients file, a file is encrypted for your own identity. After
changing a recipients file, run `pathrc rekey` in each directory below it.

A file pathrc writes for you (`edit`) is allowed. `encrypt`, `decrypt` and
`rekey` keep a file allowed only if it was allowed before.

`encrypt` deletes the plaintext; it does not overwrite it on disk first, so
on most filesystems the old blocks remain until reused.

### Leaving a directory

In bash and zsh, what a directory's files set is taken back out when you
leave it: variables they set are unset, or get back the value they had before
(so an inner override of an outer setting is restored on the way out, and a
variable you set yourself comes back after an `unset.env`), aliases are
removed or restored, and `PATH` is put back. Moving sideways from one project
to its sibling unloads one and loads the other.

Some consequences:

- A value you change by hand after a file set it is reset when you leave.
- Files already loaded and unchanged are not read again on `cd`, so an
  encrypted file is not decrypted on every `cd` inside its scope. Crossing
  into or out of a directory with its own file reads the files in scope
  again.
- A new shell (a subshell, a terminal multiplexer pane) loads for itself; it
  inherits exported variables but not the record of what to undo.
- fish loads settings but does not unload them yet.

### Allowing files

pathrc applies its files on every `cd`, so a cloned repository containing a
`.pathrc.toml` could otherwise put its own `bin` first on your `PATH` or alias
a command you use the moment you enter it. A file is only applied once you
have read it and allowed it:

```shell
pathrc allow            # the .pathrc.toml and .pathrc.toml.age in this directory
pathrc allow path/to/.pathrc.toml
pathrc deny             # stop applying it
```

An encrypted file is allowed by the hash of its ciphertext, so checking it
never needs decryption. pathrc records each allowed file's path and SHA-256 in
`$XDG_DATA_HOME/pathrc/allowed` (`~/.local/share/pathrc/allowed`). Editing a
file means allowing it again; until then `cd` skips it with a hint on stderr,
and `pathrc list` marks it `[changed]` (or `[untrusted]` if never allowed).

A new file needs `pathrc allow` once, unless `pathrc edit` wrote it.

### Commands

#### Init

The most important command of which is `init` which initialises
the shell to use pathrc. By default, without any arguments `init` will try to
determine the shell from the SHELL environment variable. In some cases there may
be a need to explicitly specify the shell, i.e. if the SHELL environment is
missing or in an unrecognised format. Being able to override the shell might
also be useful for running under different but compatible shells i.e. specifying
bash with the `Bourne` shell. NOTE this is completely untested!

#### Collate

This is the engine the `cd` hook runs. When the files in scope have changed
it works out the settings they add up to and prints the shell code that
undoes the old ones and applies the new.

#### Edit, encrypt, decrypt and rekey

See [Working with encrypted files](#working-with-encrypted-files).

#### Show

`pathrc show` prints the settings in effect here, decrypted, as plain shell
code, after a `# File:` line for each file they come from. Use it to read
what a directory sets up, or `eval "$(pathrc show)"` it in a script.
`--shell zsh` shows another shell's. It exits 1 if any file
was left out (not allowed, unreadable or undecryptable), naming each on
stderr.

#### Allow and deny

See [Allowing files](#allowing-files).

#### List

This is a useful utility for identifying the pathrc configuration files that
will be processed at the current directory. A file that cannot be read, is
not allowed, or has a mistake in it is marked.

`collate` never breaks `cd`: a file it cannot read is reported on stderr and
skipped, and the other files still load.

#### Usage

`pathrc` itself has the usual flags for help `-h` and version `-V`.

`list` and `collate` have no further options; `pathrc <command> -h` shows
the rest.

`init` takes an optional `shell` argument with `env` being the default which
tried to fetch the current shell from the environment.

```shell
eval "$(pathrc init)"
```

or to force a shell i.e. bash

```shell
eval "$(pathrc init bash)"
```

or to read only the `[common]` and `[sh]` tables, for a shell that is
neither bash nor zsh but compatible

```shell
eval "$(pathrc init sh)"
```

### Environment variables

#### PATHRC_SHELL

This environment variable is dynamically set during init and should not need to
be manually altered.

The value of the variable is set to the name of the shell so not to rely on the
SHELL environment variable that maybe missing or different in some systems.

#### PATHRC_START

This environment variable is dynamically set during init to the shell's starting
working directory.

#### PATHRC_IDENTITY

The age identity file for decrypting `.pathrc.toml.age` files. See
[Encrypted files](#encrypted-files).

#### PATHRC_LOADED

Set by the hook for each run of `collate`, from the shell's own
`__pathrc_loaded` variable: the files loaded so far, one `sha256 path` line
each. It is not exported, so a new shell starts with nothing loaded.

#### PATHRC_DEBUG

When set, `collate` reports each file it decrypts on stderr.
