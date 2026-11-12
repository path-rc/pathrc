//! `pathrc _undo`: what a file changed, as shell code that changes it back.
//!
//! The init scripts snapshot the shell's variables, aliases and functions
//! before and after evaluating each file (`__pathrc_dump`) and pipe both
//! snapshots here, on stdin. The undo script is kept in the shell and run
//! when the file goes out of scope. Snapshots are the shell's own output,
//! `declare -p`/`alias -p`/`declare -f` in bash and `typeset -p`/`alias -L`/
//! `typeset -f` in zsh, so restoring a value is replaying its old line.
//!
//! The undo runs inside a shell function (the `cd` hook), so it must never
//! create locals: bash lines are rewritten as plain assignments, and zsh's
//! own `typeset -p` already prints `typeset -g` there.

use crate::shell::Shell;
use std::collections::BTreeMap;

/// The snapshot sections are separated by lines `<mark>after`,
/// `<mark>aliases` and `<mark>functions`, where the shell picks a random
/// `<mark>` for each load and passes it as an argument. A marker counts only
/// as a whole line, so a variable whose value happens to contain one (say
/// `BASH_EXECUTION_STRING`) can't split a snapshot in the wrong place.
fn split<'a>(text: &'a str, marker: &str) -> (&'a str, &'a str) {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches('\n') == marker {
            return (&text[..offset], &text[offset + line.len()..]);
        }
        offset += line.len();
    }
    (text, "")
}

/// Names that change on their own, or belong to pathrc, and are never undone.
fn volatile(name: &str) -> bool {
    name.starts_with("__pathrc")
        || name.starts_with("BASH")
        || matches!(
            name,
            "_" | "PATHRC_LOADED"
                | "PWD"
                | "OLDPWD"
                | "PIPESTATUS"
                | "FUNCNAME"
                | "LINENO"
                | "RANDOM"
                | "SRANDOM"
                | "SECONDS"
                | "EPOCHREALTIME"
                | "EPOCHSECONDS"
                | "HISTCMD"
                | "COLUMNS"
                | "LINES"
                | "DIRSTACK"
                | "GROUPS"
                | "REPLY"
                | "MAPFILE"
                | "cd"
                | "pathrc_after_cd"
        )
}

#[derive(Debug, Default, PartialEq)]
struct Snapshot {
    vars: BTreeMap<String, Var>,
    aliases: BTreeMap<String, Alias>,
    functions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Var {
    /// The whole declaration, as printed.
    line: String,
    flags: String,
    exported: bool,
    /// The text after `NAME=`, still quoted; `None` for declared-but-unset.
    value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Alias {
    line: String,
    suffix: bool,
}

/// Splits `text` into entries, each starting at a line for which `starts`
/// is true; other lines continue the entry before (multi-line values).
fn entries(text: &str, starts: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if starts(line) || out.is_empty() {
            out.push(line.to_string());
        } else if let Some(last) = out.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    out.retain(|entry| !entry.trim().is_empty());
    out
}

/// `declare -x NAME="v"`, `typeset -g -a NAME=( … )`, `export NAME=v`.
fn parse_var(entry: &str) -> Option<(String, Var)> {
    let mut rest = entry;
    let command = rest.split_whitespace().next()?;
    if !matches!(command, "declare" | "typeset" | "export") {
        return None;
    }
    rest = rest[command.len()..].trim_start();
    let mut flags = String::new();
    while rest.starts_with('-') {
        let end = rest.find(' ').unwrap_or(rest.len());
        flags.push_str(rest[1..end].trim_start_matches('-'));
        rest = rest[end..].trim_start();
    }
    let (name, value) = match rest.find('=') {
        Some(eq) if !rest[..eq].contains(char::is_whitespace) => {
            (rest[..eq].to_string(), Some(rest[eq + 1..].to_string()))
        }
        _ => (rest.trim().to_string(), None),
    };
    if name.is_empty() || flags.contains('r') {
        return None;
    }
    let exported = command == "export" || flags.contains('x');
    Some((
        name,
        Var {
            line: entry.to_string(),
            flags,
            exported,
            value,
        },
    ))
}

/// `alias ll='ls -l'`, `alias -g G='| grep'`, `alias -s txt=less`.
fn parse_alias(entry: &str) -> Option<(String, Alias)> {
    let mut rest = entry.strip_prefix("alias ")?;
    let mut suffix = false;
    while rest.starts_with('-') {
        let end = rest.find(' ')?;
        suffix |= rest[..end].contains('s');
        rest = &rest[end + 1..];
    }
    let name = &rest[..rest.find('=')?];
    Some((
        name.to_string(),
        Alias {
            line: entry.to_string(),
            suffix,
        },
    ))
}

/// A function header: bash prints `name () `, zsh `name () {`.
fn function_name(line: &str) -> Option<&str> {
    let (name, rest) = line.split_once(" () ")?;
    let rest = rest.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace) && (rest.is_empty() || rest == "{"))
        .then_some(name)
}

