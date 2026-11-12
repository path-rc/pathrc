//! Encrypted `.pathrc.toml.age` files and `.enc` values in `collate`,
//! `list` and `show`.

use age::secrecy::ExposeSecret;
use assert_cmd::Command;
use std::fs;
use std::io::Write;
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
    identity: PathBuf,
    recipient: Box<dyn age::Recipient + Send>,
}

impl Env {
    /// A scratch HOME with a fresh age X25519 identity in PATHRC_IDENTITY.
    fn new() -> Env {
        let home = tempdir();
        let key = age::x25519::Identity::generate();
        let identity = home.path().join("identity.txt");
        fs::write(&identity, format!("{}\n", key.to_string().expose_secret())).unwrap();
        Env {
            home,
            identity,
            recipient: Box::new(key.to_public()),
        }
    }

    fn pathrc(&self, start: &Path, cwd: &Path) -> Command {
        let mut cmd = Command::cargo_bin("pathrc").unwrap();
        cmd.current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("PATHRC_IDENTITY", &self.identity)
            .env("PATHRC_SHELL", "bash")
            .env("PATHRC_START", start);
        cmd
    }

    fn seal(&self, dir: &Path, plain: &str) -> PathBuf {
        seal_to(dir, plain, self.recipient.as_ref())
    }

    fn allow_all(&self, dir: &Path) {
        self.pathrc(dir, dir).arg("allow").assert().success();
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

/// The variables the mixed tree sets, in one line.
const CHECK: &str = "echo \"$LAST|$WITHIN|$TOP|${MID-}|${IN-}|${TOP_SECRET-}|${ZSH_ONLY-}\"";

fn seal_to(dir: &Path, plain: &str, recipient: &dyn age::Recipient) -> PathBuf {
    let encryptor = age::Encryptor::with_recipients(std::iter::once(recipient)).unwrap();
    let mut out = Vec::new();
    let armor =
        age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor).unwrap();
    let mut writer = encryptor.wrap_output(armor).unwrap();
    writer.write_all(plain.as_bytes()).unwrap();
    writer.finish().unwrap().finish().unwrap();
    let path = dir.join(".pathrc.toml.age");
    fs::write(&path, out).unwrap();
    path
}

/// top: plain + age, top/mid: age only, top/mid/in: plain only.
fn mixed_tree(env: &Env) -> (tempfile::TempDir, PathBuf) {
    let root = tempdir();
    let mid = root.path().join("mid");
    let inner = mid.join("in");
    fs::create_dir_all(&inner).unwrap();
    // Each file overrides LAST; WITHIN shows .age comes after plain.
    fs::write(
        root.path().join(".pathrc.toml"),
        "[common]\nenv.LAST = \"top-plain\"\nenv.WITHIN = \"plain\"\nenv.TOP = \"top-plain\"\n",
    )
    .unwrap();
    env.seal(
        root.path(),
        "[common]\nenv.LAST = \"top-age\"\nenv.WITHIN = \"age\"\n[bash]\nenv.TOP_SECRET = \"s1\"\n[zsh]\nenv.ZSH_ONLY = \"no\"\n",
    );
    env.seal(
        &mid,
        "[common]\nenv.LAST = \"mid-age\"\nenv.MID = \"mid-age\"\n",
    );
    fs::write(
        inner.join(".pathrc.toml"),
        "[common]\nenv.LAST = \"in-plain\"\nenv.IN = \"in-plain\"\n",
    )
    .unwrap();
    for dir in [root.path(), mid.as_path(), inner.as_path()] {
        env.allow_all(dir);
    }
    (root, inner)
}

#[test]
fn encrypted_files_at_any_level_are_decrypted_in_order() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);
    let (stdout, stderr) = env.collate(root.path(), &inner);
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(
        eval_in_bash(&stdout, CHECK),
        "in-plain|age|top-plain|mid-age|in-plain|s1|\n",
        "in order, and shell tables apply inside .age files too"
    );
}

