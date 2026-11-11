use assert_cmd::Command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A scratch HOME and XDG_DATA_HOME, so the allow list is never the user's.
struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn pathrc(&self, start: &Path, cwd: &Path) -> Command {
        let mut cmd = Command::cargo_bin("pathrc").unwrap();
        cmd.current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("PATHRC_SHELL", "bash")
            .env("PATHRC_START", start);
        cmd
    }

    fn allow(&self, file: &Path) {
        self.pathrc(file.parent().unwrap(), file.parent().unwrap())
            .arg("allow")
            .arg(file)
            .assert()
            .success();
    }

    fn collate(&self, start: &Path, cwd: &Path) -> (String, String) {
        let output = self.pathrc(start, cwd).arg("collate").output().unwrap();
        assert!(output.status.success());
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    }
}

/// root/.pathrc, root/a/.pathrc, root/a/b/.pathrc, all allowed.
fn three_levels(env: &Env) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let middle = root.path().join("a");
    let inner = middle.join("b");
    fs::create_dir_all(&inner).unwrap();
    fs::write(root.path().join(".pathrc"), "export OUTER=1").unwrap();
    fs::write(middle.join(".pathrc"), "export MIDDLE=1\n").unwrap();
    fs::write(inner.join(".pathrc"), "export INNER=1\n").unwrap();
    for dir in [root.path(), &middle, &inner] {
        env.allow(&dir.join(".pathrc"));
    }
    (root, middle, inner)
}

#[test]
fn collate_skips_an_unreadable_file_and_loads_the_rest() {
    let env = Env::new();
    let (root, middle, inner) = three_levels(&env);
    let broken = middle.join(".pathrc");
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&broken).is_ok() {
        eprintln!("running as a user who can read mode 000 files; skipping");
        return;
    }

    let (stdout, stderr) = env.collate(root.path(), &inner);
    // A file without a trailing newline still ends its own line.
    assert!(stdout.contains("export OUTER=1\n# File:"), "{stdout}");
    assert!(stdout.contains("export INNER=1"), "{stdout}");
    assert!(!stdout.contains("MIDDLE"), "{stdout}");
    assert!(
        stderr.starts_with(&format!("pathrc: {}: ", broken.display())),
        "{stderr}"
    );

    let listed = env
        .pathrc(root.path(), &inner)
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(listed.contains("[unreadable:"), "{listed}");
}

#[test]
fn collate_skips_untrusted_and_changed_files() {
    let env = Env::new();
    let (root, middle, inner) = three_levels(&env);
    let middle_file = middle.join(".pathrc");
    let inner_file = inner.join(".pathrc");
    env.pathrc(&inner, &inner).arg("deny").assert().success();
    fs::write(&middle_file, "export MIDDLE=changed\n").unwrap();

    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert!(stdout.contains("OUTER=1"), "{stdout}");
    assert!(!stdout.contains("MIDDLE"), "{stdout}");
    assert!(!stdout.contains("INNER"), "{stdout}");
    assert!(stderr.contains(&format!(
        "{}: changed since it was allowed",
        middle_file.display()
    )));
    assert!(stderr.contains(&format!("{}: not allowed", inner_file.display())));

    let listed = env
        .pathrc(root.path(), &inner)
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(
        listed.contains(&format!("{}  [changed]", middle_file.display())),
        "{listed}"
    );
    assert!(
        listed.contains(&format!("{}  [untrusted]", inner_file.display())),
        "{listed}"
    );

    env.allow(&middle_file);
    env.pathrc(&inner, &inner).arg("allow").assert().success();
    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert!(
        stdout.contains("MIDDLE=changed") && stdout.contains("INNER=1"),
        "{stdout}"
    );
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn allow_without_a_file_in_the_directory_fails() {
    let env = Env::new();
    let dir = tempfile::tempdir().unwrap();
    let output = env
        .pathrc(dir.path(), dir.path())
        .arg("allow")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("no .pathrc or .pathrc.age file"));
}

#[test]
fn collate_without_init_writes_nothing_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("pathrc")
        .unwrap()
        .current_dir(dir.path())
        .env_clear()
        .arg("collate")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn collate_with_an_unknown_shell_does_not_panic() {
    let env = Env::new();
    let dir = tempfile::tempdir().unwrap();
    let output = env
        .pathrc(dir.path(), dir.path())
        .env("PATHRC_SHELL", "tcsh")
        .arg("collate")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("unknown PATHRC_SHELL"));
}
