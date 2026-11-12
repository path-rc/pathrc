//! The cascade (RFC 0002 section 3): the layers in scope, outermost first,
//! folded into the settings the shell should have, and the shell code that
//! applies them.

use crate::config::Layer;
use crate::quote::quote;
use crate::shell::Shell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use zeroize::{Zeroize, Zeroizing};

/// The settings wanted in a directory. `None` is a name a file unset.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub env: BTreeMap<String, Option<String>>,
    pub alias: BTreeMap<String, Option<String>>,
    /// To put in front of `PATH`, in order.
    pub path: Vec<PathBuf>,
}

impl Drop for Wanted {
    fn drop(&mut self) {
        for value in self.env.values_mut().chain(self.alias.values_mut()) {
            value.zeroize();
        }
    }
}

impl Wanted {
    /// Applies one layer on top: `unset.all`, then the named unsets, then
    /// `env`, `path` and `alias`.
    pub fn apply(&mut self, layer: &Layer) {
        if layer.unset_all {
            *self = Wanted::default();
        }
        for name in &layer.unset_env {
            self.env.insert(name.clone(), None);
        }
        for name in &layer.unset_alias {
            self.alias.insert(name.clone(), None);
        }
        for (name, value) in &layer.env {
            self.env.insert(name.clone(), Some(value.clone()));
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
        self.env.is_empty() && self.alias.is_empty() && self.path.is_empty()
    }

    /// Shell code applying these settings on top of the shell as it is.
    pub fn render(&self, shell: Shell) -> Zeroizing<String> {
        let mut out = Zeroizing::new(String::new());
        let fish = shell == Shell::Fish;
        for name in self.env.iter().filter(|(_, v)| v.is_none()).map(|(n, _)| n) {
            out.push_str(&if fish {
                format!("set -e {name}\n")
            } else {
                format!("unset {name}\n")
            });
        }
        for name in self
            .alias
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(n, _)| n)
        {
            out.push_str(&if fish {
                format!("functions -e {name}\n")
            } else {
                format!("unalias {name} 2>/dev/null\n")
            });
        }
        for (name, value) in &self.env {
            if let Some(value) = value {
                let value = quote(value, shell);
                out.push_str(&if fish {
                    format!("set -gx {name} {value}\n")
                } else {
                    format!("export {name}={value}\n")
                });
            }
        }
        if !self.path.is_empty() {
            let dirs: Vec<String> = self
                .path
                .iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect();
            out.push_str(&if fish {
                let words: Vec<String> = dirs.iter().map(|d| quote(d, shell)).collect();
                format!("set -gx PATH {} $PATH\n", words.join(" "))
            } else {
                format!("export PATH={}:\"$PATH\"\n", quote(&dirs.join(":"), shell))
            });
        }
        for (name, value) in &self.alias {
            if let Some(value) = value {
                let value = quote(value, shell);
                out.push_str(&if fish {
                    format!("alias {name} {value}\n")
                } else {
                    format!("alias {name}={value}\n")
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;
    use std::path::Path;

    fn layer(dir: &str, text: &str) -> Layer {
        parse(text, Path::new(dir)).ok().unwrap().layer(Shell::Bash)
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
        assert_eq!(w.env["REGION"].as_deref(), Some("eu-west-1"));
        assert_eq!(w.env["PROJECT"].as_deref(), Some("billing"));
        assert_eq!(w.env["ACCOUNT"].as_deref(), Some("1"));
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
        assert_eq!(w.env["AWS_PROFILE"], None);
        assert_eq!(w.env["OTHER"], None);
        assert_eq!(w.alias["k"], None);
        let w = fold(&[layer(
            "/p",
            "[common]\nunset.env = [\"X\"]\nenv.X = \"again\"\n",
        )]);
        assert_eq!(w.env["X"].as_deref(), Some("again"));
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
        expected.env.insert("B".into(), Some("2".into()));
        assert_eq!(w, expected);
    }

    #[test]
    fn deeper_path_entries_go_first() {
        let w = fold(&[
            layer("/p", "[common]\npath.prepend = [\"bin\"]\n"),
            layer("/p/s", "[common]\npath.prepend = [\"bin\", \"/opt/x\"]\n"),
        ]);
        let p: Vec<_> = w.path.iter().map(|p| p.to_str().unwrap()).collect();
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
}
