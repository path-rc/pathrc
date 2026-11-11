//! The `.pathrc` file format: one file, with sections per shell.
//!
//! ```text
//! export EDITOR=vim      # before any marker: every shell
//! #sh
//! alias ll='ls -l'       # bash and zsh
//! #fish
//! alias ll 'ls -l'
//! ```

use crate::shell::{Section, Shell};

/// The lines of `content` that `shell` reads, in file order. Marker lines
/// themselves are left out. Empty when nothing applies to the shell.
pub fn select(content: &str, shell: Shell) -> String {
    let mut section = Section::Common;
    let mut out = String::new();
    for line in content.split_inclusive('\n') {
        if let Some(next) = Section::from_marker(line) {
            section = next;
            continue;
        }
        if shell.reads(section) {
            out.push_str(line);
        }
    }
    if out.trim().is_empty() {
        return String::new();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// A structural problem in a `.pathrc` file.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based; 0 for the file as a whole.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

/// Checks the file's structure, not its shell code: it is UTF-8 text with
/// no NUL bytes, and every line shaped like a marker (`#` and a lower-case
/// word, nothing else) is a known one, which catches `#bahs`. Ordinary
/// comments have a space after the `#` and are never flagged.
pub fn validate(bytes: &[u8]) -> Vec<Problem> {
    let content = match std::str::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) => {
            return vec![Problem {
                line: 0,
                message: format!("not UTF-8 text ({error})"),
            }]
        }
    };
    let mut problems = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line_no = index + 1;
        if line.contains('\0') {
            problems.push(Problem {
                line: line_no,
                message: "contains a NUL byte".to_string(),
            });
        }
        let trimmed = line.trim();
        let looks_like_marker = trimmed.len() > 1
            && trimmed.starts_with('#')
            && trimmed[1..].chars().all(|c| c.is_ascii_lowercase());
        if looks_like_marker && Section::from_marker(trimmed).is_none() {
            problems.push(Problem {
                line: line_no,
                message: format!(
                    "unknown section '{trimmed}' (sections are #sh, #bash, #zsh and #fish)"
                ),
            });
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "export ALL=1\n#sh\nexport SH=1\n#bash\nexport BASH_ONLY=1\n#zsh\nexport ZSH_ONLY=1\n#fish\nset -gx FISH_ONLY 1\n#sh\nexport SH_AGAIN=1\n";

    #[test]
    fn bash_gets_common_sh_and_bash_in_file_order() {
        assert_eq!(
            select(FILE, Shell::Bash),
            "export ALL=1\nexport SH=1\nexport BASH_ONLY=1\nexport SH_AGAIN=1\n"
        );
    }

    #[test]
    fn zsh_gets_common_sh_and_zsh() {
        assert_eq!(
            select(FILE, Shell::Zsh),
            "export ALL=1\nexport SH=1\nexport ZSH_ONLY=1\nexport SH_AGAIN=1\n"
        );
    }

    #[test]
    fn fish_gets_common_and_fish() {
        assert_eq!(
            select(FILE, Shell::Fish),
            "export ALL=1\nset -gx FISH_ONLY 1\n"
        );
    }

    #[test]
    fn shared_sh_gets_common_and_sh() {
        assert_eq!(
            select(FILE, Shell::Sh),
            "export ALL=1\nexport SH=1\nexport SH_AGAIN=1\n"
        );
    }

    #[test]
    fn a_file_with_nothing_for_the_shell_selects_nothing() {
        assert_eq!(select("#fish\nset -gx A 1\n", Shell::Bash), "");
        assert_eq!(select("", Shell::Bash), "");
    }

    #[test]
    fn a_missing_final_newline_is_added() {
        assert_eq!(select("export A=1", Shell::Bash), "export A=1\n");
    }

    #[test]
    fn validate_accepts_comments_and_known_markers() {
        let ok = "# a comment\n#!/bin/bash\n#sh\n  #zsh  \n#\n#TODO\n";
        assert_eq!(validate(ok.as_bytes()), vec![]);
    }

    #[test]
    fn validate_flags_unknown_markers_with_line_numbers() {
        let problems = validate(b"export A=1\n#bahs\nexport B=2\n");
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, 2);
        assert!(problems[0]
            .to_string()
            .starts_with("line 2: unknown section '#bahs'"));
    }

    #[test]
    fn validate_flags_binary_content() {
        assert_eq!(validate(&[0xff, 0xfe])[0].line, 0);
        assert_eq!(validate(b"a\nb\0c\n")[0].line, 2);
    }
}
