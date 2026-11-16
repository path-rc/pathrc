//! The `.pathrc.toml` format (RFC 0002, with RFC 0003's `var` and `.enc`):
//! settings, not shell code.
//!
//! ```toml
//! [common]                      # every shell
//! env.AWS_REGION = "eu-west-2"  # exported
//! var.STAGE = "dev"             # a shell variable, not exported
//! path.prepend = ["bin"]        # relative to this file's directory
//! alias.tfp = "terraform plan"
//!
//! [common.enc]                  # secrets: one line of base64 age each
//! env.AWS_ACCOUNT_ID = "YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+…"
//!
//! [sh]                          # bash and zsh; then [bash], [zsh], [fish]
//! unset.env = ["AWS_PROFILE"]
//! ```
//!
//! A file is parsed and checked once, without decrypting anything; then
//! [`Config::layer`] merges the tables a shell reads, each followed by its
//! `.enc`, into the settings that file contributes.

use crate::crypto::Plain;
use crate::shell::Shell;
use base64::Engine;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use toml::Spanned;
use zeroize::Zeroize;

type Text = Spanned<String>;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(default)]
    common: Table,
    #[serde(default)]
    sh: Table,
    #[serde(default)]
    bash: Table,
    #[serde(default)]
    zsh: Table,
    #[serde(default)]
    fish: Table,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Table {
    #[serde(default)]
    env: BTreeMap<Text, Text>,
    #[serde(default)]
    var: BTreeMap<Text, Text>,
    #[serde(default)]
    alias: BTreeMap<Text, Text>,
    path: Option<PathTable>,
    #[serde(default)]
    unset: Unset,
    enc: Option<Enc>,
}

/// `[common.enc]` and the like: `env` and `var` only, each value an age
/// message as one line of base64.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Enc {
    #[serde(default)]
    env: BTreeMap<Text, Text>,
    #[serde(default)]
    var: BTreeMap<Text, Text>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathTable {
    prepend: Vec<Text>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Unset {
    #[serde(default)]
    env: Vec<Text>,
    #[serde(default)]
    var: Vec<Text>,
    #[serde(default)]
    alias: Vec<Text>,
    #[serde(default)]
    all: bool,
}

impl Drop for Table {
    // Values may be secrets (an .enc value, decrypted).
    fn drop(&mut self) {
        let values = self.env.values_mut();
        for value in values
            .chain(self.var.values_mut())
            .chain(self.alias.values_mut())
        {
            value.as_mut().zeroize();
        }
    }
}

/// A parsed and checked `.pathrc.toml`.
pub struct Config {
    document: Document,
    dir: PathBuf,
    /// Where the double-quoted (TOML basic) strings start: their `$NAME`s
    /// expand. Single-quoted (literal) strings are used as written.
    basic: HashSet<usize>,
}

/// A variable a file sets: exported (`env`) or a shell variable (`var`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {
    pub value: String,
    pub exported: bool,
    /// The value has `$NAME` references for the shell to expand (a
    /// double-quoted plain value with a `$` in it).
    pub expand: bool,
}

impl Var {
    pub fn env(value: &str) -> Var {
        Var {
            value: value.to_string(),
            exported: true,
            expand: false,
        }
    }

    pub fn shell(value: &str) -> Var {
        Var {
            value: value.to_string(),
            exported: false,
            expand: false,
        }
    }
}

/// A `path.prepend` entry: a directory, or one with `$NAME` references
/// still to expand. A relative entry is joined to the file's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub text: String,
    pub expand: bool,
}

impl PathEntry {
    pub fn fixed(dir: &str) -> PathEntry {
        PathEntry {
            text: dir.to_string(),
            expand: false,
        }
    }
}

/// What one file contributes for one shell: its tables merged, a later
/// table's key overriding an earlier one's, whether `env` or `var`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Layer {
    pub unset_all: bool,
    /// From `unset.env` and `unset.var`: either way the name goes.
    pub unset_vars: BTreeSet<String>,
    pub unset_alias: BTreeSet<String>,
    pub vars: BTreeMap<String, Var>,
    /// Relative entries are already joined to the file's directory.
    pub path: Vec<PathEntry>,
    pub alias: BTreeMap<String, String>,
}