#[test]
fn list_marks_encrypted_files_without_decrypting() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);
    // No identity at all: list must still work, so it isn't decrypting.
    let output = env
        .pathrc(root.path(), &inner)
        .env("PATHRC_IDENTITY", root.path().join("missing"))
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success());
    assert!(
        listed.contains(&format!(
            "{}  [age]",
            root.path().join(".pathrc.toml.age").display()
        )),
        "{listed}"
    );
    assert!(
        listed.contains(&format!("{}\n", root.path().join(".pathrc.toml").display())),
        "{listed}"
    );
}

#[test]
fn a_wrong_or_missing_identity_reports_and_still_loads_plain_files() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);

    let wrong = env.home.path().join("wrong.txt");
    let other = age::x25519::Identity::generate();
    fs::write(&wrong, format!("{}\n", other.to_string().expose_secret())).unwrap();
    for identity in [wrong, env.home.path().join("missing")] {
        let output = env
            .pathrc(root.path(), &inner)
            .env("PATHRC_IDENTITY", &identity)
            .arg("collate")
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            eval_in_bash(&stdout, CHECK),
            "in-plain|plain|top-plain||in-plain||\n",
            "{stdout}"
        );
        assert_eq!(stderr.matches("cannot decrypt").count(), 2, "{stderr}");
    }
}

#[test]
fn nothing_is_written_while_decrypting() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);
    // Every directory involved read-only, TMPDIR included: any attempt to
    // write plaintext (a temp file, a cache) would fail and show on stderr.
    let mut dirs: Vec<PathBuf> = inner.ancestors().take(3).map(PathBuf::from).collect();
    dirs.push(env.home.path().into());
    for dir in &dirs {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let (stdout, stderr) = {
        let output = env
            .pathrc(root.path(), &inner)
            .env("TMPDIR", root.path())
            .arg("collate")
            .output()
            .unwrap();
        (
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    for dir in &dirs {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(stderr.is_empty(), "{stderr}");
    assert!(eval_in_bash(&stdout, CHECK).ends_with("|s1|\n"), "{stdout}");
}

#[test]
fn an_ssh_ed25519_key_works_as_the_identity() {
    let env = Env::new();
    let key = env.home.path().join("id_ed25519");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "pathrc-test", "-f"])
        .arg(&key)
        .status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("ssh-keygen not available; skipping");
        return;
    }
    let public = fs::read_to_string(key.with_extension("pub")).unwrap();
    let recipient: age::ssh::Recipient = public.trim().parse().unwrap();
    let dir = tempdir();
    seal_to(dir.path(), "[common]\nenv.VIA_SSH = \"yes\"\n", &recipient);
    env.allow_all(dir.path());

    // Found as ~/.ssh/id_ed25519 with no PATHRC_IDENTITY.
    fs::create_dir_all(env.home.path().join(".ssh")).unwrap();
    fs::rename(&key, env.home.path().join(".ssh/id_ed25519")).unwrap();
    let output = env
        .pathrc(dir.path(), dir.path())
        .env_remove("PATHRC_IDENTITY")
        .arg("collate")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        eval_in_bash(&stdout, "echo \"$VIA_SSH\""),
        "yes\n",
        "{stdout}"
    );
}

#[test]
fn show_prints_the_decrypted_cascade_for_a_shell() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);
    let output = env
        .pathrc(root.path(), &inner)
        .arg("show")
        .output()
        .unwrap();
    assert!(output.status.success());
    let shown = String::from_utf8(output.stdout).unwrap();
    assert!(shown.starts_with(&format!(
        "# File: {}\n",
        root.path().join(".pathrc.toml").display()
    )));
    // Evaluable as it stands.
    let script = format!("{shown}\n{CHECK}");
    let evaluated = std::process::Command::new("bash")
        .args(["-c", &script])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(evaluated.stdout).unwrap(),
        "in-plain|age|top-plain|mid-age|in-plain|s1|\n"
    );

    let zsh = env
        .pathrc(root.path(), &inner)
        .args(["show", "--shell", "zsh"])
        .output()
        .unwrap();
    let zsh = String::from_utf8(zsh.stdout).unwrap();
    assert!(
        zsh.contains("ZSH_ONLY") && !zsh.contains("TOP_SECRET"),
        "{zsh}"
    );
}

