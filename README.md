<p align="center">
  <a href="https://path-rc.com"><img src="docs/assets/img/readme-banner.png" alt="path-rc" width="720"></a>
</p>

# path-rc

## Logical project partitioning and more

[path-rc.com](https://path-rc.com) ·
[Install](https://path-rc.com/#install) ·
[Releases](https://github.com/path-rc/pathrc/releases)

### path-rc is a command line utility, `pathrc`, providing per path configurations for

* environment variables
* aliases
* `PATH` entries

Currently supported shells are:

* `Bash` and `Zsh`

Additional experimental testing with:

* `Fish`

Other `Bourne` shell derivates may also work if initialised as `Bash`.

### Installation

The latest release for Linux (x86-64, arm64) or macOS (Intel, Apple Silicon),
checked against its SHA-256 checksum and installed to `/usr/local/bin`:

```shell
curl -fsSL https://path-rc.com/install.sh | sh
```

`PATHRC_VERSION=v0.3.0` picks a release, and
`PATHRC_INSTALL_DIR=$HOME/.local/bin` installs somewhere you can write without
sudo. The archives are also on the
[releases page](https://github.com/path-rc/pathrc/releases).

From source, with [Rust](https://rust-lang.org/tools/install/):

```shell
cargo install --git https://github.com/path-rc/pathrc
```

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
| `var.NAME`     | set `NAME` in the shell without exporting it             |
| `alias.NAME`   | define alias `NAME`                                      |
| `path.prepend` | put these directories at the front of `PATH`             |
| `unset.env`    | remove these variables while here, even ones you set     |
| `unset.var`    | the same, for shell variables                            |
| `unset.alias`  | remove these aliases while here                          |
| `unset.all`    | ignore everything the outer directories' files set       |

Values are strings, used exactly as written: `"$HOME/bin"` is not expanded,
and a number such as an account ID must be quoted. A key in a shell's table
overrides the same key in `[common]`, and a deeper directory's file overrides
an outer one. Within a file, `unset` applies first, then `env`, `path` and
`alias`. Shell functions, completions and options belong in your shell's own
rc files.

A `var` is seen by the shell and its aliases but not by programs you start,
so an alias can use a secret without every command inheriting it:
`alias.api = "curl -H \"Authorization: Bearer $API_TOKEN\""` with
`var.API_TOKEN`. The shell expands `$API_TOKEN` when the alias runs.

#### Secrets: `.enc` tables

Each table can have an `.enc` table, `[common.enc]`, `[sh.enc]` and so on,
holding `env` and `var` values encrypted with
[age](https://age-encryption.org): each value one line of base64, decrypted
in memory when you enter the directory. Names stay readable, so a diff shows
which secret changed. A table's `.enc` applies straight after it, so a secret
overrides a plain default of the same name.

```toml
[common]
env.AWS_REGION = "eu-west-1"

[common.enc]
env.AWS_ACCOUNT_ID = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBu…"
var.API_TOKEN = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBv…"
```

Add one without ever seeing its ciphertext, or put it in your shell's
history:

```shell
pathrc secret AWS_ACCOUNT_ID           # prompts without echo; env.NAME in [common.enc]
pathrc secret API_TOKEN --var          # var.NAME: not exported
pathrc secret TOKEN --table zsh        # in [zsh.enc]
some-password-manager get token | pathrc secret TOKEN   # or from stdin
```

`pathrc list` shows how many encrypted values a file has, without decrypting.
If a value can't be decrypted, the file is skipped and the value is named.
See [Working with secrets](#working-with-secrets) for editing them.

`pathrc list` and `pathrc edit` report a mistake with its line and column:
an unknown table or key, a value that is not a string, or a name that is not
a valid variable or alias name.

Files of older versions are no longer read, and `pathrc list` marks them:
the marker-section `.pathrc` and `.pathrc.age` of pathrc 0.1 (move their
`export` and `alias` lines into a `.pathrc.toml`), and the whole-file
`.pathrc.toml.age` of 0.3 to 0.5 (move its values into `.enc` tables, for
example with `pathrc secret`).

The search traverses parent directories up to and including the directory
specified in the `PATHRC_START` environment variable.

### Secrets and identities

`.enc` values are encrypted with [age](https://age-encryption.org) and
decrypted in memory when you enter the directory; the plaintext is never
written to disk. pathrc uses the first identity it finds:

1. the file named in `PATHRC_IDENTITY`;
2. `~/.config/pathrc/identity` (`$XDG_CONFIG_HOME/pathrc/identity`);
3. `~/.ssh/id_ed25519`.

An age identity (`age-keygen -o ~/.config/pathrc/identity`) or an
unencrypted ssh-ed25519 or ssh-rsa key works. A passphrase-protected identity
prompts on the terminal, once per run; plugin identities such as
`age-plugin-yubikey` work if the plugin is on your `PATH`, and may prompt
once per value. A file whose values can't be decrypted is reported on stderr
and skipped; the other files still load.

#### Working with secrets

```shell
pathrc secret NAME          # add or replace one value, typed without echo
pathrc edit                 # edit .pathrc.toml here, secrets shown decrypted
pathrc rekey                # re-encrypt every value after the recipients change
```

`edit` copies the file into a private temporary file (mode 0600 in
`$XDG_RUNTIME_DIR/pathrc`, normally RAM-backed) with its `.enc` values
decrypted, opens `$VISUAL`, `$EDITOR` or `vi`, and checks the file when you
save. Values you added or changed in `.enc` tables are encrypted; the ones
you left alone keep their ciphertext, so a diff shows only what changed. If
something is wrong it reopens the editor with the problems noted at the top.
The temporary file is removed however the edit ends, Ctrl-C included. A file
with secrets is only decrypted under `XDG_RUNTIME_DIR`; without it, pass
`--tmp-ok` to use the system temp directory.

Values are encrypted for the recipients in the nearest `.pathrc-recipients`
up the directory tree, one age or SSH public key per line, so a team
project can list everyone's keys:

```shell
# ~/work/client/.pathrc-recipients
age1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs3290gq   # alice
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyForBob bob@laptop
```

With no recipients file, values are encrypted for your own identity. After
changing a recipients file, run `pathrc rekey` in each directory below it.

A file `edit` writes is allowed, and so is a new file `secret` creates.
`secret` and `rekey` keep an existing file allowed only if it was allowed
before.

### Leaving a directory

What a directory's files set is taken back out when you leave it, in bash,
zsh and fish: variables they set are unset, or get back the value they had
before (so an inner override of an outer setting is restored on the way out,
and a variable you set yourself comes back after an `unset.env`), aliases are
removed or restored, and the `PATH` entries they added are removed. Moving
sideways from one project to its sibling leaves one and enters the other.

pathrc does this by having the shell save each name's value, in a variable
of its own, before a file changes it, and put it back when you leave. The
values never pass through pathrc, so a secret is only ever in your shell.

Some consequences:

- A value you change by hand after a file set it is reset when you leave.
  Changes you make to `PATH` yourself are kept: only pathrc's entries are
  removed.
- Only a file you newly enter is read. Moving around inside a project reads
  and decrypts nothing, and going deeper reads only the deeper files.
- A new shell (a subshell, a terminal multiplexer pane) enters for itself; it
  inherits exported variables but not the record of what to undo.
- After upgrading pathrc, start a new shell: a shell started before keeps
  the old hook, and pathrc says so rather than printing code it can't run.

### Allowing files

pathrc applies its files on every `cd`, so a cloned repository containing a
`.pathrc.toml` could otherwise put its own `bin` first on your `PATH` or alias
a command you use the moment you enter it. A file is only applied once you
have read it and allowed it:

```shell
pathrc allow            # the .pathrc.toml in this directory
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
it prints the shell code that leaves the files no longer in scope, innermost
first, and enters the new ones.

#### Edit, secret and rekey

See [Working with secrets](#working-with-secrets).

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

The age identity file for decrypting `.enc` values. See
[Secrets and identities](#secrets-and-identities).

#### PATHRC_STATE

Set by the hook for each run of `collate`, from the shell's own
`__pathrc_state` variable: the files entered so far, and for each the names
it changed and the `PATH` entries it added. It holds no values. It is not
exported, so a new shell starts with nothing entered.

#### PATHRC_DEBUG

When set, `collate` reports each file it decrypts on stderr.
