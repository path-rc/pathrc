//! `pathrc edit`, `secret` and `rekey`.

use age::secrecy::ExposeSecret;
use assert_cmd::Command;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A temporary directory named by its resolved path. On macOS the temp
/// directory is under `/var`, a symlink to `/private/var`, and pathrc reports
/// resolved paths.
fn tempdir() -> tempfile::TempDir {
    let base = std::env::temp_dir().canonicalize().unwrap();
    tempfile::tempdir_in(base).unwrap()
}

struct Env {
    home: tempfile::TempDir,
    key: age::x25519::Identity,
}

impl Env {
    fn new() -> Env {
        let env = Env {
            home: tempdir(),
            key: age::x25519::Identity::generate(),
        };
        let config = env.home.path().join("config/pathrc");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("identity"),
            format!("{}\n", env.key.to_string().expose_secret()),
        )
        .unwrap();
        let runtime = env.runtime();
        fs::create_dir_all(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        env
    }

    fn runtime(&self) -> PathBuf {
        self.home.path().join("run")
    }

    fn pathrc(&self, cwd: &Path) -> Command {
        let mut cmd = Command::cargo_bin("pathrc").unwrap();
        cmd.current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_RUNTIME_DIR", self.runtime())
            .env("PATHRC_SHELL", "bash")
            .env("PATHRC_START", cwd);
        cmd
    }

    /// An editor script; `$1` is the file. Logs the file's path and mode.
    fn editor(&self, name: &str, body: &str) -> String {
        let path = self.home.path().join(name);
        let log = self.home.path().join("editor.log");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$(ls -ld \"$1\" | cut -c1-10)\" >> '{}'\n{body}\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn editor_log(&self) -> String {
        fs::read_to_string(self.home.path().join("editor.log")).unwrap_or_default()
    }

    /// The plaintext of `[table.enc] kind.name` in `file`.
    fn plain(&self, file: &Path, table: &str, kind: &str, name: &str) -> String {
        decrypt_with(&self.key, &sealed(file, table, kind, name))
            .expect("decrypts with the user's key")
    }

    /// `pathrc secret` with `value` on stdin.
    fn secret(&self, cwd: &Path, args: &[&str], value: &str) -> std::process::Output {
        self.pathrc(cwd)
            .arg("secret")
            .args(args)
            .write_stdin(value)
            .output()
            .unwrap()
    }

    /// What the cd hook would set for `cwd`, as `echo`ed by `then`.
    fn loaded(&self, cwd: &Path, then: &str) -> String {
        let collated = self.pathrc(cwd).arg("collate").output().unwrap();
        let helpers: String = include_str!("../init/pathrc.bash.cfg")
            .lines()
            .filter(|line| line.trim() != "__pathrc_collate")
            .map(|line| format!("{line}\n"))
            .collect();
        let script = format!("{helpers}\neval \"$1\"; {then}");
        let out = std::process::Command::new("bash")
            .args([
                "-c",
                &script,
                "_",
                &String::from_utf8(collated.stdout).unwrap(),
            ])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    }
}

/// The base64 value of `[table.enc] kind.name` in `file`.
fn sealed(file: &Path, table: &str, kind: &str, name: &str) -> String {
    let text = fs::read_to_string(file).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap();
    doc[table]["enc"][kind][name].as_str().unwrap().to_string()
}

fn decrypt_with(key: &age::x25519::Identity, value: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()?;
    let decryptor = age::Decryptor::new(&bytes[..]).ok()?;
    let mut reader = decryptor
        .decrypt(std::iter::once(key as &dyn age::Identity))
        .ok()?;
    let mut out = String::new();
    reader.read_to_string(&mut out).ok()?;
    Some(out)
}

fn assert_no_temp_left(env: &Env) {
    let dir = env.runtime().join("pathrc");
    let left: Vec<_> = fs::read_dir(&dir)
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn secret_adds_an_encrypted_value_and_allows_a_new_file() {
    let env = Env::new();
    let dir = tempdir();
    let out = env.secret(dir.path(), &["API_TOKEN", "--var"], "it's a \"token\"\n");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file = dir.path().join(".pathrc.toml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("[common.enc]\nvar.API_TOKEN = \""),
        "{text}"
    );
    assert!(
        !text.contains("token"),
        "the value is not in the file: {text}"
    );
    // One final newline is dropped; the rest is kept exactly.
    assert_eq!(
        env.plain(&file, "common", "var", "API_TOKEN"),
        "it's a \"token\""
    );
    assert_eq!(
        env.loaded(
            dir.path(),
            "echo \"$API_TOKEN|$(sh -c 'echo ${API_TOKEN-unexported}')\""
        ),
        "it's a \"token\"|unexported\n",
        "a new file is allowed, and var is not exported"
    );
}

#[test]
fn secret_replaces_a_value_and_moves_it_between_env_and_var() {
    let env = Env::new();
    let dir = tempdir();
    let file = dir.path().join(".pathrc.toml");
    fs::write(&file, "# kept\n[common]\nenv.REGION = \"eu\"\n").unwrap();
    env.pathrc(dir.path()).arg("allow").assert().success();
    assert!(env
        .secret(dir.path(), &["TOKEN", "--var"], "one")
        .status
        .success());
    assert!(env.secret(dir.path(), &["TOKEN"], "two").status.success());
    assert!(env
        .secret(dir.path(), &["ZSH_ONLY", "--table", "zsh"], "z")
        .status
        .success());
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("# kept\n[common]\nenv.REGION = \"eu\"\n"),
        "{text}"
    );
    assert!(!text.contains("var.TOKEN"), "{text}");
    assert_eq!(env.plain(&file, "common", "env", "TOKEN"), "two");
    assert_eq!(env.plain(&file, "zsh", "env", "ZSH_ONLY"), "z");
    assert_eq!(
        env.loaded(dir.path(), "echo \"$REGION|$TOKEN|${ZSH_ONLY-}\""),
        "eu|two|\n",
        "still allowed, and bash doesn't read [zsh.enc]"
    );
    let bad = env.secret(dir.path(), &["X", "--table", "tcsh"], "v");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("no table [tcsh]"));
    let bad = env.secret(dir.path(), &["A-B"], "v");
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a variable name"));
}