#[test]
fn show_fails_when_a_file_is_left_out() {
    let env = Env::new();
    let (root, inner) = mixed_tree(&env);
    env.pathrc(&inner, &inner).arg("deny").assert().success();
    let output = env
        .pathrc(root.path(), &inner)
        .arg("show")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let shown = String::from_utf8(output.stdout).unwrap();
    assert!(
        shown.contains("mid-age") && !shown.contains("in-plain"),
        "{shown}"
    );
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("not allowed"));

    let uninit = env
        .pathrc(root.path(), &inner)
        .env_remove("PATHRC_SHELL")
        .arg("show")
        .output()
        .unwrap();
    assert!(!uninit.status.success());
}

/// One value for a `.enc` table: an age message for `recipient`, unarmored,
/// as one line of base64.
fn seal_value(plain: &str, recipient: &dyn age::Recipient) -> String {
    use base64::Engine;
    let encryptor = age::Encryptor::with_recipients(std::iter::once(recipient)).unwrap();
    let mut out = Vec::new();
    let mut writer = encryptor.wrap_output(&mut out).unwrap();
    writer.write_all(plain.as_bytes()).unwrap();
    writer.finish().unwrap();
    base64::engine::general_purpose::STANDARD.encode(out)
}

/// A .pathrc.toml with plain settings and a [common.enc] table.
fn enc_file(env: &Env, dir: &Path) -> PathBuf {
    let r = env.recipient.as_ref();
    let file = dir.join(".pathrc.toml");
    fs::write(
        &file,
        format!(
            "[common]\nenv.PLAIN = \"plain\"\nenv.SECRET = \"default\"\n\
             alias.api = \"curl -H \\\"Authorization: Bearer $TOKEN\\\"\"\n\
             [common.enc]\nenv.SECRET = \"{}\"\nvar.TOKEN = \"{}\"\n",
            seal_value("from-enc", r),
            seal_value("t0ken 'quoted'", r),
        ),
    )
    .unwrap();
    env.allow_all(dir);
    file
}

#[test]
fn enc_values_are_decrypted_and_a_var_is_not_exported() {
    let env = Env::new();
    let dir = tempdir();
    enc_file(&env, dir.path());
    let (stdout, stderr) = env.collate(dir.path(), dir.path());
    assert!(stderr.is_empty(), "{stderr}");
    assert_eq!(
        eval_in_bash(
            &stdout,
            "echo \"$PLAIN|$SECRET|$TOKEN|$(sh -c 'echo ${TOKEN-not exported}')|$(alias api)\""
        ),
        "plain|from-enc|t0ken 'quoted'|not exported|alias api='curl -H \"Authorization: Bearer $TOKEN\"'\n"
    );
}

#[test]
fn a_wrong_identity_names_the_value_and_skips_the_file() {
    let env = Env::new();
    let dir = tempdir();
    let file = enc_file(&env, dir.path());
    let wrong = env.home.path().join("wrong.txt");
    let other = age::x25519::Identity::generate();
    fs::write(&wrong, format!("{}\n", other.to_string().expose_secret())).unwrap();
    let output = env
        .pathrc(dir.path(), dir.path())
        .env("PATHRC_IDENTITY", &wrong)
        .arg("collate")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with(&format!(
            "pathrc: {}: [common.enc] env.SECRET: cannot decrypt",
            file.display()
        )),
        "{stderr}"
    );
    // A half-applied file would be confusing: none of it is applied.
    assert!(output.stdout.is_empty());
}

#[test]
fn list_counts_enc_values_without_decrypting_and_show_decrypts_them() {
    let env = Env::new();
    let dir = tempdir();
    let file = enc_file(&env, dir.path());
    let listed = env
        .pathrc(dir.path(), dir.path())
        .env("PATHRC_IDENTITY", dir.path().join("missing"))
        .arg("list")
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert!(
        listed.contains(&format!("{}  [2 encrypted values]", file.display())),
        "{listed}"
    );
    let shown = env
        .pathrc(dir.path(), dir.path())
        .arg("show")
        .output()
        .unwrap();
    assert!(shown.status.success());
    let shown = String::from_utf8(shown.stdout).unwrap();
    assert!(
        shown.contains("export SECRET='from-enc'") && shown.contains("unset TOKEN\nTOKEN="),
        "{shown}"
    );
}