fn parse(dump: &str, mark: &str) -> Snapshot {
    let (vars, rest) = split(dump, &format!("{mark}aliases"));
    let (aliases, functions) = split(rest, &format!("{mark}functions"));
    let var_start = |l: &str| {
        l.starts_with("declare ") || l.starts_with("typeset ") || l.starts_with("export ")
    };
    let mut snapshot = Snapshot::default();
    for entry in entries(vars, var_start) {
        if let Some((name, var)) = parse_var(&entry) {
            if !volatile(&name) {
                snapshot.vars.insert(name, var);
            }
        }
    }
    for entry in entries(aliases, |l| l.starts_with("alias ")) {
        if let Some((name, alias)) = parse_alias(&entry) {
            snapshot.aliases.insert(name, alias);
        }
    }
    for entry in entries(functions, |l| function_name(l).is_some()) {
        if let Some(name) = entry.lines().next().and_then(function_name) {
            if !volatile(name) {
                snapshot.functions.insert(name.to_string(), entry.clone());
            }
        }
    }
    snapshot
}

/// Shell code restoring a variable to `before`, from inside a function.
fn restore_var(name: &str, before: &Var, shell: Shell) -> String {
    let Some(value) = &before.value else {
        return format!("unset {name}\n");
    };
    match shell {
        // Unsetting zsh's PATH would untie it from path: assign it back.
        Shell::Zsh if name == "PATH" => format!("{}\n", before.line),
        Shell::Zsh => format!("unset {name} 2>/dev/null\n{}\n", before.line),
        _ if before.flags.contains('A') => {
            format!("unset {name}\ndeclare -gA {name}={value}\n")
        }
        // bash 3.2 prints arrays quoted, `declare -a A='([0]="1")'`: a plain
        // assignment of that would make a string, so let eval unquote it.
        _ if before.flags.contains('a') && value.starts_with("'(") => {
            format!("unset {name}\neval {name}={value}\n")
        }
        _ => {
            let export = if before.exported {
                format!("\nexport {name}")
            } else {
                String::new()
            };
            format!("unset {name}\n{name}={value}{export}\n")
        }
    }
}

/// The undo script for one file, from `before` and `after` snapshots.
/// Undone innermost change first does not matter within one file: every
/// name is restored independently.
pub fn undo(input: &str, shell: Shell, mark: &str) -> String {
    let (before, after) = split(input, &format!("{mark}after"));
    let (before, after) = (parse(before, mark), parse(after, mark));
    let mut out = String::new();

    for (name, var) in &after.vars {
        match before.vars.get(name) {
            None => out.push_str(&format!("unset {name}\n")),
            Some(old) if old != var => out.push_str(&restore_var(name, old, shell)),
            Some(_) => {}
        }
    }
    for (name, old) in &before.vars {
        if !after.vars.contains_key(name) {
            out.push_str(&restore_var(name, old, shell));
        }
    }

    for (name, alias) in &after.aliases {
        let unalias = if alias.suffix {
            format!("unalias -s -- {name}")
        } else {
            format!("unalias -- {name}")
        };
        match before.aliases.get(name) {
            None => out.push_str(&format!("{unalias}\n")),
            Some(old) if old != alias => {
                out.push_str(&format!("{unalias} 2>/dev/null\n{}\n", old.line))
            }
            Some(_) => {}
        }
    }
    for (name, old) in &before.aliases {
        if !after.aliases.contains_key(name) {
            out.push_str(&format!("{}\n", old.line));
        }
    }

    for (name, body) in &after.functions {
        match before.functions.get(name) {
            None => out.push_str(&format!("unset -f {name}\n")),
            Some(old) if old != body => out.push_str(&format!("{old}\n")),
            Some(_) => {}
        }
    }
    for (name, old) in &before.functions {
        if !after.functions.contains_key(name) {
            out.push_str(&format!("{old}\n"));
        }
    }
    out
}