impl Drop for Layer {
    fn drop(&mut self) {
        for var in self.vars.values_mut() {
            var.value.zeroize();
        }
        for value in self.alias.values_mut() {
            value.zeroize();
        }
    }
}

/// What a decrypted `.enc` value turned out not to be.
fn decrypted_text(plain: Plain) -> Result<String, String> {
    let text = String::from_utf8(plain.to_vec()).map_err(|_| "decrypted, but not UTF-8 text")?;
    if text.contains('\0') {
        let mut text = text;
        text.zeroize();
        return Err("decrypted, but contains a NUL character".to_string());
    }
    Ok(text)
}

impl Config {
    /// The tables `shell` reads, in order, with their names: `[common]`,
    /// then `[sh]` for bash, zsh and sh, then the shell's own.
    fn tables(&self, shell: Shell) -> Vec<(&'static str, &Table)> {
        let d = &self.document;
        let (common, sh) = (("common", &d.common), ("sh", &d.sh));
        match shell {
            Shell::Sh => vec![common, sh],
            Shell::Bash => vec![common, sh, ("bash", &d.bash)],
            Shell::Zsh => vec![common, sh, ("zsh", &d.zsh)],
            Shell::Fish => vec![common, ("fish", &d.fish)],
        }
    }

    /// How many encrypted values the file holds, in every table.
    pub fn encrypted(&self) -> usize {
        let d = &self.document;
        [&d.common, &d.sh, &d.bash, &d.zsh, &d.fish]
            .iter()
            .filter_map(|t| t.enc.as_ref())
            .map(|e| e.env.len() + e.var.len())
            .sum()
    }

    /// The file's settings for `shell`. Each table is followed by its
    /// `.enc`, whose values `decrypt` turns from age messages into text, so
    /// a secret overrides a plain default of the same name.
    pub fn layer(
        &self,
        shell: Shell,
        decrypt: &mut dyn FnMut(&[u8]) -> Result<Plain, String>,
    ) -> Result<Layer, String> {
        let mut layer = Layer::default();
        let expands = |value: &Text| {
            self.basic.contains(&value.span().start) && value.get_ref().contains('$')
        };
        for (table_name, table) in self.tables(shell) {
            for (name, value) in &table.env {
                let var = Var {
                    expand: expands(value),
                    ..Var::env(value.get_ref())
                };
                layer.vars.insert(name.get_ref().clone(), var);
            }
            for (name, value) in &table.var {
                let var = Var {
                    expand: expands(value),
                    ..Var::shell(value.get_ref())
                };
                layer.vars.insert(name.get_ref().clone(), var);
            }
            for (name, value) in &table.alias {
                layer
                    .alias
                    .insert(name.get_ref().clone(), value.get_ref().clone());
            }
            if let Some(path) = &table.path {
                // One key: a shell's list replaces [common]'s.
                layer.path = path
                    .prepend
                    .iter()
                    .map(|dir| {
                        let expand = expands(dir);
                        let text = dir.get_ref();
                        // "$PROJECT/bin" is whatever $PROJECT is; "bin" and
                        // "sub/$X" are under this file's directory.
                        let text = if expand && text.starts_with('$') {
                            text.clone()
                        } else {
                            resolve(&self.dir, text).to_string_lossy().into_owned()
                        };
                        PathEntry { text, expand }
                    })
                    .collect();
            }
            let unset = &table.unset;
            layer.unset_vars.extend(
                unset
                    .env
                    .iter()
                    .chain(&unset.var)
                    .map(|n| n.get_ref().clone()),
            );
            layer
                .unset_alias
                .extend(unset.alias.iter().map(|n| n.get_ref().clone()));
            layer.unset_all |= unset.all;

            let Some(enc) = &table.enc else { continue };
            let entries = enc.env.iter().map(|e| (e, "env", true));
            for ((name, cipher), key, exported) in
                entries.chain(enc.var.iter().map(|e| (e, "var", false)))
            {
                let what = format!("[{table_name}.enc] {key}.{}", name.get_ref());
                // Checked when parsed, so this decodes.
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(cipher.get_ref())
                    .map_err(|e| format!("{what}: {e}"))?;
                let text = decrypt(&bytes)
                    .and_then(decrypted_text)
                    .map_err(|e| format!("{what}: {e}"))?;
                // A secret is used exactly as written, whatever its quotes.
                layer.vars.insert(
                    name.get_ref().clone(),
                    Var {
                        value: text,
                        exported,
                        expand: false,
                    },
                );
            }
        }
        Ok(layer)
    }
}

