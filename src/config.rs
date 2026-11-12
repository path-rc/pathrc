//! The `.pathrc.toml` format (RFC 0002): settings, not shell code.
//!
//! ```toml
//! [common]                      # every shell
//! env.AWS_REGION = "eu-west-2"
//! path.prepend = ["bin"]        # relative to this file's directory
//! alias.tfp = "terraform plan"
//!
//! [sh]                          # bash and zsh; then [bash], [zsh], [fish]
//! unset.env = ["AWS_PROFILE"]
//! ```
//!
//! A file is parsed and checked once, then [`Config::layer`] merges the
//! tables a shell reads into the settings that file contributes.

use crate::shell::Shell;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
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
    alias: BTreeMap<Text, Text>,
    path: Option<PathTable>,
    #[serde(default)]
    unset: Unset,
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
    alias: Vec<Text>,
    #[serde(default)]
    all: bool,
}

impl Drop for Table {
    // Values may be secrets from a decrypted file.
    fn drop(&mut self) {
        for value in self.env.values_mut().chain(self.alias.values_mut()) {
            value.as_mut().zeroize();
        }
    }
}

/// A parsed and checked `.pathrc.toml`.
pub struct Config {
    document: Document,
    dir: PathBuf,
}

/// What one file contributes for one shell: its tables merged, a later
/// table's key overriding an earlier one's.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Layer {
    pub unset_all: bool,
    pub unset_env: BTreeSet<String>,
    pub unset_alias: BTreeSet<String>,
    pub env: BTreeMap<String, String>,
    /// Absolute: relative entries are resolved against the file's directory.
    pub path: Vec<PathBuf>,
    pub alias: BTreeMap<String, String>,
}

impl Drop for Layer {
    fn drop(&mut self) {
        for value in self.env.values_mut().chain(self.alias.values_mut()) {
            value.zeroize();
        }
    }
}

impl Config {
    /// The tables `shell` reads, in order: `[common]`, then `[sh]` for bash,
    /// zsh and sh, then the shell's own.
    fn tables(&self, shell: Shell) -> Vec<&Table> {
        let d = &self.document;
        match shell {
            Shell::Sh => vec![&d.common, &d.sh],
            Shell::Bash => vec![&d.common, &d.sh, &d.bash],
            Shell::Zsh => vec![&d.common, &d.sh, &d.zsh],
            Shell::Fish => vec![&d.common, &d.fish],
        }
    }

    pub fn layer(&self, shell: Shell) -> Layer {
        let mut layer = Layer::default();
        for table in self.tables(shell) {
            for (name, value) in &table.env {
                layer
                    .env
                    .insert(name.get_ref().clone(), value.get_ref().clone());
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
                    .map(|dir| resolve(&self.dir, dir.get_ref()))
                    .collect();
            }
            layer
                .unset_env
                .extend(table.unset.env.iter().map(|n| n.get_ref().clone()));
            layer
                .unset_alias
                .extend(table.unset.alias.iter().map(|n| n.get_ref().clone()));
            layer.unset_all |= table.unset.all;
        }
        layer
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

    fn table(&mut self, table: &Table) {
        for (name, value) in &table.env {
            self.env_name(name, "env.");
            self.no_nul(value);
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
        }
    }
}

/// Parses and checks a file's text. `dir` is the file's directory, for
/// relative `path.prepend` entries.
pub fn parse(text: &str, dir: &Path) -> Result<Config, Vec<Problem>> {
    let mut checker = Checker {
        text,
        problems: Vec::new(),
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
    for table in [
        &document.common,
        &document.sh,
        &document.bash,
        &document.zsh,
        &document.fish,
    ] {
        checker.table(table);
    }
    if checker.problems.is_empty() {
        Ok(Config {
            document,
            dir: dir.to_path_buf(),
        })
    } else {
        checker.problems.sort_by_key(|p| (p.line, p.column));
        Err(checker.problems)
    }
}

/// The problems in a file's bytes, for `list`, `edit` and `encrypt`.
pub fn validate(bytes: &[u8]) -> Vec<Problem> {
    match std::str::from_utf8(bytes) {
        Err(error) => vec![Problem {
            line: 0,
            column: 0,
            message: format!("not UTF-8 text ({error})"),
        }],
        Ok(text) => parse(text, Path::new("/")).err().unwrap_or_default(),
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
            .layer(shell)
    }

    fn strings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn bash_reads_common_sh_and_bash_with_later_keys_winning() {
        let l = layer(FILE, Shell::Bash);
        assert_eq!(l.env, strings(&[("A", "bash"), ("B", "sh")]));
        assert_eq!(l.alias, strings(&[("ll", "ls -l")]));
        assert_eq!(
            l.path,
            vec![PathBuf::from("/proj/bin"), PathBuf::from("/opt/tool/bin")]
        );
        assert_eq!(l.unset_env, set(&["GONE"]));
        assert_eq!(l.unset_alias, set(&["old"]));
        assert!(!l.unset_all);
    }

    #[test]
    fn zsh_reads_sh_and_its_own_path_list_replaces_commons() {
        let l = layer(FILE, Shell::Zsh);
        assert_eq!(l.env, strings(&[("A", "common"), ("B", "sh")]));
        assert_eq!(l.path, vec![PathBuf::from("/proj/zbin")]);
    }

    #[test]
    fn fish_skips_sh() {
        let l = layer(FILE, Shell::Fish);
        assert_eq!(l.env, strings(&[("A", "common"), ("B", "common")]));
        assert_eq!(l.alias, strings(&[("ll", "ls -lh")]));
        assert!(l.unset_alias.is_empty());
        assert!(l.unset_all);
    }

    #[test]
    fn shared_sh_reads_common_and_sh() {
        let l = layer(FILE, Shell::Sh);
        assert_eq!(l.env, strings(&[("A", "common"), ("B", "sh")]));
    }

    #[test]
    fn an_empty_file_is_an_empty_layer() {
        assert_eq!(layer("", Shell::Bash), Layer::default());
        assert_eq!(layer("# just a comment\n", Shell::Fish), Layer::default());
    }

    #[test]
    fn values_are_not_expanded() {
        let l = layer("[common]\nenv.X = \"$HOME/x\"\n", Shell::Bash);
        assert_eq!(l.env["X"], "$HOME/x");
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
}
