//! Quoting strings into shell code.

use crate::shell::Shell;

/// `text` as one single-quoted word for `shell`.
pub fn quote(text: &str, shell: Shell) -> String {
    match shell {
        // Fish single quotes treat \\ and \' as escapes.
        Shell::Fish => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
        _ => format!("'{}'", text.replace('\'', "'\\''")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_quotes_survive_single_quotes() {
        assert_eq!(quote("it's $HOME", Shell::Bash), "'it'\\''s $HOME'");
    }

    #[test]
    fn fish_escapes_backslash_and_quote() {
        assert_eq!(quote("a\\b'c", Shell::Fish), "'a\\\\b\\'c'");
    }
}
