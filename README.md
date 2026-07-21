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

to your shell init file:

By default, pathrc rc files should be named according to the shell:

* Under `Bash` pathrc searches for files named `.pathrc-bash`
* Under`Zsh` pathrc searches for files named `.pathrc-zsh`
* Under`Fish` pathrc searches for files named `.pathrc-fish`

During initialisation shells with a reasonable level of compatibility i.e.
`Bash` and `Zsh` can use the same files by setting the shell to sh. This uses
the bash compatible method for detecting a directory change and `.pathrc-sh` for
configuratoon files.

The search traverses parent directories up to and including the directory
specified in the `PATHRC_START` environment variable.

### Commands

#### Init

There are three commands the most important of which is `init` which initialises
the shell to use pathrc. By default, without any arguments `init` will try to
determine the shell from the SHELL environment variable. In some cases there may
be a need to explicitly specify the shell, i.e. if the SHELL environment is
missing or in an unrecognised format. Being able to override the shell might
also be useful for running under different but compatible shells i.e. specifying
bash with the `Bourne` shell. NOTE this is completely untested!

#### Collate

This is the engine that collates all the pathrc configuration files which get
evaluated into the shell when changing directory.

#### List

This is a useful utility for identifying the pathrc configuration files that
will be processed at the current directory.

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

or for a shared bash and zsh config

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