#[test]
fn secret_on_an_unallowed_file_does_not_allow_it() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(dir.path().join(".pathrc.toml"), "[common]\nenv.X = \"1\"\n").unwrap();
    let out = env.secret(dir.path(), &["S"], "s");
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .contains("not allowed yet"));
    assert_eq!(env.loaded(dir.path(), "echo \"-${X-}${S-}\""), "-\n");
}

#[test]
fn recipients_file_and_rekey() {
    let env = Env::new();
    let alice = age::x25519::Identity::generate();
    let bob = age::x25519::Identity::generate();
    let team = tempdir();
    let project = team.path().join("project");
    fs::create_dir_all(&project).unwrap();
    // The nearest .pathrc-recipients up the hierarchy wins.
    fs::write(
        team.path().join(".pathrc-recipients"),
        format!(
            "# the team\n{}\n{}\n",
            env.key.to_public(),
            alice.to_public()
        ),
    )
    .unwrap();
    assert!(env.secret(&project, &["TEAM"], "1").status.success());
    let file = project.join(".pathrc.toml");
    let value = sealed(&file, "common", "env", "TEAM");
    assert!(decrypt_with(&alice, &value).is_some());
    assert!(decrypt_with(&bob, &value).is_none());

    fs::write(
        team.path().join(".pathrc-recipients"),
        format!("{}\n{}\n", env.key.to_public(), bob.to_public()),
    )
    .unwrap();
    let out = env.pathrc(&project).arg("rekey").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .starts_with("re-encrypted 1 value in "));
    let value = sealed(&file, "common", "env", "TEAM");
    assert!(decrypt_with(&alice, &value).is_none());
    assert_eq!(decrypt_with(&bob, &value).unwrap(), "1");
    assert_eq!(
        env.loaded(&project, "echo $TEAM"),
        "1\n",
        "rekeyed file stays allowed"
    );
}

#[test]
fn edit_shows_secrets_decrypted_and_reencrypts_only_what_changed() {
    let env = Env::new();
    let dir = tempdir();
    let file = dir.path().join(".pathrc.toml");
    fs::write(&file, "[common]\nenv.PLAIN = \"p\"\n").unwrap();
    env.pathrc(dir.path()).arg("allow").assert().success();
    for (name, value) in [("KEEP", "kept"), ("CHANGE", "old")] {
        assert!(env.secret(dir.path(), &[name], value).status.success());
    }
    let keep_before = sealed(&file, "common", "env", "KEEP");
    // The editor keeps a copy of what it was shown, changes one value and
    // adds a new one in plain text.
    let shown = env.home.path().join("shown.toml");
    let editor = env.editor(
        "change.sh",
        &format!(
            "cp \"$1\" '{}'\nsed -i.bak 's/\"old\"/\"new\"/' \"$1\"\nrm -f \"$1.bak\"\nprintf 'var.ADDED = \"fresh\"\\n' >> \"$1\"",
            shown.display()
        ),
    );
    let out = env
        .pathrc(dir.path())
        .arg("edit")
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("2 values encrypted"), "{stdout}");

    let shown = fs::read_to_string(&shown).unwrap();
    assert!(
        shown.contains("env.KEEP = \"kept\"") && shown.contains("env.CHANGE = \"old\""),
        "{shown}"
    );
    assert!(
        shown.starts_with("# pathrc: values in .enc tables are shown decrypted"),
        "{shown}"
    );
    assert_eq!(
        sealed(&file, "common", "env", "KEEP"),
        keep_before,
        "unchanged ciphertext"
    );
    assert_eq!(env.plain(&file, "common", "env", "CHANGE"), "new");
    assert_eq!(env.plain(&file, "common", "var", "ADDED"), "fresh");
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains("pathrc:") && !text.contains("\"new\""),
        "{text}"
    );

    let log = env.editor_log();
    for line in log.lines() {
        let (path, mode) = line.split_once(' ').unwrap();
        assert!(
            path.starts_with(&env.runtime().join("pathrc").display().to_string()),
            "{log}"
        );
        assert_eq!(mode, "-rw-------", "{log}");
    }
    assert_no_temp_left(&env);
}

