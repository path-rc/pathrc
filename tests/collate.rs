use assert_cmd::Command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn pathrc(start: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::cargo_bin("pathrc").unwrap();
    cmd.current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PATHRC_SHELL", "bash")
        .env("PATHRC_START", start);
    cmd
}

#[test]
fn collate_skips_an_unreadable_file_and_loads_the_rest() {
    let root = tempfile::tempdir().unwrap();
    let middle = root.path().join("a");
    let inner = middle.join("b");
    fs::create_dir_all(&inner).unwrap();
    fs::write(root.path().join(".pathrc-bash"), "export OUTER=1").unwrap();
    let broken = middle.join(".pathrc-bash");
    fs::write(&broken, "export MIDDLE=1\n").unwrap();
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o000)).unwrap();
    fs::write(inner.join(".pathrc-bash"), "export INNER=1\n").unwrap();

    if fs::read(&broken).is_ok() {
        eprintln!("running as a user who can read mode 000 files; skipping");
        return;
    }

    let output = pathrc(root.path(), &inner).arg("collate").output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(output.status.success());
    // A file without a trailing newline still ends its own line.
    assert!(stdout.contains("export OUTER=1\n# File:"), "{stdout}");
    assert!(stdout.contains("export INNER=1"), "{stdout}");
    assert!(!stdout.contains("MIDDLE"), "{stdout}");
    assert!(
        stderr.starts_with(&format!("pathrc: {}: ", broken.display())),
        "{stderr}"
    );

    let listed = pathrc(root.path(), &inner).arg("list").output().unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(listed.contains("[unreadable:"), "{listed}");
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
    let dir = tempfile::tempdir().unwrap();
    let output = pathrc(dir.path(), dir.path())
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