/// `entry` against `dir`, without touching the filesystem.
fn resolve(dir: &Path, entry: &str) -> PathBuf {
    let path = Path::new(entry);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    };
    joined
        .components()
        .filter(|c| *c != Component::CurDir)
        .collect()
}

/// A problem in a `.pathrc.toml`, at a position in it.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based; 0 for the file as a whole.
    pub line: usize,
    /// 1-based, in characters; 0 when unknown.
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (0, _) => write!(f, "{}", self.message),
            (line, 0) => write!(f, "line {line}: {}", self.message),
            (line, column) => write!(f, "line {line}, column {column}: {}", self.message),
        }
    }
}

/// Joins problems for a one-line report.
pub fn describe(problems: &[Problem]) -> String {
    let list: Vec<_> = problems.iter().map(|p| p.to_string()).collect();
    list.join("; ")
}

struct Checker<'a> {
    text: &'a str,
    problems: Vec<Problem>,
    /// `.enc` values are shown decrypted (`pathrc edit`), so they are
    /// checked as text rather than as age messages.
    revealed: bool,
    /// Where double-quoted strings start.
    basic: HashSet<usize>,
    /// The variables this file sets, in any table.
    file_names: BTreeSet<String>,
}

impl Checker<'_> {
    fn at(&mut self, offset: usize, message: String) {
        let before = &self.text[..offset.min(self.text.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        self.problems.push(Problem {
            line,
            column,
            message,
        });
    }

    fn env_name(&mut self, name: &Text, what: &str) {
        let n = name.get_ref();
        let valid = n.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            let message = format!(
                "'{n}' is not a variable name (letters, digits and _, not starting with a digit)"
            );
            self.at(name.span().start, message);
        } else if n == "PATH" {
            self.at(
                name.span().start,
                format!("{what} PATH is not allowed; use path.prepend"),
            );
        }
    }

    fn alias_name(&mut self, name: &Text) {
        let n = name.get_ref();
        let valid = !n.is_empty()
            && !n.starts_with('-')
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.:+@%,-".contains(c));
        if !valid {
            self.at(
                name.span().start,
                format!("'{n}' is not an alias name (letters, digits and _.:+@%,-)"),
            );
        }
    }

    fn no_nul(&mut self, value: &Text) {
        if value.get_ref().contains('\0') {
            self.at(value.span().start, "contains a NUL character".to_string());
        }
    }

    fn enc_value(&mut self, value: &Text) {
        if self.revealed {
            self.no_nul(value);
        } else {
            self.age_value(value);
        }
    }

    /// One line of base64 that decodes to an age message.
    fn age_value(&mut self, value: &Text) {
        let decoded = base64::engine::general_purpose::STANDARD.decode(value.get_ref());
        if !decoded.is_ok_and(|bytes| bytes.starts_with(b"age-encryption.org/v1\n")) {
            self.at(
                value.span().start,
                "not an encrypted value: expected one line of base64 age; \
                 add one with `pathrc secret NAME`, or edit with `pathrc edit`"
                    .to_string(),
            );
        }
    }

    /// A name set by both `env` and `var` in the same table (plain or
    /// `.enc`), which would leave its kind to chance.
    fn both(&mut self, env: &BTreeMap<Text, Text>, var: &BTreeMap<Text, Text>) {
        for name in var
            .keys()
            .filter(|n| env.contains_key(n.get_ref().as_str()))
        {
            self.at(
                name.span().start,
                format!(
                    "'{}' is set by both env and var here; it is either exported or not",
                    name.get_ref()
                ),
            );
        }
    }

    /// A double-quoted plain value's `$`s: each must be a reference (or
    /// `$$`), and none may refer to another name this file sets, since the
    /// order within a file isn't the file's to choose. `own` is the name
    /// being set: referring to its previous value is fine.
    fn references(&mut self, own: Option<&str>, value: &Text) {
        let start = value.span().start;
        if !self.text[start..].starts_with('"') {
            return;
        }
        self.basic.insert(start);
        if !value.get_ref().contains('$') {
            return;
        }
        match crate::expand::parse(value.get_ref()) {
            Err(message) => self.at(start, message),
            Ok(parts) => {
                let clashes: Vec<String> = crate::expand::refs(&parts)
                    .filter(|r| Some(*r) != own && self.file_names.contains(*r))
                    .map(String::from)
                    .collect();
                for name in clashes {
                    self.at(
                        start,
                        format!(
                            "refers to ${name}, which this file also sets; a value can use \
                             outer directories' variables and its own previous value"
                        ),
                    );
                }
            }
        }
    }

    fn table(&mut self, table: &Table) {
        for (name, value) in &table.env {
            self.env_name(name, "env.");
            self.no_nul(value);
            self.references(Some(name.get_ref()), value);
        }
        for (name, value) in &table.var {
            self.env_name(name, "var.");
            self.no_nul(value);
            self.references(Some(name.get_ref()), value);
        }
        self.both(&table.env, &table.var);
        for name in &table.unset.var {
            self.env_name(name, "unsetting");
        }
        if let Some(enc) = &table.enc {
            for (name, value) in &enc.env {
                self.env_name(name, "env.");
                self.enc_value(value);
            }
            for (name, value) in &enc.var {
                self.env_name(name, "var.");
                self.enc_value(value);
            }
            self.both(&enc.env, &enc.var);
        }
        for (name, value) in &table.alias {
            self.alias_name(name);
            self.no_nul(value);
        }
        for name in &table.unset.env {
            self.env_name(name, "unsetting");
        }
        for name in &table.unset.alias {
            self.alias_name(name);
        }
        for dir in table.path.iter().flat_map(|p| &p.prepend) {
            let d = dir.get_ref();
            let message = if d.is_empty() {
                Some("an empty directory".to_string())
            } else if d.contains(':') {
                Some(format!("'{d}' contains ':', which separates PATH entries"))
            } else if d.starts_with('~') {
                Some(format!("'{d}': ~ is not expanded; write the full path"))
            } else {
                None
            };
            if let Some(message) = message {
                self.at(dir.span().start, message);
            }
            self.no_nul(dir);
            self.references(None, dir);
        }
    }
}

