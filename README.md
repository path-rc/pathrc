# PATHRC

## Logical project partitioning and more

### pathrc is a  command line utility providing per path configurations for

* aliases
* environment variables
* functions

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

Each directory holds one `.pathrc` file, read by every shell. Lines before
any section marker apply to every shell. A line that is exactly one of these
markers starts a section, which runs to the next marker:

| Marker  | Read by                                       |
|---------|-----------------------------------------------|
| `#sh`   | bash and zsh (and `pathrc init sh`)           |
| `#bash` | bash                                          |
| `#zsh`  | zsh                                           |
| `#fish` | fish                                          |

```shell
export PROJECT=acme
#sh
alias build='make -j8'
#zsh
setopt extended_glob
#fish
alias build 'make -j8'
```

`pathrc list` flags a marker-shaped line it does not know, such as `#bahs`.
Ordinary comments have a space after the `#` and are left alone.

Migrating from the per-shell files (`.pathrc-bash`, `.pathrc-zsh`,
`.pathrc-sh`, `.pathrc-fish`), which are no longer read:

```shell
{ echo '#bash'; cat .pathrc-bash; } > .pathrc && rm .pathrc-bash
```

The search traverses parent directories up to and including the directory
specified in the `PATHRC_START` environment variable.

### Encrypted files

A directory may also hold `.pathrc.age`: a `.pathrc` (same sections)
encrypted with [age](https://age-encryption.org). It is read after the
plain `.pathrc` in the same directory, so a secret can override a default.
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

To make one by hand until `pathrc encrypt` exists:

```shell
age -a -r "$(age-keygen -y ~/.config/pathrc/identity)" -o .pathrc.age .pathrc
```

### Leaving a directory

In bash and zsh, what a directory's files set is taken back out when you
leave it: variables they created are unset, variables they changed get their
old values back (so an inner override of an outer setting is restored on the
way out), and their aliases and functions are removed or restored. Moving
sideways from one project to its sibling unloads one and loads the other.

pathrc does this by snapshotting the shell's variables, aliases and functions
before and after each file, so a file can contain any shell code. Some
consequences:

- A value you change by hand after a file set it is reset when you leave.
- Files already loaded and unchanged are not read again on `cd`, so an
  encrypted file is decrypted once when you enter its scope, not on every
  `cd` inside it. A changed file is reloaded, with every file inside it.
- A new shell (a subshell, a terminal multiplexer pane) loads for itself; it
  inherits exported variables but not the record of what to undo.
- fish loads files but does not unload them.

### Allowing files

pathrc evaluates its files on every `cd`, so a cloned repository containing a
`.pathrc` could otherwise run code the moment you enter it. A file is only
evaluated once you have read it and allowed it:

```shell
pathrc allow            # the .pathrc and .pathrc.age in this directory
pathrc allow path/to/.pathrc
pathrc deny             # stop evaluating it
```

An encrypted file is allowed by the hash of its ciphertext, so checking it
never needs decryption. pathrc records each allowed file's path and SHA-256 in
`$XDG_DATA_HOME/pathrc/allowed` (`~/.local/share/pathrc/allowed`). Editing a
file means allowing it again; until then `cd` skips it with a hint on stderr,
and `pathrc list` marks it `[changed]` (or `[untrusted]` if never allowed).

**Upgrading:** existing files are not allowed yet. Run `pathrc allow` once in
each directory that has a `.pathrc`.

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

This is the engine the `cd` hook runs. It works out which files have left
scope or changed and which are new, and prints the shell code that unloads
and loads them.

#### Allow and deny

See [Allowing files](#allowing-files).

#### List

This is a useful utility for identifying the pathrc configuration files that
will be processed at the current directory. A file that cannot be read, or has
an unknown section marker, is marked.

`collate` never breaks `cd`: a file it cannot read is reported on stderr and
skipped, and the other files still load.

#### Usage

`pathrc` itself has the usual flags for help `-h` and version `-V`.

`list` and `collate` have no further options.

`init` takes an optional `shell` argument with `env` being the default which
tried to fetch the current shell from the environment.

```shell
eval "$(pathrc init)"
```

or to force a shell i.e. bash

```shell
eval "$(pathrc init bash)"
```

or to read only the common and `#sh` sections, for a shell that is neither
bash nor zsh but compatible

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

The age identity file for decrypting `.pathrc.age` files. See
[Encrypted files](#encrypted-files).

#### PATHRC_LOADED

Set by the hook for each run of `collate`, from the shell's own
`__pathrc_loaded` variable: the files loaded so far, one `sha256 path` line
each. It is not exported, so a new shell starts with nothing loaded.

#### PATHRC_DEBUG

When set, `collate` reports each file it decrypts on stderr.
