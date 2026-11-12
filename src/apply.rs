//! The shell code that enters and leaves levels (RFC 0003 section 4).
//!
//! Entering a level saves each name in the shell before its first change
//! (`__pathrc_vsave SLOT NAME`, `__pathrc_asave SLOT NAME`, defined by
//! `pathrc init`), then changes it. Leaving restores the saved values,
//! innermost level first. No value passes through pathrc: it only names
//! slots. Slots are `<level>_<n>`, the n-th name the level saved.
//!
//! pathrc tracks which names are exported, starting from its own
//! environment (the shell's exported variables) and following the changes
//! it emits, so a restore knows whether to export the old value again
//! without asking the shell. It tracks `PATH` the same way and sets it once
//! at the end, removing only the entries pathrc added, so the user's own
//! changes to `PATH` are kept.

use crate::config::Layer;
use crate::quote::quote;
use crate::settings::{set_alias, set_var, unset_alias, unset_var};
use crate::shell::Shell;
use crate::state::{self, Entry, Level};
use std::collections::HashSet;
use zeroize::Zeroizing;

pub struct Emitter {
    shell: Shell,
    out: Zeroizing<String>,
    exported: HashSet<String>,
    path: Vec<String>,
    path_changed: bool,
}

impl Emitter {
    /// Starts from this process's environment, which is the shell's
    /// exported variables and `PATH`.
    pub fn from_env(shell: Shell) -> Emitter {
        let exported = std::env::vars_os()
            .filter_map(|(name, _)| name.into_string().ok())
            .collect();
        let path = std::env::var("PATH").unwrap_or_default();
        Emitter::new(shell, exported, &path)
    }

    pub fn new(shell: Shell, exported: HashSet<String>, path: &str) -> Emitter {
        Emitter {
            shell,
            out: Zeroizing::new(String::new()),
            exported,
            path: path
                .split(':')
                .filter(|d| !d.is_empty())
                .map(String::from)
                .collect(),
            path_changed: false,
        }
    }

    fn call(&mut self, function: &str, slot: &str, name: &str) {
        let name = quote(name, self.shell);
        self.out.push_str(&format!("{function} {slot} {name}\n"));
    }

    fn save_var(&mut self, index: usize, level: &mut Level, name: &str) {
        if level.vars.iter().any(|(n, _)| n == name) {
            return;
        }
        let slot = format!("{index}_{}", level.vars.len());
        self.call("__pathrc_vsave", &slot, name);
        let exported = self.exported.contains(name);
        level.vars.push((name.to_string(), exported));
    }

    fn save_alias(&mut self, index: usize, level: &mut Level, name: &str) {
        if level.aliases.iter().any(|n| n == name) {
            return;
        }
        let slot = format!("{index}_{}", level.aliases.len());
        self.call("__pathrc_asave", &slot, name);
        level.aliases.push(name.to_string());
    }

    fn remove_path(&mut self, dir: &str) -> bool {
        match self.path.iter().position(|d| d == dir) {
            Some(at) => {
                self.path.remove(at);
                self.path_changed = true;
                true
            }
            None => false,
        }
    }

    /// Enters the file `entry` as level `index`, applying `layer` on top of
    /// the `outer` levels, and returns what it did.
    pub fn enter(&mut self, index: usize, entry: &Entry, layer: &Layer, outer: &[Level]) -> Level {
        let shell = self.shell;
        let mut level = Level {
            entry: entry.clone(),
            ..Level::default()
        };
        if layer.unset_all {
            // Everything the outer levels set: their values come back when
            // this level is left.
            for outer_level in outer {
                for (name, _) in &outer_level.vars {
                    self.save_var(index, &mut level, name);
                    self.out.push_str(&unset_var(name, shell));
                    self.exported.remove(name);
                }
                for name in &outer_level.aliases {
                    self.save_alias(index, &mut level, name);
                    self.out.push_str(&unset_alias(name, shell));
                }
                for dir in &outer_level.path_added {
                    if self.remove_path(dir) {
                        level.path_dropped.push(dir.clone());
                    }
                }
            }
        }
        for name in &layer.unset_vars {
            self.save_var(index, &mut level, name);
            self.out.push_str(&unset_var(name, shell));
            self.exported.remove(name);
        }
        for name in &layer.unset_alias {
            self.save_alias(index, &mut level, name);
            self.out.push_str(&unset_alias(name, shell));
        }
        for (name, var) in &layer.vars {
            self.save_var(index, &mut level, name);
            self.out.push_str(&set_var(name, var, shell));
            if var.exported {
                self.exported.insert(name.clone());
            } else {
                self.exported.remove(name);
            }
        }
        if !layer.path.is_empty() {
            let added: Vec<String> = layer
                .path
                .iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect();
            let mut path = added.clone();
            path.append(&mut self.path);
            self.path = path;
            self.path_changed = true;
            level.path_added = added;
        }
        for (name, value) in &layer.alias {
            self.save_alias(index, &mut level, name);
            self.out.push_str(&set_alias(name, value, shell));
        }
        level
    }

