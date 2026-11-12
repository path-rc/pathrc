<p align="center">
  <a href="https://path-rc.com"><img src="docs/assets/img/readme-banner.png" alt="path-rc" width="720"></a>
</p>

# path-rc

**Your shell, set up for the directory you're in.**

[path-rc.com](https://path-rc.com) ·
[Install](#install) ·
[Releases](https://github.com/path-rc/pathrc/releases)

path-rc's command, `pathrc`, sets environment variables, aliases and `PATH`
entries as you `cd` into a project, and takes them away again when you leave.
Secrets stay encrypted with [age](https://age-encryption.org) until the
moment they're needed. It works in bash, zsh and fish, on Linux and macOS.

- [Install](#install)
- [A first project](#a-first-project)
- [The `.pathrc.toml` format](#the-pathrctoml-format)
- [Secrets](#secrets)
- [Leaving a directory](#leaving-a-directory)
- [Allowing files](#allowing-files)
- [Upgrading from older versions](#upgrading-from-older-versions)
- [Commands](#commands)
- [Environment variables](#environment-variables)

## Install

The latest release for Linux (x86-64, arm64) or macOS (Intel, Apple Silicon),
checked against its SHA-256 checksum and installed to `/usr/local/bin`:

```shell
curl -fsSL https://path-rc.com/install.sh | sh
```

`PATHRC_VERSION=v0.6.0` picks a release, and
`PATHRC_INSTALL_DIR=$HOME/.local/bin` installs somewhere you can write without
sudo. The archives are also on the
[releases page](https://github.com/path-rc/pathrc/releases). From source,
with [Rust](https://rust-lang.org/tools/install/):

```shell
cargo install --git https://github.com/path-rc/pathrc
```

Then hook it into your shell's rc file:

```shell
eval "$(pathrc init)"             # ~/.bashrc or ~/.zshrc
pathrc init fish | source         # ~/.config/fish/config.fish
```

`pathrc init` works out the shell from `$SHELL`; name it (`pathrc init zsh`)
if that's missing or wrong. `pathrc init sh` reads only the `[common]` and
`[sh]` tables, for a shell that is neither bash nor zsh but compatible
(untested).

On Windows, use WSL.

## A first project

An AWS project with an account per directory: the top sets what every account
shares, and each account's directory adds or overrides what differs.

```toml
# ~/work/aws-project/.pathrc.toml
[common]
env.AWS_REGION = "eu-west-2"
env.TF_VAR_project = "billing"
path.prepend = ["bin"]           # relative to this file's directory
alias.tfp = "terraform plan"
```

```toml
# ~/work/aws-project/prod/.pathrc.toml
[common]
env.AWS_REGION = "eu-west-1"     # only prod differs
unset.env = ["AWS_PROFILE"]

[common.enc]
env.AWS_ACCOUNT_ID = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBu…"
```

```shell
cd ~/work/aws-project
pathrc edit                      # write .pathrc.toml in $EDITOR; saving allows it
cd prod
pathrc edit
pathrc secret AWS_ACCOUNT_ID     # add a secret, typed without echo
pathrc list                      # the files in scope, and any problems
```

Now `cd ~/work/aws-project/prod` sets all of it, `cd ..` takes prod's back
out, and `cd ~` leaves nothing behind.

## The `.pathrc.toml` format

Each directory can hold one `.pathrc.toml` of settings, not shell code. pathrc
reads the files from your shell's starting directory (`PATHRC_START`) down to
the one you're in, outermost first.

| Table      | Read by                                       |
|------------|-----------------------------------------------|
| `[common]` | every shell                                   |
| `[sh]`     | bash and zsh (and `pathrc init sh`)           |
| `[bash]`   | bash                                          |
| `[zsh]`    | zsh                                           |
| `[fish]`   | fish                                          |

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

and an [`.enc` table](#secrets) of encrypted `env` and `var` values.

- **Values are strings, used exactly as written.** `"$HOME/bin"` is not
  expanded, and a number such as an account ID must be quoted.
- **Later wins.** A key in a shell's table overrides the same key in
  `[common]`, a table's `.enc` overrides the table, and a deeper directory's
  file overrides an outer one. Within a file, `unset` applies first, then
  `env` and `var`, `path` and `alias`.
- **A `var` stays in the shell.** The shell and its aliases see it; programs
  you start don't. So an alias can use a secret without every command
  inheriting it: `alias.api = "curl -H \"Authorization: Bearer $API_TOKEN\""`
  with `var.API_TOKEN`. The shell expands `$API_TOKEN` when the alias runs.
- **Settings, not code.** Shell functions, completions and options belong in
  your shell's own rc files.

`pathrc list` and `pathrc edit` report a mistake with its line and column:
an unknown table or key, a value that isn't a string, or a name that isn't a
valid variable or alias name.

## Secrets

Each table can have an `.enc` table, `[common.enc]`, `[sh.enc]` and so on,
holding `env` and `var` values encrypted with age: one line of base64 each,
decrypted in memory when you enter the directory and never written to disk.
Names stay readable, so a review shows which secret changed.

```toml
[common]
env.AWS_REGION = "eu-west-1"

[common.enc]
env.AWS_ACCOUNT_ID = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBu…"
var.API_TOKEN = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IFgyNTUxOSBv…"
```

### Working with secrets

```shell
pathrc secret AWS_ACCOUNT_ID       # prompts without echo; env.NAME in [common.enc]
pathrc secret API_TOKEN --var      # var.NAME: not exported
pathrc secret TOKEN --table zsh    # in [zsh.enc]
some-password-manager get token | pathrc secret TOKEN   # or from stdin
pathrc edit                        # edit .pathrc.toml here, secrets shown decrypted
pathrc rekey                       # re-encrypt every value after the recipients change
```

`secret` never puts the value in your shell's history or on screen.

`edit` copies the file into a private temporary file (mode 0600 in
`$XDG_RUNTIME_DIR/pathrc`, normally RAM-backed) with its `.enc` values
decrypted, opens `$VISUAL`, `$EDITOR` or `vi`, and checks the file when you
save. Values you added or changed in `.enc` tables are encrypted; the ones
you left alone keep their ciphertext, so a diff shows only what changed. If
something is wrong it reopens the editor with the problems noted at the top.
The temporary file is removed however the edit ends, Ctrl-C included. A file
with secrets is only decrypted under `XDG_RUNTIME_DIR`; without it (as on
macOS), pass `--tmp-ok` to use the system temp directory.

### Who can decrypt

Values are encrypted for the recipients in the nearest `.pathrc-recipients`
up the directory tree, one age or SSH public key per line, so a team project
can list everyone's keys:

```shell
# ~/work/client/.pathrc-recipients
age1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs3290gq   # alice
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyForBob bob@laptop
```

With no recipients file, values are encrypted for your own identity. After
changing a recipients file, run `pathrc rekey` in each directory below it.

To decrypt, pathrc uses the first identity it finds:

1. the file named in `PATHRC_IDENTITY`;
2. `~/.config/pathrc/identity` (`$XDG_CONFIG_HOME/pathrc/identity`);
3. `~/.ssh/id_ed25519`.

An age identity (`age-keygen -o ~/.config/pathrc/identity`) or an
unencrypted ssh-ed25519 or ssh-rsa key works. A passphrase-protected identity
prompts on the terminal, once per run; plugin identities such as
`age-plugin-yubikey` work if the plugin is on your `PATH`, and may prompt
once per value. A file whose values can't be decrypted is reported on stderr,
naming the value, and skipped; the other files still load. `pathrc list`
counts a file's encrypted values without decrypting them.

## Leaving a directory

What a directory's file set is taken back out when you leave it: variables it
set are unset, or get back the value they had before (so an inner override of
an outer setting is restored on the way out, and a variable you set yourself
comes back after an `unset.env`), aliases are removed or restored, and the
`PATH` entries it added are removed. Moving sideways from one project to its
sibling leaves one and enters the other.

pathrc does this by having the shell save each name's value, in a variable of
its own, before a file changes it, and put it back when you leave. The values
never pass through pathrc, so a secret is only ever in your shell.

- A value you change by hand after a file set it is reset when you leave.
  Changes you make to `PATH` yourself are kept: only pathrc's entries are
  removed.
- Only a file you newly enter is read. Moving around inside a project reads
  and decrypts nothing, and going deeper reads only the deeper files.
- A new shell (a subshell, a terminal multiplexer pane) enters for itself; it
  inherits exported variables but not the record of what to undo.

## Allowing files

pathrc applies its files on every `cd`, so a cloned repository containing a
`.pathrc.toml` could otherwise put its own `bin` first on your `PATH` or alias
a command you use the moment you enter it. A file is only applied once you
have read it and allowed it:

```shell
pathrc allow            # the .pathrc.toml in this directory
pathrc allow path/to/.pathrc.toml
pathrc deny             # stop applying it
```

pathrc records each allowed file's path and SHA-256 in
`$XDG_DATA_HOME/pathrc/allowed` (`~/.local/share/pathrc/allowed`); `.enc`
values are hashed as ciphertext, so checking never needs decryption. Changing
a file means allowing it again; until then `cd` skips it with a hint on
stderr, and `pathrc list` marks it `[changed]` (or `[untrusted]` if never
allowed).

A file `pathrc edit` writes is allowed, and so is a new file `pathrc secret`
creates. `secret`, `rekey` and `migrate` keep an existing file allowed only if
it, and every file it came from, was.

## Upgrading from older versions

After upgrading, start a new shell: a shell started before keeps the old
hook, and pathrc says so rather than printing code it can't run.

Files of older versions are no longer read; `pathrc list` marks them
`[old format: pathrc migrate]`, and `pathrc migrate` converts them into the
directory's `.pathrc.toml`:

- **`.pathrc` and `.pathrc.age`** (pathrc 0.1): shell code in `#sh`, `#bash`,
  `#zsh` and `#fish` sections, which become those tables. Lines that are
  plainly settings are converted: `export NAME=value`, `NAME=value`, `alias`,
  `unset`, `unalias`, prepending to `PATH`, and fish's `set -gx`, `set -g`,
  `set -e`, `alias` and `functions -e`. Anything a shell must run (a
  function, a condition, a command, `$` expansion) stops that directory's
  conversion and is listed with its line, so you can move it to your shell's
  rc file and try again.
- **`.pathrc.toml.age`** (pathrc 0.3 to 0.5): a whole `.pathrc.toml`,
  encrypted.

Variables from an encrypted file go into `.enc` tables, each encrypted for the
nearest `.pathrc-recipients`. A key that's already in `.pathrc.toml` stops the
conversion rather than being overwritten. The old files are left for you to
delete.

```shell
cd ~/work/aws-project/prod
pathrc migrate                   # or: pathrc migrate dir…
```

## Commands

| Command | |
|---|---|
| `pathrc init [bash\|zsh\|fish\|sh]` | Print the hook for your shell's rc file |
| `pathrc list` | The files in scope, in order, and any problems with them |
| `pathrc show [--shell S]` | The settings in effect here, decrypted, as shell code you can read or `eval` |
| `pathrc edit [file]` | Edit a `.pathrc.toml`, secrets shown decrypted |
| `pathrc secret NAME [--var] [--table T]` | Add or replace an encrypted value |
| `pathrc rekey [file…]` | Re-encrypt every value for the current recipients |
| `pathrc allow [file…]`, `pathrc deny [file…]` | Allow or stop a file being applied |
| `pathrc migrate [dir…]` | Convert older pathrc files |
| `pathrc collate` | Run by the hook on every `cd`; prints the code that leaves and enters files |

`pathrc <command> -h` has the details. `show` exits 1 if a file in scope was
left out (not allowed, unreadable or undecryptable), naming each on stderr.
`collate` never breaks `cd`: a file it can't use is reported on stderr and
skipped, and the other files still load.

## Environment variables

| Variable | |
|---|---|
| `PATHRC_SHELL` | The shell `pathrc init` set up; set by the hook |
| `PATHRC_START` | The shell's starting directory, where the search for files stops; set by the hook |
| `PATHRC_IDENTITY` | The age identity for decrypting `.enc` values; see [Who can decrypt](#who-can-decrypt) |
| `PATHRC_STATE` | Passed to each `collate` from the shell's `__pathrc_state`: the files entered and the names each changed, no values. Not exported, so a new shell starts afresh |
| `PATHRC_DEBUG` | When set, `collate` reports each value it decrypts on stderr |

Security reports: see [SECURITY.md](SECURITY.md).
