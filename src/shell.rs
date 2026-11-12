//! The shell a session was initialised for.

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
