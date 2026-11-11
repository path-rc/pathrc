//! The stack of files a shell has loaded, and what to unload and load next.
//!
//! The shell keeps it in `__pathrc_loaded` (not exported, so a child shell
//! starts empty and loads for itself) and passes it to `collate` as
//! `PATHRC_LOADED`: one `<sha256> <path>` line per loaded file, outermost
//! first. The hash is of the file's bytes (ciphertext for `.pathrc.age`).

/// One loaded file: `<sha256> <path>`.
pub type Entry = String;

pub fn entry(hash: &str, path: &std::path::Path) -> Entry {
    format!("{hash} {}", path.display())
}

pub fn parse(state: &str) -> Vec<Entry> {
    state
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

pub fn render(entries: &[Entry]) -> String {
    entries.iter().map(|e| format!("{e}\n")).collect()
}

/// How many leading entries `loaded` and `wanted` share. Everything loaded
/// above that is unloaded, innermost first, and everything wanted above it
/// is loaded: a changed outer file reloads every file inside it, so inner
/// overrides land on the right values again.
pub fn common_prefix(loaded: &[Entry], wanted: &[Entry]) -> usize {
    loaded
        .iter()
        .zip(wanted)
        .take_while(|(a, b)| a == b)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(s: &str) -> Entry {
        s.to_string()
    }

    #[test]
    fn same_scope_shares_everything() {
        let loaded = vec![e("h1 /a"), e("h2 /a/b")];
        assert_eq!(common_prefix(&loaded, &loaded.clone()), 2);
    }

    #[test]
    fn sideways_shares_the_parent() {
        let loaded = vec![e("h1 /w"), e("h2 /w/a")];
        let wanted = vec![e("h1 /w"), e("h3 /w/b")];
        assert_eq!(common_prefix(&loaded, &wanted), 1);
    }

    #[test]
    fn a_changed_outer_file_reloads_from_there() {
        let loaded = vec![e("h1 /w"), e("h2 /w/a")];
        let wanted = vec![e("h9 /w"), e("h2 /w/a")];
        assert_eq!(common_prefix(&loaded, &wanted), 0);
    }

    #[test]
    fn round_trip() {
        let entries = vec![e("h1 /with space"), e("h2 /b")];
        assert_eq!(parse(&render(&entries)), entries);
        assert!(parse("").is_empty());
    }
}
