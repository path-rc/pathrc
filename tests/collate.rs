use assert_cmd::Command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const FILE: &str = ".pathrc.toml";

/// A temporary directory named by its resolved path. On macOS the temp
/// directory is under `/var`, a symlink to `/private/var`, and pathrc reports
/// resolved paths.
fn tempdir() -> tempfile::TempDir {
    let base = std::env::temp_dir().canonicalize().unwrap();
    tempfile::tempdir_in(base).unwrap()
}

/// A scratch HOME and XDG_DATA_HOME, so the allow list is never the user's.
struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        Env { home: tempdir() }
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

/// Evaluates collate's output in bash with stand-in load functions, then
/// runs `then`, returning its stdout.
fn eval_in_bash(collated: &str, then: &str) -> String {
    let script = format!(
        "__pathrc_load() {{ eval \"$3\"; }}; __pathrc_unload() {{ :; }}; eval \"$1\"; {then}"
    );
    let output = std::process::Command::new("bash")
        .args(["-c", &script, "_", collated])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

const LEVELS: &str = "echo \"${OUTER-}|${MIDDLE-}|${INNER-}\"";

/// root/.pathrc.toml, root/a/.pathrc.toml, root/a/b/.pathrc.toml, all allowed.
fn three_levels(env: &Env) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempdir();
    let middle = root.path().join("a");
    let inner = middle.join("b");
    fs::create_dir_all(&inner).unwrap();
    fs::write(root.path().join(FILE), "[common]\nenv.OUTER = \"1\"").unwrap();
    fs::write(middle.join(FILE), "[common]\nenv.MIDDLE = \"1\"\n").unwrap();
    fs::write(inner.join(FILE), "[bash]\nenv.INNER = \"1\"\n").unwrap();
    for dir in [root.path(), &middle, &inner] {
        env.allow(&dir.join(FILE));
    }
    (root, middle, inner)
}

#[test]
fn collate_skips_an_unreadable_file_and_loads_the_rest() {
    let env = Env::new();
    let (root, middle, inner) = three_levels(&env);
    let broken = middle.join(FILE);
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&broken).is_ok() {
        eprintln!("running as a user who can read mode 000 files; skipping");
        return;
    }

    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert_eq!(eval_in_bash(&stdout, LEVELS), "1||1\n", "{stdout}");
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
    let middle_file = middle.join(FILE);
    let inner_file = inner.join(FILE);
    env.pathrc(&inner, &inner).arg("deny").assert().success();
    fs::write(&middle_file, "[common]\nenv.MIDDLE = \"changed\"\n").unwrap();

    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert_eq!(eval_in_bash(&stdout, LEVELS), "1||\n", "{stdout}");
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
    assert_eq!(eval_in_bash(&stdout, LEVELS), "1|changed|1\n", "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn allow_without_a_file_in_the_directory_fails() {
    let env = Env::new();
    let dir = tempdir();
    let output = env
        .pathrc(dir.path(), dir.path())
        .arg("allow")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("no .pathrc.toml or .pathrc.toml.age file"));
}

#[test]
fn collate_without_init_writes_nothing_to_stdout() {
    let dir = tempdir();
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
    let dir = tempdir();
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

#[test]
fn an_invalid_file_is_reported_with_its_position_and_skipped() {
    let env = Env::new();
    let (root, middle, inner) = three_levels(&env);
    let bad = middle.join(FILE);
    fs::write(&bad, "[common]\nenv.ACCOUNT = 012345678901\n").unwrap();
    env.allow(&bad);

    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert_eq!(eval_in_bash(&stdout, LEVELS), "1||1\n", "{stdout}");
    assert!(!stdout.contains("ACCOUNT"), "{stdout}");
    assert!(
        stderr.starts_with(&format!("pathrc: {}: line 2, column ", bad.display()))
            && stderr.contains("quote it"),
        "{stderr}"
    );

    let listed = env
        .pathrc(root.path(), &inner)
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(
        listed.contains(&format!("{}  [invalid: line 2", bad.display())),
        "{listed}"
    );
}

#[test]
fn collate_prints_nothing_while_the_files_in_scope_are_unchanged() {
    let env = Env::new();
    let (root, _, inner) = three_levels(&env);
    let output = env
        .pathrc(root.path(), &inner)
        .arg("collate")
        .output()
        .unwrap();
    let first = String::from_utf8(output.stdout).unwrap();
    // The state the shell records is the quoted argument after the index.
    let loaded = first
        .split_once("__pathrc_load 0 '")
        .and_then(|(_, rest)| rest.split_once("' '"))
        .map(|(state, _)| state.to_string())
        .expect("a load line");
    assert_eq!(loaded.lines().count(), 3, "{loaded}");
    let again = env
        .pathrc(root.path(), &inner)
        .env("PATHRC_LOADED", &loaded)
        .arg("collate")
        .output()
        .unwrap();
    assert!(
        again.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&again.stdout)
    );
}

#[test]
fn old_format_files_are_not_read_but_are_listed() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(dir.path().join(".pathrc"), "export OLD=1\n").unwrap();
    let (stdout, _) = env.collate(dir.path(), dir.path());
    assert!(stdout.is_empty(), "{stdout}");
    let listed = env
        .pathrc(dir.path(), dir.path())
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(
        listed.contains(&format!(
            "{}  [old format: convert it to .pathrc.toml]",
            dir.path().join(".pathrc").display()
        )),
        "{listed}"
    );
}
