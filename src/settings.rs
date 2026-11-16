//! The cascade (RFC 0002 section 3): the layers in scope, outermost first,
//! folded into the settings the shell should have, and the shell code that
//! applies them.

use crate::config::{Layer, PathEntry, Var};
use crate::expand;
use crate::quote::quote;
use crate::shell::Shell;
use std::collections::BTreeMap;
use zeroize::{Zeroize, Zeroizing};

/// The settings wanted in a directory. `None` is a name a file unset.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    /// Exported (`env`) and shell (`var`) variables.
    pub vars: BTreeMap<String, Option<Var>>,
    pub alias: BTreeMap<String, Option<String>>,
    /// To put in front of `PATH`, in order.
    pub path: Vec<PathEntry>,
}

impl Drop for Wanted {
    fn drop(&mut self) {
        for var in self.vars.values_mut().flatten() {
            var.value.zeroize();
        }
        for value in self.alias.values_mut() {
            value.zeroize();
        }
    }
}

impl Wanted {
    /// Applies one layer on top: `unset.all`, then the named unsets, then
    /// `env` and `var`, `path` and `alias`.
    pub fn apply(&mut self, layer: &Layer) {
        if layer.unset_all {
            *self = Wanted::default();
        }
        for name in &layer.unset_vars {
            self.vars.insert(name.clone(), None);
        }
        for name in &layer.unset_alias {
            self.alias.insert(name.clone(), None);
        }
        for (name, var) in &layer.vars {
            self.vars.insert(name.clone(), Some(var.clone()));
        }
        if !layer.path.is_empty() {
            // A deeper file's entries go in front of the outer ones.
            let mut path = layer.path.clone();
            path.append(&mut self.path);
            self.path = path;
        }
        for (name, value) in &layer.alias {
            self.alias.insert(name.clone(), Some(value.clone()));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.alias.is_empty() && self.path.is_empty()
    }

    /// Shell code applying these settings on top of the shell as it is.
    pub fn render(&self, shell: Shell) -> Zeroizing<String> {
        let mut out = Zeroizing::new(String::new());
        for name in self
            .vars
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| n)
        {
            out.push_str(&unset_var(name, shell));
        }
        for name in self
            .alias
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| n)
        {
            out.push_str(&unset_alias(name, shell));
        }
        for (name, var) in &self.vars {
            if let Some(var) = var {
                out.push_str(&set_var(name, var, shell));
            }
        }
        if !self.path.is_empty() {
            // Each entry a word of its own, expanding its references.
            let words: Vec<String> = self
                .path
                .iter()
                .map(|e| value_word(&e.text, e.expand, shell))
                .collect();
            out.push_str(&if shell == Shell::Fish {
                format!("set -gx PATH {} $PATH\n", words.join(" "))
            } else {
                format!("export PATH={}:\"$PATH\"\n", words.join(":"))
            });
        }
        for (name, value) in &self.alias {
            if let Some(value) = value {
                out.push_str(&set_alias(name, value, shell));
            }
        }
        out
    }
}

/// Code setting a variable: exported for `env`, not for `var`. A `var` is
/// unset first: assigning to a name that is already exported would keep it
/// exported, and a `var` must not reach child programs.
pub fn set_var(name: &str, var: &Var, shell: Shell) -> String {
    let value = value_word(&var.value, var.expand, shell);
    match (shell, var.exported, var.expand) {
        (Shell::Fish, true, _) => format!("set -gx {name} {value}\n"),
        (_, true, _) => format!("export {name}={value}\n"),
        // An expanding var may refer to its own previous value, which the
        // unset below would lose: work it out first.
        (Shell::Fish, false, true) => {
            format!("set -l __pathrc_v {value}\nset -e {name}\nset -gu {name} \"$__pathrc_v\"\n")
        }
        (Shell::Fish, false, false) => format!("set -e {name}\nset -gu {name} {value}\n"),
        // typeset -g: plain assignment in a function warns under zsh's
        // WARN_CREATE_GLOBAL.
        // Not `local`: `pathrc show`'s output may run outside a function.
        (Shell::Zsh, false, true) => format!(
            "typeset -g __pathrc_v={value}\nunset {name}\n\
             typeset -g {name}=\"$__pathrc_v\"\nunset __pathrc_v\n"
        ),
        (Shell::Zsh, false, false) => format!("unset {name}\ntypeset -g {name}={value}\n"),
        (_, false, true) => {
            format!("__pathrc_v={value}\nunset {name}\n{name}=\"$__pathrc_v\"\nunset __pathrc_v\n")
        }
        (_, false, false) => format!("unset {name}\n{name}={value}\n"),
    }
}

/// A value as one shell word: expanding its `$NAME` references when it
/// came from a double-quoted string with a `$`, else exactly as written.
pub fn value_word(value: &str, expand: bool, shell: Shell) -> String {
    match expand.then(|| expand::parse(value)) {
        Some(Ok(parts)) => expand::render(&parts, shell),
        // Checked when the file was parsed; never seen, but stay literal.
        _ => quote(value, shell),
    }
}

pub fn unset_var(name: &str, shell: Shell) -> String {
    match shell {
        Shell::Fish => format!("set -e {name}\n"),
        _ => format!("unset {name}\n"),
    }
}

pub fn set_alias(name: &str, value: &str, shell: Shell) -> String {
    let value = quote(value, shell);
    match shell {
        Shell::Fish => format!("alias {name} {value}\n"),
        _ => format!("alias {name}={value}\n"),
    }
}