pub fn handle_undo(shell: Shell, mark: &str) -> std::process::ExitCode {
    use std::io::Read;
    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("pathrc: _undo: {error}");
        return std::process::ExitCode::FAILURE;
    }
    print!("{}", undo(&input, shell, mark));
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARK: &str = "#__pathrc_42_";

    fn dump(vars: &str, aliases: &str, functions: &str) -> String {
        format!("{vars}\n{MARK}aliases\n{aliases}\n{MARK}functions\n{functions}\n")
    }

    fn run(before: &str, after: &str, shell: Shell) -> String {
        undo(&format!("{before}{MARK}after\n{after}"), shell, MARK)
    }

    #[test]
    fn a_marker_inside_a_value_does_not_split_the_snapshot() {
        let vars = format!("declare -- X=\"{MARK}after {MARK}aliases\"\ndeclare -- Y=\"1\"");
        let before = dump(&vars, "alias a='b'", "");
        assert_eq!(run(&before, &before, Shell::Bash), "");
    }

    #[test]
    fn bash_new_changed_and_removed_variables() {
        let before = dump(
            "declare -x HOME=\"/h\"\ndeclare -- KEEP=\"1\"\ndeclare -x OVER=\"outer\"\ndeclare -- GONE=$'a\\nb'",
            "",
            "",
        );
        let after = dump(
            "declare -x HOME=\"/h\"\ndeclare -- KEEP=\"1\"\ndeclare -x OVER=\"inner\"\ndeclare -x NEW=\"x\"\ndeclare -- _=\"junk\"\ndeclare -- __pathrc_before=\"…\"",
            "",
            "",
        );
        assert_eq!(
            run(&before, &after, Shell::Bash),
            "unset NEW\nunset OVER\nOVER=\"outer\"\nexport OVER\nunset GONE\nGONE=$'a\\nb'\n"
        );
    }

    #[test]
    fn bash_export_flag_changes_are_undone() {
        let before = dump("declare -- X=\"1\"", "", "");
        let after = dump("declare -x X=\"1\"", "", "");
        assert_eq!(run(&before, &after, Shell::Bash), "unset X\nX=\"1\"\n");
    }

    #[test]
    fn bash_arrays_and_assoc_arrays() {
        let before = dump(
            "declare -a ARR=([0]=\"1\")\ndeclare -A MAP=([k]=\"v\" )",
            "",
            "",
        );
        let after = dump(
            "declare -a ARR=([0]=\"2\")\ndeclare -A MAP=([k]=\"w\" )",
            "",
            "",
        );
        assert_eq!(
            run(&before, &after, Shell::Bash),
            "unset ARR\nARR=([0]=\"1\")\nunset MAP\ndeclare -gA MAP=([k]=\"v\" )\n"
        );
    }

    #[test]
    fn bash_3_2_quoted_arrays_are_restored_as_arrays() {
        let before = dump("declare -a ARR='([0]=\"1\" [1]=\"two words\")'", "", "");
        let after = dump("declare -a ARR='([0]=\"2\")'", "", "");
        assert_eq!(
            run(&before, &after, Shell::Bash),
            "unset ARR\neval ARR='([0]=\"1\" [1]=\"two words\")'\n"
        );
    }

    #[test]
    fn readonly_and_volatile_names_are_left_alone() {
        let before = dump("declare -r RO=\"1\"\ndeclare -- RANDOM=\"1\"", "", "");
        let after = dump(
            "declare -r RO=\"2\"\ndeclare -- RANDOM=\"2\"\ndeclare -a BASH_LINENO=()",
            "",
            "",
        );
        assert_eq!(run(&before, &after, Shell::Bash), "");
    }

    #[test]
    fn zsh_replays_its_own_typeset_g_lines() {
        let before = dump("typeset -g -i N=1\nexport Y='q\"z'", "", "");
        let after = dump(
            "typeset -g -i N=2\nexport Y=other\ntypeset -g NEW=1",
            "",
            "",
        );
        assert_eq!(
            run(&before, &after, Shell::Zsh),
            "unset N 2>/dev/null\ntypeset -g -i N=1\nunset NEW\nunset Y 2>/dev/null\nexport Y='q\"z'\n"
        );
    }

    #[test]
    fn aliases() {
        let before = dump("", "alias ll='ls -l'\nalias old='x'", "");
        let after = dump(
            "",
            "alias ll='ls -la'\nalias gs='git status'\nalias -s txt=less",
            "",
        );
        assert_eq!(
            run(&before, &after, Shell::Zsh),
            "unalias -- gs\nunalias -- ll 2>/dev/null\nalias ll='ls -l'\nunalias -s -- txt\nalias old='x'\n"
        );
    }

    #[test]
    fn bash_and_zsh_functions() {
        let bash_before = dump(
            "",
            "",
            "kept () \n{ \n    echo k\n}\nf () \n{ \n    echo old\n}",
        );
        let bash_after = dump(
            "",
            "",
            "cd () \n{ \n    builtin cd\n}\nkept () \n{ \n    echo k\n}\nf () \n{ \n    echo new\n}\ng () \n{ \n    echo g\n}",
        );
        assert_eq!(
            run(&bash_before, &bash_after, Shell::Bash),
            "f () \n{ \n    echo old\n}\nunset -f g\n"
        );
        let zsh_before = dump("", "", "");
        let zsh_after = dump("", "", "h () {\n\techo h\n}");
        assert_eq!(run(&zsh_before, &zsh_after, Shell::Zsh), "unset -f h\n");
    }
}