    /// Leaves level `index`: restores what it saved, puts back the outer
    /// `PATH` entries it dropped and takes out the ones it added.
    pub fn leave(&mut self, index: usize, level: &Level) {
        for (n, (name, exported)) in level.vars.iter().enumerate().rev() {
            let name_q = quote(name, self.shell);
            let flag = u8::from(*exported);
            self.out
                .push_str(&format!("__pathrc_vrestore {index}_{n} {name_q} {flag}\n"));
            if *exported {
                self.exported.insert(name.clone());
            } else {
                self.exported.remove(name);
            }
        }
        for (n, name) in level.aliases.iter().enumerate().rev() {
            self.call("__pathrc_arestore", &format!("{index}_{n}"), name);
        }
        for dir in &level.path_added {
            self.remove_path(dir);
        }
        if !level.path_dropped.is_empty() {
            let mut path = level.path_dropped.clone();
            path.append(&mut self.path);
            self.path = path;
            self.path_changed = true;
        }
    }

    /// The code, ending with `PATH` (if it changed) and the new state.
    pub fn finish(mut self, stack: &[Level]) -> Zeroizing<String> {
        let shell = self.shell;
        if self.path_changed {
            let line = match shell {
                Shell::Fish => {
                    let words: Vec<String> = self.path.iter().map(|d| quote(d, shell)).collect();
                    format!("set -gx PATH {}\n", words.join(" "))
                }
                _ => format!("export PATH={}\n", quote(&self.path.join(":"), shell)),
            };
            self.out.push_str(&line);
        }
        let state = quote(&state::render(stack), shell);
        self.out.push_str(&match shell {
            Shell::Fish => format!("set -g __pathrc_state {state}\n"),
            _ => format!("__pathrc_state={state}\n"),
        });
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Var;
    use std::path::PathBuf;

    fn layer() -> Layer {
        Layer::default()
    }

    fn emitter(exported: &[&str]) -> Emitter {
        let exported = exported.iter().map(|n| n.to_string()).collect();
        Emitter::new(Shell::Bash, exported, "/usr/bin:/bin")
    }

    #[test]
    fn entering_saves_each_name_once_before_changing_it() {
        let mut l = layer();
        l.vars.insert("REGION".into(), Var::env("eu"));
        l.vars.insert("TOKEN".into(), Var::shell("t"));
        l.unset_vars.insert("REGION".into());
        l.alias.insert("tfp".into(), "terraform plan".into());
        let mut e = emitter(&["REGION"]);
        let level = e.enter(0, &"h /p".to_string(), &l, &[]);
        assert_eq!(
            level.vars,
            vec![("REGION".to_string(), true), ("TOKEN".to_string(), false)]
        );
        assert_eq!(level.aliases, vec!["tfp".to_string()]);
        let code = e.finish(&[level]);
        assert!(
            code.starts_with(
                "__pathrc_vsave 0_0 'REGION'\nunset REGION\n\
             export REGION='eu'\n__pathrc_vsave 0_1 'TOKEN'\nunset TOKEN\nTOKEN='t'\n\
             __pathrc_asave 0_0 'tfp'\nalias tfp='terraform plan'\n__pathrc_state='pathrc-state 1"
            ),
            "{}",
            *code
        );
    }

    #[test]
    fn leaving_restores_innermost_first_with_the_saved_export_flag() {
        let level = Level {
            entry: "h /p".into(),
            vars: vec![("A".into(), true), ("B".into(), false)],
            aliases: vec!["k".into()],
            ..Level::default()
        };
        let mut e = emitter(&[]);
        e.leave(3, &level);
        let code = e.finish(&[]);
        assert_eq!(
            *code,
            "__pathrc_vrestore 3_1 'B' 0\n__pathrc_vrestore 3_0 'A' 1\n\
             __pathrc_arestore 3_0 'k'\n__pathrc_state='pathrc-state 1\n'\n"
        );
    }

    #[test]
    fn path_entries_go_in_front_and_only_ours_come_out() {
        let mut l = layer();
        l.path = vec![PathBuf::from("/p/bin")];
        let mut e = Emitter::new(Shell::Bash, HashSet::new(), "/usr/bin:/bin");
        let level = e.enter(0, &"h /p".to_string(), &l, &[]);
        assert_eq!(e.path, ["/p/bin", "/usr/bin", "/bin"]);
        // The user puts /mine in front meanwhile; leaving keeps it.
        let mut e = Emitter::new(Shell::Bash, HashSet::new(), "/mine:/p/bin:/usr/bin:/bin");
        e.leave(0, &level);
        assert_eq!(e.path, ["/mine", "/usr/bin", "/bin"]);
        assert!(e
            .finish(&[])
            .contains("export PATH='/mine:/usr/bin:/bin'\n"));
    }

    #[test]
    fn unset_all_saves_and_removes_what_the_outer_levels_set() {
        let outer = Level {
            entry: "h1 /p".into(),
            vars: vec![("REGION".into(), true)],
            aliases: vec!["tfp".into()],
            path_added: vec!["/p/bin".into()],
            path_dropped: vec![],
        };
        let mut l = layer();
        l.unset_all = true;
        let mut e = Emitter::new(Shell::Bash, ["REGION".to_string()].into(), "/p/bin:/bin");
        let level = e.enter(1, &"h2 /p/s".to_string(), &l, std::slice::from_ref(&outer));
        assert_eq!(level.vars, vec![("REGION".to_string(), true)]);
        assert_eq!(level.path_dropped, vec!["/p/bin".to_string()]);
        assert_eq!(e.path, ["/bin"]);
        // Leaving puts the outer entry back in front.
        e.leave(1, &level);
        assert_eq!(e.path, ["/p/bin", "/bin"]);
    }

    #[test]
    fn fish_state_is_a_global() {
        let e = Emitter::new(Shell::Fish, HashSet::new(), "/bin");
        assert_eq!(*e.finish(&[]), "set -g __pathrc_state 'pathrc-state 1\n'\n");
    }
}