pub fn unset_alias(name: &str, shell: Shell) -> String {
    match shell {
        Shell::Fish => format!("functions -e {name}\n"),
        _ => format!("unalias {name} 2>/dev/null\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use std::path::Path;

    fn layer(dir: &str, text: &str) -> Layer {
        parse(text, Path::new(dir))
            .ok()
            .unwrap()
            .layer(Shell::Bash, &mut |_| Err("no identity".to_string()))
            .unwrap()
    }

    fn fold(layers: &[Layer]) -> Wanted {
        let mut wanted = Wanted::default();
        for layer in layers {
            wanted.apply(layer);
        }
        wanted
    }

    #[test]
    fn a_deeper_file_overrides_an_outer_one() {
        let w = fold(&[
            layer(
                "/p",
                "[common]\nenv.REGION = \"eu-west-2\"\nenv.PROJECT = \"billing\"\n",
            ),
            layer(
                "/p/prod",
                "[common]\nenv.REGION = \"eu-west-1\"\nenv.ACCOUNT = \"1\"\n",
            ),
        ]);
        assert_eq!(w.vars["REGION"], Some(Var::env("eu-west-1")));
        assert_eq!(w.vars["PROJECT"], Some(Var::env("billing")));
        assert_eq!(w.vars["ACCOUNT"], Some(Var::env("1")));
    }

    #[test]
    fn a_named_unset_stays_unset_and_a_file_can_set_it_again() {
        let w = fold(&[
            layer(
                "/p",
                "[common]\nenv.AWS_PROFILE = \"dev\"\nalias.k = \"kubectl\"\n",
            ),
            layer(
                "/p/s",
                "[common]\nunset.env = [\"AWS_PROFILE\", \"OTHER\"]\nunset.alias = [\"k\"]\n",
            ),
        ]);
        assert_eq!(w.vars["AWS_PROFILE"], None);
        assert_eq!(w.vars["OTHER"], None);
        assert_eq!(w.alias["k"], None);
        let w = fold(&[layer(
            "/p",
            "[common]\nunset.env = [\"X\"]\nenv.X = \"again\"\n",
        )]);
        assert_eq!(w.vars["X"], Some(Var::env("again")));
    }

    #[test]
    fn unset_all_starts_afresh_path_included() {
        let w = fold(&[
            layer(
                "/p",
                "[common]\nenv.A = \"1\"\nunset.env = [\"U\"]\npath.prepend = [\"bin\"]\n",
            ),
            layer("/p/s", "[common]\nunset.all = true\nenv.B = \"2\"\n"),
        ]);
        let mut expected = Wanted::default();
        expected.vars.insert("B".into(), Some(Var::env("2")));
        assert_eq!(w, expected);
    }

    #[test]
    fn deeper_path_entries_go_first() {
        let w = fold(&[
            layer("/p", "[common]\npath.prepend = [\"bin\"]\n"),
            layer("/p/s", "[common]\npath.prepend = [\"bin\", \"/opt/x\"]\n"),
        ]);
        let p: Vec<_> = w.path.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(p, ["/p/s/bin", "/opt/x", "/p/bin"]);
    }

    #[test]
    fn renders_unsets_then_env_path_alias() {
        let w = fold(&[layer(
            "/p",
            "[common]\nunset.env = [\"G\"]\nunset.alias = [\"o\"]\nenv.V = \"it's\"\npath.prepend = [\"bin\"]\nalias.ll = \"ls -l\"\n",
        )]);
        assert_eq!(
            *w.render(Shell::Bash),
            "unset G\nunalias o 2>/dev/null\nexport V='it'\\''s'\n\
             export PATH='/p/bin':\"$PATH\"\nalias ll='ls -l'\n"
        );
        assert_eq!(
            *w.render(Shell::Fish),
            "set -e G\nfunctions -e o\nset -gx V 'it\\'s'\n\
             set -gx PATH '/p/bin' $PATH\nalias ll 'ls -l'\n"
        );
    }

    #[test]
    fn nothing_wanted_renders_nothing() {
        assert!(Wanted::default().is_empty());
        assert_eq!(*Wanted::default().render(Shell::Zsh), "");
    }

    #[test]
    fn a_var_is_unset_first_so_it_is_never_exported() {
        let w = fold(&[layer("/p", "[common]\nvar.TOKEN = \"t\"\nenv.E = \"e\"\n")]);
        assert_eq!(
            *w.render(Shell::Bash),
            "export E='e'\nunset TOKEN\nTOKEN='t'\n"
        );
        assert_eq!(
            *w.render(Shell::Fish),
            "set -gx E 'e'\nset -e TOKEN\nset -gu TOKEN 't'\n"
        );
    }

    #[test]
    fn expanding_values_are_written_for_the_shell_to_expand() {
        let w = fold(&[layer(
            "/p",
            "[common]\nenv.E = \"$HOME/a`b`\"\nvar.V = \"$V:more\"\npath.prepend = [\"$HOME/bin\", \"lit\"]\n",
        )]);
        assert_eq!(
            *w.render(Shell::Bash),
            "export E=\"${HOME-}/a\\`b\\`\"\n__pathrc_v=\"${V-}:more\"\nunset V\nV=\"$__pathrc_v\"\nunset __pathrc_v\n\
             export PATH=\"${HOME-}/bin\":'/p/lit':\"$PATH\"\n"
        );
        assert_eq!(
            *w.render(Shell::Fish),
            "set -gx E \"$HOME\"\"/a`b`\"\nset -l __pathrc_v \"$V\"\":more\"\nset -e V\nset -gu V \"$__pathrc_v\"\n\
             set -gx PATH \"$HOME\"\"/bin\" '/p/lit' $PATH\n"
        );
    }
}
