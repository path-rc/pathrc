//! `$NAME` references in double-quoted values (RFC 0002, revised
//! 2026-11-16).
//!
//! A double-quoted (TOML basic string) plain `env`, `var` or `path.prepend`
//! value may refer to variables as `$NAME` or `${NAME}`; `$$` is a literal
//! `$`. Nothing else with a `$` is allowed, so a value can never turn into
//! a command substitution or a positional parameter. Single-quoted values
//! and `.enc` values are never looked at.

use crate::quote::quote;
use crate::shell::Shell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Ref(String),
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The parts of `value`, or what is wrong with its `$`s.
pub fn parse(value: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = value.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c != '$' {
            text.push(c);
            continue;
        }
        let rest = &value[at + 1..];
        let name: String = if rest.starts_with('$') {
            chars.next();
            text.push('$');
            continue;
        } else if let Some(braced) = rest.strip_prefix('{') {
            let name: String = braced.chars().take_while(|c| name_char(*c)).collect();
            if name.is_empty() || !braced[name.len()..].starts_with('}') {
                return Err(bad(value));
            }
            for _ in 0..name.len() + 2 {
                chars.next();
            }
            name
        } else {
            let name: String = rest.chars().take_while(|c| name_char(*c)).collect();
            for _ in 0..name.len() {
                chars.next();
            }
            name
        };
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(bad(value));
        }
        if !text.is_empty() {
            parts.push(Part::Text(std::mem::take(&mut text)));
        }
        parts.push(Part::Ref(name));
    }
    if !text.is_empty() || parts.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

fn bad(value: &str) -> String {
    format!(
        "'{value}': a $ must start $NAME or ${{NAME}} (or be $$ for a $); \
         use single quotes for a value used exactly as written"
    )
}

/// The names `parts` refer to.
pub fn refs(parts: &[Part]) -> impl Iterator<Item = &str> {
    parts.iter().filter_map(|p| match p {
        Part::Ref(name) => Some(name.as_str()),
        Part::Text(_) => None,
    })
}

/// `parts` with each reference looked up; a name `lookup` can't answer is
/// an error.
pub fn substitute(
    parts: &[Part],
    lookup: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::new();
    for part in parts {
        match part {
            Part::Text(text) => out.push_str(text),
            Part::Ref(name) => out.push_str(&lookup(name)?),
        }
    }
    Ok(out)
}

/// Shell code for the value, expanding its references when the shell runs
/// it and nothing else: POSIX shells get one double-quoted word with
/// `\`, `"`, `$` and `` ` `` escaped and each reference as `${NAME-}` (empty
/// when unset, also under `set -u`); fish gets double-quoted pieces side by
/// side, each reference as `"$NAME"` (empty when unset).
pub fn render(parts: &[Part], shell: Shell) -> String {
    match shell {
        Shell::Fish => {
            let pieces: Vec<String> = parts
                .iter()
                .filter(|p| !matches!(p, Part::Text(t) if t.is_empty()))
                .map(|part| match part {
                    Part::Text(text) => {
                        let escaped = text
                            .replace('\\', "\\\\")
                            .replace('"', "\\\"")
                            .replace('$', "\\$");
                        format!("\"{escaped}\"")
                    }
                    Part::Ref(name) => format!("\"${name}\""),
                })
                .collect();
            if pieces.is_empty() {
                quote("", shell)
            } else {
                pieces.join("")
            }
        }
        _ => {
            let mut out = String::from("\"");
            for part in parts {
                match part {
                    Part::Text(text) => {
                        for c in text.chars() {
                            if matches!(c, '\\' | '"' | '$' | '`') {
                                out.push('\\');
                            }
                            out.push(c);
                        }
                    }
                    Part::Ref(name) => out.push_str(&format!("${{{name}-}}")),
                }
            }
            out.push('"');
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Part {
        Part::Ref(s.to_string())
    }

    fn t(s: &str) -> Part {
        Part::Text(s.to_string())
    }

    #[test]
    fn references_and_text() {
        assert_eq!(parse("$HOME/x").unwrap(), [r("HOME"), t("/x")]);
        assert_eq!(
            parse("a${B}c$D_1").unwrap(),
            [t("a"), r("B"), t("c"), r("D_1")]
        );
        assert_eq!(parse("cost: $$5").unwrap(), [t("cost: $5")]);
        assert_eq!(parse("plain").unwrap(), [t("plain")]);
        assert_eq!(parse("").unwrap(), [t("")]);
    }

    #[test]
    fn anything_else_with_a_dollar_is_refused() {
        for value in [
            "$(id)", "a$", "$1", "${}", "${A", "${A:-x}", "$?", "x $ y", "$@",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn posix_rendering_escapes_everything_but_references() {
        let parts = parse("$HOME/a \"b\" `c` \\d $$e").unwrap();
        assert_eq!(
            render(&parts, Shell::Bash),
            "\"${HOME-}/a \\\"b\\\" \\`c\\` \\\\d \\$e\""
        );
    }

    #[test]
    fn fish_rendering_quotes_each_piece() {
        let parts = parse("$HOME/a\"$$").unwrap();
        assert_eq!(render(&parts, Shell::Fish), "\"$HOME\"\"/a\\\"\\$\"");
        assert_eq!(render(&parse("").unwrap(), Shell::Fish), "''");
    }

    #[test]
    fn substitution_looks_each_name_up() {
        let parts = parse("$A-${B}").unwrap();
        let out = substitute(&parts, &mut |n| Ok(n.to_lowercase())).unwrap();
        assert_eq!(out, "a-b");
        assert!(substitute(&parts, &mut |n| Err(format!("no {n}"))).is_err());
    }
}
