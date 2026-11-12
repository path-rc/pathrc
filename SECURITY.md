# Security

pathrc evaluates shell code from `.pathrc` files and decrypts `.pathrc.age`
files with your age or SSH identity, so a bug in it can expose secrets or run
code you did not mean to run.

## Reporting a vulnerability

Report it privately through
[GitHub's private vulnerability reporting](https://github.com/path-rc/pathrc/security/advisories/new),
not in a public issue or pull request. Include the pathrc version
(`pathrc --version`), your shell and OS, and the steps to reproduce it.

You should get a reply within a week. Fixes are released from `main`; only the
latest release is supported.

## Scope

In scope: pathrc running code or revealing a secret you did not allow, such as
a file evaluated without `pathrc allow`, or plaintext from a `.pathrc.age`
written to disk. Out of scope: anything that needs control of your account or
shell session already, and secrets after they are loaded, which are as visible
as any other environment variable.
