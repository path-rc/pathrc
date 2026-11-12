//! The stack of levels a shell has entered, one per file in scope (RFC 0003
//! section 4).
//!
//! The shell keeps it in `__pathrc_state` (not exported, so a child shell
//! starts empty and enters for itself) and passes it to `collate` as
//! `PATHRC_STATE`. It holds no values: for each level, the file and its
//! hash, the names whose previous values the shell saved (and whether each
//! was exported), and the `PATH` entries the level added or dropped. The
//! saved values themselves stay in the shell.
//!
//! ```text
//! pathrc-state 1
//! level <sha256> <path>
//! var AWS_REGION 1
//! alias tfp
//! path /work/project/bin
//! dropped /opt/outer/bin
//! ```

/// One file in scope: `<sha256> <path>`.
pub type Entry = String;

const HEADER: &str = "pathrc-state 1";

pub fn entry(hash: &str, path: &std::path::Path) -> Entry {
    format!("{hash} {}", path.display())
}

/// What entering one file did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Level {
    pub entry: Entry,
    /// Names saved before the level changed them, in slot order, with
    /// whether each was exported at the time.
    pub vars: Vec<(String, bool)>,
    /// Alias names saved, in slot order.
    pub aliases: Vec<String>,
    /// `PATH` entries this level put in front.
    pub path_added: Vec<String>,
    /// Outer levels' `PATH` entries this level's `unset.all` took out.
    pub path_dropped: Vec<String>,
}

/// The stack, outermost first. Anything unreadable (an older pathrc's
/// state, a hand-edited variable) reads as empty, so the shell enters
/// everything afresh.
pub fn parse(text: &str) -> Vec<Level> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Vec::new();
    }
    let mut levels: Vec<Level> = Vec::new();
    for line in lines {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        if kind == "level" {
            levels.push(Level {
                entry: rest.to_string(),
                ..Level::default()
            });
            continue;
        }
        let Some(level) = levels.last_mut() else {
            return Vec::new();
        };
        match kind {
            "var" => match rest.rsplit_once(' ') {
                Some((name, flag)) => level.vars.push((name.to_string(), flag == "1")),
                None => return Vec::new(),
            },
            "alias" => level.aliases.push(rest.to_string()),
            "path" => level.path_added.push(rest.to_string()),
            "dropped" => level.path_dropped.push(rest.to_string()),
            _ => return Vec::new(),
        }
    }
    levels
}

pub fn render(levels: &[Level]) -> String {
    let mut out = format!("{HEADER}\n");
    for level in levels {
        out.push_str(&format!("level {}\n", level.entry));
        for (name, exported) in &level.vars {
            out.push_str(&format!("var {name} {}\n", u8::from(*exported)));
        }
        for name in &level.aliases {
            out.push_str(&format!("alias {name}\n"));
        }
        for dir in &level.path_added {
            out.push_str(&format!("path {dir}\n"));
        }
        for dir in &level.path_dropped {
            out.push_str(&format!("dropped {dir}\n"));
        }
    }
    out
}

/// How many leading levels `loaded` and `wanted` share. Everything loaded
/// above that is left, innermost first, and everything wanted above it is
/// entered: a changed outer file is entered again with every file inside
/// it, so inner overrides land on the right values.
pub fn common_prefix(loaded: &[Level], wanted: &[Entry]) -> usize {
    loaded
        .iter()
        .zip(wanted)
        .take_while(|(level, entry)| &level.entry == *entry)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(entry: &str) -> Level {
        Level {
            entry: entry.to_string(),
            ..Level::default()
        }
    }

    fn entries(list: &[&str]) -> Vec<Entry> {
        list.iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn same_scope_shares_everything() {
        let loaded = vec![level("h1 /a"), level("h2 /a/b")];
        assert_eq!(common_prefix(&loaded, &entries(&["h1 /a", "h2 /a/b"])), 2);
    }

    #[test]
    fn sideways_shares_the_parent() {
        let loaded = vec![level("h1 /w"), level("h2 /w/a")];
        assert_eq!(common_prefix(&loaded, &entries(&["h1 /w", "h3 /w/b"])), 1);
    }

    #[test]
    fn a_changed_outer_file_reenters_from_there() {
        let loaded = vec![level("h1 /w"), level("h2 /w/a")];
        assert_eq!(common_prefix(&loaded, &entries(&["h9 /w", "h2 /w/a"])), 0);
    }

    #[test]
    fn round_trip_with_no_values() {
        let levels = vec![
            Level {
                entry: "h1 /with space".into(),
                vars: vec![("A".into(), true), ("B".into(), false)],
                aliases: vec!["k.x".into()],
                path_added: vec!["/with space/bin".into()],
                path_dropped: vec![],
            },
            Level {
                entry: "h2 /with space/in".into(),
                path_dropped: vec!["/with space/bin".into()],
                ..Level::default()
            },
        ];
        let text = render(&levels);
        assert_eq!(parse(&text), levels);
        assert!(text.starts_with("pathrc-state 1\nlevel h1 /with space\nvar A 1\nvar B 0\n"));
    }

    #[test]
    fn anything_else_reads_as_empty() {
        assert!(parse("").is_empty());
        // pathrc 0.4's PATHRC_LOADED format
        assert!(parse("abc123 /w/.pathrc.toml\n").is_empty());
        assert!(parse("pathrc-state 1\nvar A 1\n").is_empty());
        assert!(parse("pathrc-state 1\nlevel h /a\nwhat x\n").is_empty());
    }
}
