//! `pathrc migrate`: older pathrc files into a `.pathrc.toml` (RFC 0002
//! section 6, RFC 0003).
//!
//! - `.pathrc` and `.pathrc.age` (pathrc 0.1): shell code in `#sh`, `#bash`,
//!   `#zsh` and `#fish` sections. Only lines that are plainly settings are
//!   understood: assignments, `alias`, `unset`, `unalias`, fish's `set`,
//!   `alias` and `functions -e`, and prepending to `PATH`. Anything else
//!   (a function, a condition, a command, `$` expansion) stops that file's
//!   conversion with its line numbers.
//! - `.pathrc.toml.age` (pathrc 0.3 to 0.5): a `.pathrc.toml`, encrypted as
//!   a whole.
//!
//! From an encrypted file, `env` and `var` values go into `.enc` tables,
//! each encrypted on its own; aliases, `PATH` entries and unsets stay plain.
//! The result is merged into the directory's `.pathrc.toml`, refusing any
//! key that is already there. Old files are left where they are.

use crate::config::{describe, Problem};
use crate::secrets::{sub_table, Make, TABLES};
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, Item, TableLike, Value};
use zeroize::Zeroizing;

/// The settings of one table, in the order they were found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub env: Vec<(String, String)>,
    pub var: Vec<(String, String)>,
    pub alias: Vec<(String, String)>,
    pub path: Vec<String>,
    pub unset_env: Vec<String>,
    pub unset_alias: Vec<String>,
    pub unset_all: bool,
}

impl Drop for Section {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for (_, value) in self.env.iter_mut().chain(self.var.iter_mut()) {
            value.zeroize();
        }
    }
}

/// A converted file: its settings per table (`common`, `sh`, ...).
pub type Converted = BTreeMap<&'static str, Section>;

fn problem(line: usize, message: impl Into<String>) -> Problem {
    Problem {
        line,
        column: 0,
        message: message.into(),
    }
}

/// The words of one line of shell code, quotes removed, if it is simple
/// enough to read without running it. `$PATH` (or `${PATH}`) is kept as a
/// word of its own, `\u{0}PATH`, so PATH lines can be recognised.
fn words(line: &str) -> Result<Vec<String>, String> {
    const PATH_REF: &str = "\u{0}PATH";
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '#' if !in_word => break,
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("an unclosed single quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err("an unclosed double quote".into()),
                        },
                        Some('$') | Some('`') => {
                            return Err(
                                "a value that expands ($ or `); pathrc values are used as written"
                                    .into(),
                            )
                        }
                        Some(c) => word.push(c),
                        None => return Err("an unclosed double quote".into()),
                    }
                }
            }
            '$' => {
                let rest: String = chars.clone().collect();
                let len = if rest.starts_with("PATH")
                    && !rest[4..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
                {
                    4
                } else if rest.starts_with("{PATH}") {
                    6
                } else {
                    return Err(
                        "a value that expands ($); pathrc values are used as written".into(),
                    );
                };
                for _ in 0..len {
                    chars.next();
                }
                in_word = true;
                word.push_str(PATH_REF);
            }
            '\\' => match chars.next() {
                Some(c) => {
                    in_word = true;
                    word.push(c);
                }
                None => return Err("a line continuation".into()),
            },
            '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '*' | '?' | '[' => {
                return Err(format!("shell syntax ('{c}') that only a shell can run"))
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        out.push(word);
    }
    Ok(out)
}

fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `dir:dir:$PATH` (or `$PATH` last among fish's words) as the directories
/// put in front of PATH.
fn prepended(parts: &[&str]) -> Result<Vec<String>, String> {
    match parts.split_last() {
        Some((&"\u{0}PATH", dirs)) if !dirs.is_empty() => {
            if dirs.iter().any(|d| d.is_empty() || d.contains('\u{0}')) {
                return Err("PATH set in a way other than prepending directories".into());
            }
            Ok(dirs.iter().map(|d| d.to_string()).collect())
        }
        _ => Err("PATH set in a way other than prepending directories to $PATH".into()),
    }
}

fn assignment(section: &mut Section, word: &str, exported: bool) -> Result<(), String> {
    let (name, value) = word
        .split_once('=')
        .ok_or_else(|| format!("'{word}' is not an assignment"))?;
    if !is_name(name) {
        return Err(format!("'{name}' is not a variable name"));
    }
    if name == "PATH" {
        let parts: Vec<&str> = value.split(':').collect();
        section.path.extend(prepended(&parts)?);
        return Ok(());
    }
    if value.contains('\u{0}') {
        return Err("a value that expands ($PATH)".into());
    }
    let list = if exported {
        &mut section.env
    } else {
        &mut section.var
    };
    list.push((name.to_string(), value.to_string()));
    Ok(())
}

/// One line of a shell section into `section`.
fn shell_line(section: &mut Section, words: &[String]) -> Result<(), String> {
    let Some((command, args)) = words.split_first() else {
        return Ok(());
    };
    match command.as_str() {
        "export" => {
            if args.is_empty() {
                return Err("export without an assignment".into());
            }
            for word in args {
                if word.starts_with('-') || !word.contains('=') {
                    return Err(format!(
                        "export {word}: only `export NAME=value` is understood"
                    ));
                }
                assignment(section, word, true)?;
            }
        }
        "alias" => {
            if args.len() >= 2 && !args[0].contains('=') {
                // fish: alias NAME VALUE...
                section.alias.push((args[0].clone(), args[1..].join(" ")));
            } else {
                for word in args {
                    let (name, value) = word.split_once('=').ok_or_else(|| {
                        format!("alias {word}: only `alias NAME=value` is understood")
                    })?;
                    section.alias.push((name.to_string(), value.to_string()));
                }
            }
        }
        "unset" => {
            for name in args {
                if name.starts_with('-') {
                    return Err(format!("unset {name}: only `unset NAME` is understood"));
                }
                section.unset_env.push(name.clone());
            }
        }
        "unalias" => section.unset_alias.extend(args.iter().cloned()),
        "functions" if args.first().is_some_and(|a| a == "-e" || a == "--erase") => {
            section.unset_alias.extend(args[1..].iter().cloned());
        }
        "set" => {
            let flags: String = args
                .iter()
                .take_while(|a| a.starts_with('-'))
                .map(|a| a.trim_start_matches('-'))
                .collect();
            let rest = &args[args.iter().take_while(|a| a.starts_with('-')).count()..];
            let Some((name, values)) = rest.split_first() else {
                return Err("set without a name".into());
            };
            if flags.contains('e') {
                section.unset_env.extend(rest.iter().cloned());
            } else if !flags.chars().all(|c| "gxuU".contains(c)) || flags.contains('U') {
                return Err(format!(
                    "set -{flags}: only -g, -x, -gx and -e are understood"
                ));
            } else if name == "PATH" {
                let parts: Vec<&str> = values.iter().map(|v| v.as_str()).collect();
                section.path.extend(prepended(&parts)?);
            } else {
                if !is_name(name) {
                    return Err(format!("'{name}' is not a variable name"));
                }
                let value = values.join(" ");
                if value.contains('\u{0}') {
                    return Err("a value that expands ($PATH)".into());
                }
                let exported = flags.contains('x') && !flags.contains('u');
                let list = if exported {
                    &mut section.env
                } else {
                    &mut section.var
                };
                list.push((name.clone(), value));
            }
        }
        word if word.contains('=') => {
            for word in words {
                if !word.contains('=') {
                    return Err(format!("'{word}' after an assignment runs a command"));
                }
                assignment(section, word, false)?;
            }
        }
        other => return Err(format!("'{other}' is a command, not a setting")),
    }
    Ok(())
}

/// The marker-section shell code of pathrc 0.1.
pub fn from_shell(text: &str) -> Result<Converted, Vec<Problem>> {
    let mut converted = Converted::new();
    let mut table = "common";
    let mut problems = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = line.trim();
        let marker = match trimmed {
            "#sh" => Some("sh"),
            "#bash" => Some("bash"),
            "#zsh" => Some("zsh"),
            "#fish" => Some("fish"),
            _ => None,
        };
        if let Some(marker) = marker {
            table = marker;
            continue;
        }
        let section = converted.entry(table).or_default();
        let result = words(trimmed).and_then(|words| shell_line(section, &words));
        if let Err(message) = result {
            problems.push(problem(line_no, message));
        }
    }
    if problems.is_empty() {
        Ok(converted)
    } else {
        Err(problems)
    }
}