/// Parses and checks a file's text. `dir` is the file's directory, for
/// relative `path.prepend` entries.
pub fn parse(text: &str, dir: &Path) -> Result<Config, Vec<Problem>> {
    check(text, dir, false)
}

fn check(text: &str, dir: &Path, revealed: bool) -> Result<Config, Vec<Problem>> {
    let mut checker = Checker {
        text,
        problems: Vec::new(),
        revealed,
        basic: HashSet::new(),
        file_names: BTreeSet::new(),
    };
    let document: Document = match toml::from_str(text) {
        Ok(document) => document,
        Err(error) => {
            let mut message = error.message().trim().to_string();
            // A leading zero is refused before any type check, and is the
            // shape of an AWS account ID.
            if message.contains("expected a string") || message.contains("leading zero") {
                message.push_str("; values are strings, so quote it");
            }
            match error.span() {
                Some(span) => checker.at(span.start, message),
                None => checker.problems.push(Problem {
                    line: 0,
                    column: 0,
                    message,
                }),
            }
            return Err(checker.problems);
        }
    };
    let tables = [
        &document.common,
        &document.sh,
        &document.bash,
        &document.zsh,
        &document.fish,
    ];
    for table in tables {
        let enc = table
            .enc
            .iter()
            .flat_map(|e| e.env.keys().chain(e.var.keys()));
        let names = table.env.keys().chain(table.var.keys()).chain(enc);
        checker
            .file_names
            .extend(names.map(|n| n.get_ref().clone()));
    }
    for table in tables {
        checker.table(table);
    }
    if checker.problems.is_empty() {
        Ok(Config {
            document,
            dir: dir.to_path_buf(),
            basic: checker.basic,
        })
    } else {
        checker.problems.sort_by_key(|p| (p.line, p.column));
        Err(checker.problems)
    }
}

/// The problems in a file's bytes, for `list`, `secret` and `rekey`.
pub fn validate(bytes: &[u8]) -> Vec<Problem> {
    validate_with(bytes, false)
}

