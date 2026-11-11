//! The shell a session was initialised for, and which `.pathrc` sections it reads.

use std::fmt;

/// A shell pathrc emits code for, as stored in `PATHRC_SHELL`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Shell {
    /// Shared Bash and Zsh (`pathrc init sh`)
    Sh,
    Bash,
    Zsh,
    Fish,
}

/// A section marker in a `.pathrc` file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Section {
    /// Lines before the first marker: every shell.
    Common,
    /// `#sh`: bash and zsh.
    Sh,
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    pub fn from_name(name: &str) -> Option<Shell> {
        match name {
            "sh" => Some(Shell::Sh),
            "bash" => Some(Shell::Bash),
            "zsh" => Some(Shell::Zsh),
            "fish" => Some(Shell::Fish),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shell::Sh => "sh",
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
        }
    }

    /// Whether this shell reads a section.
    pub fn reads(self, section: Section) -> bool {
        match section {
            Section::Common => true,
            Section::Sh => matches!(self, Shell::Sh | Shell::Bash | Shell::Zsh),
            Section::Bash => self == Shell::Bash,
            Section::Zsh => self == Shell::Zsh,
            Section::Fish => self == Shell::Fish,
        }
    }

    /// The shell this session was initialised for, from `PATHRC_SHELL`.
    pub fn configured() -> Result<Shell, String> {
        match std::env::var("PATHRC_SHELL") {
            Err(_) => Err("pathrc has not been initialised for this shell\n\
                 add 'eval \"$(pathrc init)\"' to your shell rc file\n\
                 see 'pathrc init -h' for options"
                .to_string()),
            Ok(name) => Shell::from_name(&name)
                .ok_or_else(|| format!("unknown PATHRC_SHELL '{name}'; run 'pathrc init'")),
        }
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Section {
    /// The section a line starts, if it is a marker: exactly `#bash`,
    /// `#zsh`, `#fish` or `#sh`, ignoring surrounding space.
    pub fn from_marker(line: &str) -> Option<Section> {
        match line.trim() {
            "#sh" => Some(Section::Sh),
            "#bash" => Some(Section::Bash),
            "#zsh" => Some(Section::Zsh),
            "#fish" => Some(Section::Fish),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_section_is_read_by_bash_zsh_and_sh_only() {
        assert!(Shell::Bash.reads(Section::Sh));
        assert!(Shell::Zsh.reads(Section::Sh));
        assert!(Shell::Sh.reads(Section::Sh));
        assert!(!Shell::Fish.reads(Section::Sh));
    }

    #[test]
    fn shell_specific_sections_are_read_by_that_shell_only() {
        assert!(Shell::Bash.reads(Section::Bash));
        assert!(!Shell::Zsh.reads(Section::Bash));
        assert!(!Shell::Sh.reads(Section::Bash));
        assert!(!Shell::Sh.reads(Section::Zsh));
        assert!(Shell::Fish.reads(Section::Fish));
    }

    #[test]
    fn markers_are_whole_lines() {
        assert_eq!(Section::from_marker("  #zsh "), Some(Section::Zsh));
        assert_eq!(Section::from_marker("# zsh"), None);
        assert_eq!(Section::from_marker("#zsh only"), None);
    }
}