/// The `.pathrc.toml` text of pathrc 0.3 to 0.5, decrypted.
pub fn from_toml(text: &str) -> Result<Converted, Vec<Problem>> {
    crate::config::parse(text, std::path::Path::new("/"))?;
    let doc: toml::Table =
        toml::from_str(text).map_err(|e| vec![problem(0, e.message().to_string())])?;
    let mut converted = Converted::new();
    let strings = |item: Option<&toml::Value>| -> Vec<(String, String)> {
        item.and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let list = |item: Option<&toml::Value>| -> Vec<String> {
        item.and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    for table in TABLES {
        let Some(t) = doc.get(table).and_then(|t| t.as_table()) else {
            continue;
        };
        let unset = t.get("unset").and_then(|u| u.as_table());
        let section = Section {
            env: strings(t.get("env")),
            var: strings(t.get("var")),
            alias: strings(t.get("alias")),
            path: list(t.get("path").and_then(|p| p.get("prepend"))),
            unset_env: list(unset.and_then(|u| u.get("env")))
                .into_iter()
                .chain(list(unset.and_then(|u| u.get("var"))))
                .collect(),
            unset_alias: list(unset.and_then(|u| u.get("alias"))),
            unset_all: unset
                .and_then(|u| u.get("all"))
                .and_then(|a| a.as_bool())
                .unwrap_or(false),
        };
        converted.insert(table, section);
    }
    Ok(converted)
}

/// Whether `table` already has `kind.name`, plain or in its `.enc`.
fn has(doc: &DocumentMut, table: &str, kind: &str, name: &str) -> bool {
    let Some(t) = doc.get(table).and_then(|t| t.as_table_like()) else {
        return false;
    };
    let plain = t
        .get(kind)
        .and_then(|k| k.as_table_like())
        .is_some_and(|k| k.contains_key(name));
    let enc = ["env", "var"].contains(&kind)
        && t.get("enc")
            .and_then(|e| e.as_table_like())
            .and_then(|e| e.get(kind))
            .and_then(|k| k.as_table_like())
            .is_some_and(|k| k.contains_key(name));
    plain || enc
}

/// The table at `[table]` (or `[table.enc]`), made if missing.
fn table_mut<'a>(
    doc: &'a mut DocumentMut,
    table: &str,
    enc: bool,
) -> Result<&'a mut dyn TableLike, String> {
    let root = doc.as_table_mut() as &mut dyn TableLike;
    let shell = sub_table(root, table, if enc { Make::Implicit } else { Make::Header })?;
    if enc {
        sub_table(shell, "enc", Make::Header)
    } else {
        Ok(shell)
    }
}

fn put(t: &mut dyn TableLike, kind: &str, name: &str, value: &str) -> Result<(), String> {
    let values = sub_table(t, kind, Make::Dotted)?;
    values.insert(name, Item::Value(Value::from(value)));
    Ok(())
}

/// Adds names to the `unset.kind` list, skipping ones already there.
fn put_unset(t: &mut dyn TableLike, kind: &str, names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    let unset = sub_table(t, "unset", Make::Dotted)?;
    let item = unset
        .entry(kind)
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let array = item
        .as_array_mut()
        .ok_or_else(|| format!("unset.{kind} is not a list"))?;
    for name in names {
        if !array.iter().any(|v| v.as_str() == Some(name)) {
            array.push(name.as_str());
        }
    }
    Ok(())
}