/// The same for `edit`'s copy, whose `.enc` values are shown decrypted.
pub fn validate_revealed(bytes: &[u8]) -> Vec<Problem> {
    validate_with(bytes, true)
}

fn validate_with(bytes: &[u8], revealed: bool) -> Vec<Problem> {
    match std::str::from_utf8(bytes) {
        Err(error) => vec![Problem {
            line: 0,
            column: 0,
            message: format!("not UTF-8 text ({error})"),
        }],
        Ok(text) => check(text, Path::new("/"), revealed)
            .err()
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"
[common]
env.A = "common"
env.B = "common"
alias.ll = "ls -l"
path.prepend = ["bin", "/opt/tool/bin"]
unset.env = ["GONE"]

[sh]
env.B = "sh"
unset.alias = ["old"]

[bash]
env.A = "bash"

[zsh]
path.prepend = ["./zbin"]

[fish]
alias.ll = "ls -lh"
unset.all = true
"#;

    fn layer(text: &str, shell: Shell) -> Layer {
        parse(text, Path::new("/proj"))
            .map_err(|p| describe(&p))
            .unwrap()
            .layer(shell, &mut |_| Err("no identity in this test".to_string()))
            .unwrap()
    }

    fn strings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn envs(pairs: &[(&str, &str)]) -> BTreeMap<String, Var> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Var::env(v)))
            .collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn bash_reads_common_sh_and_bash_with_later_keys_winning() {
        let l = layer(FILE, Shell::Bash);
        assert_eq!(l.vars, envs(&[("A", "bash"), ("B", "sh")]));
        assert_eq!(l.alias, strings(&[("ll", "ls -l")]));
        assert_eq!(
            l.path,
            vec![
                PathEntry::fixed("/proj/bin"),
                PathEntry::fixed("/opt/tool/bin")
            ]
        );
        assert_eq!(l.unset_vars, set(&["GONE"]));
        assert_eq!(l.unset_alias, set(&["old"]));
        assert!(!l.unset_all);
    }

    #[test]
    fn zsh_reads_sh_and_its_own_path_list_replaces_commons() {
        let l = layer(FILE, Shell::Zsh);
        assert_eq!(l.vars, envs(&[("A", "common"), ("B", "sh")]));
        assert_eq!(l.path, vec![PathEntry::fixed("/proj/zbin")]);
    }

    #[test]
    fn fish_skips_sh() {
        let l = layer(FILE, Shell::Fish);
        assert_eq!(l.vars, envs(&[("A", "common"), ("B", "common")]));
        assert_eq!(l.alias, strings(&[("ll", "ls -lh")]));
        assert!(l.unset_alias.is_empty());
        assert!(l.unset_all);
    }

    #[test]
    fn shared_sh_reads_common_and_sh() {
        let l = layer(FILE, Shell::Sh);
        assert_eq!(l.vars, envs(&[("A", "common"), ("B", "sh")]));
    }

    #[test]
    fn an_empty_file_is_an_empty_layer() {
        assert_eq!(layer("", Shell::Bash), Layer::default());
        assert_eq!(layer("# just a comment\n", Shell::Fish), Layer::default());
    }

    #[test]
    fn double_quoted_values_expand_and_single_quoted_ones_dont() {
        let l = layer(
            "[common]\nenv.D = \"$HOME/x\"\nenv.S = '$HOME/x'\nenv.P = \"plain\"\nvar.V = \"${HOME}\"\n",
            Shell::Bash,
        );
        assert!(l.vars["D"].expand && l.vars["V"].expand);
        assert!(!l.vars["S"].expand, "single quotes are used as written");
        assert!(!l.vars["P"].expand, "no $, nothing to expand");
        assert_eq!(l.vars["S"].value, "$HOME/x");
    }

    #[test]
    fn a_bad_dollar_or_a_reference_to_this_files_own_names_is_an_error() {
        let p = problems("[common]\nenv.A = \"$(id)\"\n");
        assert!(
            p[0].starts_with("line 2, column 9:") && p[0].contains("single quotes"),
            "{p:?}"
        );
        assert!(
            problems("[common]\nenv.A = '$(id)'\n").is_empty(),
            "single quotes: as written"
        );
        let p = problems("[common]\nenv.A = \"1\"\n[bash]\nenv.B = \"$A/x\"\n");
        assert!(
            p[0].contains("refers to $A, which this file also sets"),
            "{p:?}"
        );
        // Its own previous value, and anything set elsewhere, is fine.
        assert!(problems("[common]\nenv.PATHS = \"$PATHS:/x\"\nvar.C = \"$OUTER\"\n").is_empty());
        let p = problems("[common]\npath.prepend = [\"$A/bin\"]\nvar.A = \"1\"\n");
        assert!(p[0].contains("refers to $A"), "{p:?}");
    }

    #[test]
    fn enc_values_never_expand() {
        let text = format!(
            "[common.enc]\nenv.SECRET = \"{}\"\n",
            fake_age("pa$$word$HOME")
        );
        let l = layer_with_secrets(&text, Shell::Bash).unwrap();
        assert_eq!(l.vars["SECRET"], Var::env("pa$$word$HOME"));
    }

    #[test]
    fn path_entries_keep_references_for_later() {
        let l = layer(
            "[common]\npath.prepend = [\"$HOME/bin\", \"sub/$X\", '$lit', \"rel\"]\n",
            Shell::Bash,
        );
        let entries: Vec<(&str, bool)> =
            l.path.iter().map(|e| (e.text.as_str(), e.expand)).collect();
        assert_eq!(
            entries,
            [
                ("$HOME/bin", true),
                ("/proj/sub/$X", true),
                ("/proj/$lit", false),
                ("/proj/rel", false)
            ]
        );
    }

    fn problems(text: &str) -> Vec<String> {
        validate(text.as_bytes())
            .iter()
            .map(|p| p.to_string())
            .collect()
    }

    #[test]
    fn unknown_tables_and_keys_are_errors_with_positions() {
        let p = problems("[bahs]\nenv.A = \"1\"\n");
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("line 1, column "), "{p:?}");
        assert!(p[0].contains("bahs"), "{p:?}");
        let p = problems("[common]\nenvv.A = \"1\"\n");
        assert!(p[0].starts_with("line 2, column 1: "), "{p:?}");
        assert!(problems("[common]\nunset.everything = true\n")[0].contains("everything"));
    }

    #[test]
    fn a_number_value_says_to_quote_it() {
        let p = problems("[common]\nenv.AWS_ACCOUNT_ID = 012345678901\n");
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("line 2"), "{p:?}");
        assert!(p[0].ends_with("quote it"), "{p:?}");
        assert!(problems("[sh]\nalias.t = true\n")[0].ends_with("quote it"));
        let p = problems("[common]\nenv.N = 5\n");
        assert!(p[0].ends_with("quote it"), "{p:?}");
    }

    #[test]
    fn bad_names_are_reported_where_they_are() {
        let p = problems(
            "[common]\nenv.\"A-B\" = \"1\"\nalias.\"two words\" = \"x\"\nunset.env = [\"9X\"]\n",
        );
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p[0].starts_with("line 2, column 5: 'A-B' is not a variable name"));
        assert!(p[1].starts_with("line 3, column 7: 'two words' is not an alias name"));
        assert!(p[2].starts_with("line 4, column 14: '9X'"));
    }

    #[test]
    fn path_is_set_only_through_prepend() {
        let p = problems("[common]\nenv.PATH = \"/x\"\n");
        assert!(p[0].contains("use path.prepend"), "{p:?}");
        let p = problems("[sh]\npath.prepend = [\"a:b\", \"~/bin\", \"\"]\n");
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p[0].contains("contains ':'"));
        assert!(p[1].contains("~ is not expanded"));
        assert!(p[2].contains("empty directory"));
    }

    #[test]
    fn binary_content_is_reported_for_the_whole_file() {
        assert_eq!(validate(&[0xff, 0xfe])[0].line, 0);
        assert!(!problems("[common]\nenv.A = \"a\\u0000b\"\n").is_empty());
    }

    #[test]
    fn a_valid_file_has_no_problems() {
        assert_eq!(problems(FILE), Vec::<String>::new());
    }

    /// A stand-in age message: the real header, then the "plaintext", so a
    /// test decrypt can return it without keys.
    fn fake_age(plain: &str) -> String {
        let bytes = format!("age-encryption.org/v1\n{plain}");
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn fake_decrypt(bytes: &[u8]) -> Result<Plain, String> {
        let text = std::str::from_utf8(bytes).unwrap();
        let plain = text.strip_prefix("age-encryption.org/v1\n").unwrap();
        Ok(zeroize::Zeroizing::new(plain.as_bytes().to_vec()))
    }

    fn layer_with_secrets(text: &str, shell: Shell) -> Result<Layer, String> {
        parse(text, Path::new("/proj"))
            .map_err(|p| describe(&p))?
            .layer(shell, &mut fake_decrypt)
    }

    #[test]
    fn var_is_a_shell_variable_and_overrides_like_env() {
        let l = layer(
            "[common]\nenv.A = \"exported\"\nvar.B = \"shell\"\n[bash]\nvar.A = \"now a var\"\n",
            Shell::Bash,
        );
        assert_eq!(l.vars["A"], Var::shell("now a var"));
        assert_eq!(l.vars["B"], Var::shell("shell"));
        let l = layer(
            "[common]\nunset.var = [\"B\"]\nunset.env = [\"A\"]\n",
            Shell::Zsh,
        );
        assert_eq!(l.unset_vars, set(&["A", "B"]));
    }

    #[test]
    fn enc_follows_its_table_and_overrides_a_plain_default() {
        let text = format!(
            "[common]\nenv.REGION = \"plain\"\nenv.KEEP = \"plain\"\n\
             [common.enc]\nenv.REGION = \"{}\"\nvar.TOKEN = \"{}\"\n\
             [bash]\nenv.TOKEN = \"bash plain wins over common.enc\"\n\
             [zsh.enc]\nenv.KEEP = \"{}\"\n",
            fake_age("secret-region"),
            fake_age("s3cret"),
            fake_age("zsh secret"),
        );
        let bash = layer_with_secrets(&text, Shell::Bash).unwrap();
        assert_eq!(bash.vars["REGION"], Var::env("secret-region"));
        assert_eq!(bash.vars["KEEP"], Var::env("plain"));
        assert_eq!(
            bash.vars["TOKEN"],
            Var::env("bash plain wins over common.enc")
        );
        let zsh = layer_with_secrets(&text, Shell::Zsh).unwrap();
        assert_eq!(zsh.vars["TOKEN"], Var::shell("s3cret"));
        assert_eq!(zsh.vars["KEEP"], Var::env("zsh secret"));
        assert_eq!(parse(&text, Path::new("/")).ok().unwrap().encrypted(), 3);
    }

    #[test]
    fn a_decrypt_failure_names_the_value() {
        let text = format!("[sh.enc]\nvar.TOKEN = \"{}\"\n", fake_age("x"));
        let error = parse(&text, Path::new("/"))
            .ok()
            .unwrap()
            .layer(Shell::Bash, &mut |_| Err("no identity".to_string()))
            .unwrap_err();
        assert_eq!(error, "[sh.enc] var.TOKEN: no identity");
        // Shells that don't read [sh] don't decrypt it.
        assert!(layer_with_secrets(&text, Shell::Fish).is_ok());
    }

    #[test]
    fn enc_holds_only_env_and_var_and_only_age_values() {
        let p = problems("[common.enc]\nalias.x = \"y\"\n");
        assert!(
            p[0].starts_with("line 2") && p[0].contains("alias"),
            "{p:?}"
        );
        let p = problems("[common.enc]\nenv.X = \"plain text\"\n");
        assert!(
            p[0].starts_with("line 2, column 9: not an encrypted value"),
            "{p:?}"
        );
        let not_age = base64::engine::general_purpose::STANDARD.encode("hello");
        assert_eq!(
            problems(&format!("[common.enc]\nenv.X = \"{not_age}\"\n")).len(),
            1
        );
        assert!(problems(&format!("[common.enc]\nenv.X = \"{}\"\n", fake_age("ok"))).is_empty());
    }

    #[test]
    fn a_name_set_by_both_env_and_var_in_one_table_is_an_error() {
        let p = problems("[common]\nenv.A = \"1\"\nvar.A = \"2\"\n");
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("both env and var"), "{p:?}");
        // In different tables it's an ordinary override.
        assert!(problems("[common]\nenv.A = \"1\"\n[sh]\nvar.A = \"2\"\n").is_empty());
    }
}
