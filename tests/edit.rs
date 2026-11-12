//! `pathrc edit`, `encrypt`, `decrypt` and `rekey`.

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

    fn decrypt(&self, file: &Path) -> String {
        decrypt_with(&self.key, file).expect("decrypts with the user's key")
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

fn decrypt_with(key: &age::x25519::Identity, file: &Path) -> Option<String> {
    let bytes = fs::read(file).ok()?;
    let decryptor = age::Decryptor::new(age::armor::ArmoredReader::new(&bytes[..])).ok()?;
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
fn encrypt_then_decrypt_round_trip_keeps_trust() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(
        dir.path().join(".pathrc.toml"),
        "[common]\nenv.SECRET = \"1\"\n",
    )
    .unwrap();
    env.pathrc(dir.path()).arg("allow").assert().success();

    env.pathrc(dir.path()).arg("encrypt").assert().success();
    let sealed = dir.path().join(".pathrc.toml.age");
    assert!(
        !dir.path().join(".pathrc.toml").exists(),
        "plaintext removed"
    );
    assert!(fs::read_to_string(&sealed)
        .unwrap()
        .starts_with("-----BEGIN AGE ENCRYPTED FILE-----"));
    assert_eq!(env.decrypt(&sealed), "[common]\nenv.SECRET = \"1\"\n");
    assert_eq!(
        env.loaded(dir.path(), "echo $SECRET"),
        "1\n",
        "still allowed"
    );

    // Refuses to replace without --force.
    fs::write(
        dir.path().join(".pathrc.toml"),
        "[common]\nenv.OTHER = \"1\"\n",
    )
    .unwrap();
    env.pathrc(dir.path()).arg("encrypt").assert().failure();
    fs::remove_file(dir.path().join(".pathrc.toml")).unwrap();

    env.pathrc(dir.path()).arg("decrypt").assert().success();
    let plain = dir.path().join(".pathrc.toml");
    assert_eq!(
        fs::read_to_string(&plain).unwrap(),
        "[common]\nenv.SECRET = \"1\"\n"
    );
    assert_eq!(
        fs::metadata(&plain).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!sealed.exists());
    assert_eq!(env.loaded(dir.path(), "echo $SECRET"), "1\n");
}

#[test]
fn encrypting_an_unallowed_file_does_not_allow_it() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(dir.path().join(".pathrc.toml"), "[common]\nenv.X = \"1\"\n").unwrap();
    let out = env.pathrc(dir.path()).arg("encrypt").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .contains("not allowed yet"));
    assert_eq!(env.loaded(dir.path(), "echo \"-${X-}\""), "-\n");
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
    fs::write(project.join(".pathrc.toml"), "[common]\nenv.TEAM = \"1\"\n").unwrap();
    env.pathrc(&project).arg("encrypt").assert().success();
    let sealed = project.join(".pathrc.toml.age");
    assert!(decrypt_with(&alice, &sealed).is_some());
    assert!(decrypt_with(&bob, &sealed).is_none());

    fs::write(
        team.path().join(".pathrc-recipients"),
        format!("{}\n{}\n", env.key.to_public(), bob.to_public()),
    )
    .unwrap();
    env.pathrc(&project).arg("allow").assert().success();
    env.pathrc(&project).arg("rekey").assert().success();
    assert!(decrypt_with(&alice, &sealed).is_none());
    assert_eq!(
        decrypt_with(&bob, &sealed).unwrap(),
        "[common]\nenv.TEAM = \"1\"\n"
    );
    assert_eq!(
        env.loaded(&project, "echo $TEAM"),
        "1\n",
        "rekeyed file stays allowed"
    );
}

#[test]
fn edit_an_encrypted_file_in_a_private_temp_file() {
    let env = Env::new();
    let dir = tempdir();
    let editor = env.editor(
        "append.sh",
        "printf '[common]\\nenv.ADDED = \"1\"\\n' >> \"$1\"",
    );
    env.pathrc(dir.path())
        .args(["edit", "--encrypt"])
        .env("EDITOR", &editor)
        .assert()
        .success();
    let sealed = dir.path().join(".pathrc.toml.age");
    assert_eq!(
        env.decrypt(&sealed),
        "[common]\nenv.ADDED = \"1\"\n",
        "template notes stripped"
    );

    // Editing again decrypts what is there.
    let editor = env.editor("append2.sh", "printf 'env.MORE = \"2\"\\n' >> \"$1\"");
    env.pathrc(dir.path())
        .arg("edit")
        .env("EDITOR", &editor)
        .assert()
        .success();
    assert_eq!(
        env.decrypt(&sealed),
        "[common]\nenv.ADDED = \"1\"\nenv.MORE = \"2\"\n"
    );
    assert_eq!(
        env.loaded(dir.path(), "echo $ADDED$MORE"),
        "12\n",
        "saved file is allowed"
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
    env.pathrc(dir.path())
        .arg("edit")
        .arg("--encrypt")
        .env(
            "EDITOR",
            env.editor("first.sh", "printf '[common]\\nenv.K = \"1\"\\n' >> \"$1\""),
        )
        .assert()
        .success();
    let sealed = dir.path().join(".pathrc.toml.age");
    let before = fs::read(&sealed).unwrap();

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
        assert_eq!(fs::read(&sealed).unwrap(), before);
        assert_no_temp_left(&env);
    }
}

#[test]
fn edit_refuses_disk_temp_for_encrypted_files_without_runtime_dir() {
    let env = Env::new();
    let dir = tempdir();
    let out = env
        .pathrc(dir.path())
        .args(["edit", "--encrypt"])
        .env_remove("XDG_RUNTIME_DIR")
        .env("EDITOR", "true")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8(out.stderr).unwrap().contains("--tmp-ok"));
}