/// `target` (the directory's `.pathrc.toml`, or empty) with `converted`
/// merged in. Values from an encrypted file go through `seal` into `.enc`
/// tables. A key already in `target` is a conflict: nothing is merged.
pub fn merge(
    target: &str,
    converted: &[(Converted, bool)],
    seal: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let mut doc = target.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    let mut conflicts = Vec::new();
    for (sections, _) in converted {
        for (table, s) in sections {
            for (kind, names) in [
                ("env", s.env.iter().map(|(n, _)| n).collect::<Vec<_>>()),
                ("var", s.var.iter().map(|(n, _)| n).collect()),
                ("alias", s.alias.iter().map(|(n, _)| n).collect()),
            ] {
                for name in names {
                    if has(&doc, table, kind, name) {
                        conflicts.push(format!("[{table}] {kind}.{name}"));
                    }
                }
            }
            let has_path = doc.get(table).and_then(|t| t.get("path")).is_some();
            if !s.path.is_empty() && has_path {
                conflicts.push(format!("[{table}] path.prepend"));
            }
        }
    }
    if !conflicts.is_empty() {
        conflicts.sort();
        conflicts.dedup();
        return Err(format!(
            "already set in .pathrc.toml: {}; nothing converted",
            conflicts.join(", ")
        ));
    }
    let mut count = 0;
    for (sections, encrypted) in converted {
        for (table, s) in sections {
            for (kind, values) in [("env", &s.env), ("var", &s.var)] {
                for (name, value) in values.iter() {
                    let t = table_mut(&mut doc, table, *encrypted)?;
                    if *encrypted {
                        let sealed = Zeroizing::new(seal(value)?);
                        put(t, kind, name, &sealed)?;
                    } else {
                        put(t, kind, name, value)?;
                    }
                    count += 1;
                }
            }
            let plain = !s.alias.is_empty()
                || !s.path.is_empty()
                || !s.unset_env.is_empty()
                || !s.unset_alias.is_empty()
                || s.unset_all;
            if !plain {
                continue;
            }
            let t = table_mut(&mut doc, table, false)?;
            for (name, value) in &s.alias {
                put(t, "alias", name, value)?;
                count += 1;
            }
            if !s.path.is_empty() {
                let path = sub_table(t, "path", Make::Dotted)?;
                let array: Array = s.path.iter().map(|d| d.as_str()).collect();
                path.insert("prepend", Item::Value(Value::Array(array)));
                count += s.path.len();
            }
            put_unset(t, "env", &s.unset_env)?;
            put_unset(t, "alias", &s.unset_alias)?;
            count += s.unset_env.len() + s.unset_alias.len();
            if s.unset_all {
                let unset = sub_table(t, "unset", Make::Dotted)?;
                unset.insert("all", Item::Value(Value::from(true)));
                count += 1;
            }
        }
    }
    Ok((doc.to_string(), count))
}

/// One line per problem, for the report.
pub fn report(problems: &[Problem]) -> String {
    describe(problems)
}