#[test]
fn a_new_file_gets_plain_text_typed_into_enc_encrypted() {
    let env = Env::new();
    let dir = tempdir();
    let editor = env.editor(
        "new.sh",
        "printf '[common]\\nenv.A = \"a\"\\n[common.enc]\\nenv.S = \"typed\"\\n' >> \"$1\"",
    );
    env.pathrc(dir.path())
        .arg("edit")
        .env("EDITOR", &editor)
        .assert()
        .success();
    let file = dir.path().join(".pathrc.toml");
    assert_eq!(env.plain(&file, "common", "env", "S"), "typed");
    assert!(!fs::read_to_string(&file).unwrap().contains("typed"));
    assert_eq!(env.loaded(dir.path(), "echo \"$A|$S\""), "a|typed\n");
}

#[test]
fn an_invalid_file_is_not_saved_without_a_terminal() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(dir.path().join(".pathrc.toml"), "[common]\nenv.A = \"1\"\n").unwrap();
    let editor = env.editor("typo.sh", "printf '[bahs]\\nenv.B = \"1\"\\n' >> \"$1\"");
    let out = env
        .pathrc(dir.path())
        .arg("edit")
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("line 3, column ") && stderr.contains("bahs"),
        "{stderr}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".pathrc.toml")).unwrap(),
        "[common]\nenv.A = \"1\"\n"
    );
    assert_no_temp_left(&env);
}

#[test]
fn a_failing_or_interrupted_editor_saves_nothing_and_cleans_up() {
    let env = Env::new();
    let dir = tempdir();
    assert!(env.secret(dir.path(), &["K"], "1").status.success());
    let file = dir.path().join(".pathrc.toml");
    let before = fs::read(&file).unwrap();

    let fails = env.editor("fails.sh", "echo 'env.K = \"2\"' >> \"$1\"; exit 1");
    // Ctrl-C reaches the whole foreground group; here pathrc gets SIGINT and
    // the editor then exits as an interrupted one would.
    let interrupted = env.editor(
        "interrupted.sh",
        // Walk up to pathrc: macOS's sh execs the editor, so pathrc is its
        // parent there, while dash leaves a shell in between.
        "echo 'env.K = \"3\"' >> \"$1\"
p=$PPID
while [ \"$p\" -gt 1 ]; do
  case $(ps -o comm= -p \"$p\") in *pathrc) kill -INT \"$p\"; break ;; esac
  p=$(ps -o ppid= -p \"$p\" | tr -d ' ')
done
sleep 0.2; exit 130",
    );
    for editor in [fails, interrupted] {
        let out = env
            .pathrc(dir.path())
            .arg("edit")
            .env("EDITOR", &editor)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8(out.stderr)
            .unwrap()
            .contains("nothing saved"));
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_no_temp_left(&env);
    }
}

#[test]
fn edit_needs_a_runtime_dir_only_for_a_file_with_secrets() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(dir.path().join(".pathrc.toml"), "[common]\nenv.A = \"1\"\n").unwrap();
    let plain = env
        .pathrc(dir.path())
        .arg("edit")
        .env_remove("XDG_RUNTIME_DIR")
        .env("EDITOR", "true")
        .output()
        .unwrap();
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert!(env.secret(dir.path(), &["S"], "s").status.success());
    let out = env
        .pathrc(dir.path())
        .arg("edit")
        .env_remove("XDG_RUNTIME_DIR")
        .env("EDITOR", "true")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8(out.stderr).unwrap().contains("--tmp-ok"));
}