/// Converts the old files in each of `dirs` (default: the current
/// directory) into its `.pathrc.toml`.
pub fn handle_migrate(dirs: &[std::path::PathBuf]) -> std::process::ExitCode {
    use crate::manage::{read, write_atomic};
    use crate::trust::{Store, Trust};
    let dirs = if dirs.is_empty() {
        match crate::common::current_dir() {
            Ok(dir) => vec![dir],
            Err(message) => {
                eprintln!("pathrc: {message}");
                return std::process::ExitCode::FAILURE;
            }
        }
    } else {
        dirs.to_vec()
    };
    let mut identities: Option<Result<crate::crypto::Identities, String>> = None;
    let mut ok = true;
    for dir in &dirs {
        match migrate_dir(dir, &mut identities) {
            Ok(message) => println!("{message}"),
            Err(message) => {
                eprintln!("pathrc: {message}");
                ok = false;
            }
        }
    }
    return if ok {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    };

    fn migrate_dir(
        dir: &std::path::Path,
        identities: &mut Option<Result<crate::crypto::Identities, String>>,
    ) -> Result<String, String> {
        let store = Store::load()?;
        let mut converted = Vec::new();
        let mut sources = Vec::new();
        let mut all_allowed = true;
        let mut problems = Vec::new();
        for name in crate::files::OLD {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let bytes = read(&path)?;
            all_allowed &= store.check(&path, &bytes) == Trust::Allowed;
            let encrypted = name.ends_with(".age");
            let plain = if encrypted {
                let ids = identities
                    .get_or_insert_with(crate::crypto::Identities::load)
                    .as_ref()
                    .map_err(|e| format!("{}: cannot decrypt: {e}", path.display()))?;
                ids.decrypt(&bytes)
                    .map_err(|e| format!("{}: cannot decrypt: {e}", path.display()))?
            } else {
                Zeroizing::new(bytes)
            };
            let text = std::str::from_utf8(&plain)
                .map_err(|_| format!("{}: not UTF-8 text", path.display()))?;
            let result = if name == ".pathrc.toml.age" {
                from_toml(text)
            } else {
                from_shell(text)
            };
            match result {
                Ok(c) => converted.push((c, encrypted)),
                Err(p) => problems.push(format!("{}: {}", path.display(), report(&p))),
            }
            sources.push(path);
        }
        if sources.is_empty() {
            return Ok(format!("{}: no old pathrc files", dir.display()));
        }
        if !problems.is_empty() {
            return Err(format!(
                "{}\nnothing converted in {}; change or remove those lines, then run pathrc migrate again",
                problems.join("\n"),
                dir.display()
            ));
        }
        let target = dir.join(crate::files::PLAIN);
        let existed = target.is_file();
        let before = if existed { read(&target)? } else { Vec::new() };
        let before_text = String::from_utf8(before.clone())
            .map_err(|_| format!("{}: not UTF-8 text", target.display()))?;
        all_allowed &= !existed || store.check(&target, &before) == Trust::Allowed;
        let mut recipients = None;
        let (text, count) = merge(&before_text, &converted, &mut |value| {
            if recipients.is_none() {
                recipients = Some(crate::crypto::recipients_for(dir)?);
            }
            let (list, _) = recipients.as_ref().expect("just set");
            crate::crypto::seal_value(value.as_bytes(), list)
        })
        .map_err(|e| format!("{}: {e}", target.display()))?;
        let problems = crate::config::validate(text.as_bytes());
        if !problems.is_empty() {
            return Err(format!(
                "{}: {}; nothing converted",
                target.display(),
                report(&problems)
            ));
        }
        write_atomic(&target, text.as_bytes(), 0o644)?;
        let mut store = Store::load()?;
        if all_allowed {
            store.allow(&target, text.as_bytes());
            store.save()?;
        }
        let names: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let list = match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            _ => names.join(""),
        };
        let mut message = format!(
            "converted {list} into {} ({count} setting{})",
            target.display(),
            if count == 1 { "" } else { "s" }
        );
        if !all_allowed {
            message.push_str(&format!(
                "\n{} is not allowed yet: not every file it came from was; check it, then run 'pathrc allow'",
                target.display()
            ));
        }
        message.push_str(&format!(
            "\nthe old file{} can be deleted: {}",
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        ));
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(text: &str, table: &str) -> Section {
        let mut c = from_shell(text).map_err(|p| report(&p)).unwrap();
        c.remove(table).unwrap_or_default()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn shell_settings_by_section() {
        let text = "\
# a comment
export REGION=eu PROJECT='my project'
STAGE=\"dev\"
alias ll='ls -l'
export PATH=/opt/tool/bin:$PATH
#sh
unset AWS_PROFILE
unalias old
#fish
set -gx FISH_E 'one two'
set -g FISH_V v
set -e GONE
alias fl 'ls -l'
set -gx PATH /opt/a /opt/b $PATH
functions -e oldf
";
        let common = section(text, "common");
        assert_eq!(
            common.env,
            pairs(&[("REGION", "eu"), ("PROJECT", "my project")])
        );
        assert_eq!(common.var, pairs(&[("STAGE", "dev")]));
        assert_eq!(common.alias, pairs(&[("ll", "ls -l")]));
        assert_eq!(common.path, ["/opt/tool/bin"]);
        let sh = section(text, "sh");
        assert_eq!(sh.unset_env, ["AWS_PROFILE"]);
        assert_eq!(sh.unset_alias, ["old"]);
        let fish = section(text, "fish");
        assert_eq!(fish.env, pairs(&[("FISH_E", "one two")]));
        assert_eq!(fish.var, pairs(&[("FISH_V", "v")]));
        assert_eq!(fish.unset_env, ["GONE"]);
        assert_eq!(fish.alias, pairs(&[("fl", "ls -l")]));
        assert_eq!(fish.path, ["/opt/a", "/opt/b"]);
        assert_eq!(fish.unset_alias, ["oldf"]);
    }

    #[test]
    fn anything_a_shell_must_run_is_refused_with_its_line() {
        let text = "\
export A=1
export B=\"$HOME/x\"
mkdir -p build
f() { echo hi; }
if true; then export C=1; fi
export PATH=$PATH:/after
";
        let problems: Vec<String> = from_shell(text)
            .unwrap_err()
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(
            problems[0].starts_with("line 2: a value that expands"),
            "{problems:?}"
        );
        assert!(
            problems[1].starts_with("line 3: 'mkdir' is a command"),
            "{problems:?}"
        );
        assert!(problems[2].starts_with("line 4:"), "{problems:?}");
        assert!(problems[3].starts_with("line 5:"), "{problems:?}");
        assert!(
            problems[4].starts_with("line 6: PATH set in a way"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_toml_age_file_reads_like_its_tables() {
        let text = "[common]\nenv.A = \"a\"\nalias.k = \"x\"\npath.prepend = [\"bin\"]\nunset.all = true\n[zsh]\nvar.Z = \"z\"\n";
        let c = from_toml(text).unwrap();
        assert_eq!(c["common"].env, pairs(&[("A", "a")]));
        assert_eq!(c["common"].path, ["bin"]);
        assert!(c["common"].unset_all);
        assert_eq!(c["zsh"].var, pairs(&[("Z", "z")]));
    }

    #[test]
    fn merging_puts_secrets_in_enc_and_refuses_conflicts() {
        let plain = from_shell("export A=1\nalias k='x'\n#bash\nunset G\n").unwrap();
        let secret = from_shell("export TOKEN=t\nS=s\n").unwrap();
        let (text, count) = merge(
            "# mine\n[common]\nenv.KEEP = \"k\"\n",
            &[(plain, false), (secret, true)],
            &mut |v| Ok(format!("SEALED({v})")),
        )
        .unwrap();
        assert_eq!(count, 5);
        assert!(
            text.starts_with("# mine\n[common]\nenv.KEEP = \"k\"\n"),
            "{text}"
        );
        assert!(text.contains("env.A = \"1\""), "{text}");
        assert!(text.contains("alias.k = \"x\""), "{text}");
        assert!(
            text.contains("[common.enc]\nenv.TOKEN = \"SEALED(t)\"\nvar.S = \"SEALED(s)\""),
            "{text}"
        );
        assert!(text.contains("[bash]\nunset.env = [\"G\"]"), "{text}");

        let again = from_shell("export KEEP=other\n").unwrap();
        let error = merge(
            "[common]\nenv.KEEP = \"k\"\n",
            &[(again, false)],
            &mut |v| Ok(v.into()),
        )
        .unwrap_err();
        assert!(error.contains("[common] env.KEEP"), "{error}");
    }
}
