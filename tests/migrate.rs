//! `pathrc migrate`: older files into a `.pathrc.toml`.

use age::secrecy::ExposeSecret;
use assert_cmd::Command;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A temporary directory named by its resolved path (macOS's /var is a
/// symlink to /private/var, and pathrc reports resolved paths).
fn tempdir() -> tempfile::TempDir {
    let base = std::env::temp_dir().canonicalize().unwrap();
    tempfile::tempdir_in(base).unwrap()
}

struct Env {
    home: tempfile::TempDir,
    identity: PathBuf,
    key: age::x25519::Identity,
}

impl Env {
    fn new() -> Env {
        let home = tempdir();
        let key = age::x25519::Identity::generate();
        let identity = home.path().join("identity.txt");
        fs::write(&identity, format!("{}\n", key.to_string().expose_secret())).unwrap();
        Env {
            home,
            identity,
            key,
        }
    }

    fn pathrc(&self, cwd: &Path) -> Command {
        let mut cmd = Command::cargo_bin("pathrc").unwrap();
        cmd.current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("PATHRC_IDENTITY", &self.identity)
            .env("PATHRC_SHELL", "bash")
            .env("PATHRC_START", cwd);
        cmd
    }

    /// An old armored whole-file age file, as pathrc 0.1 to 0.5 wrote.
    fn seal_file(&self, path: &Path, plain: &str) {
        let recipient = self.key.to_public();
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .unwrap();
        let mut out = Vec::new();
        let armor =
            age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor)
                .unwrap();
        let mut writer = encryptor.wrap_output(armor).unwrap();
        writer.write_all(plain.as_bytes()).unwrap();
        writer.finish().unwrap().finish().unwrap();
        fs::write(path, out).unwrap();
    }

    /// The plaintext of a `.enc` value.
    fn open(&self, value: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(value)
            .unwrap();
        let decryptor = age::Decryptor::new(&bytes[..]).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&self.key as &dyn age::Identity))
            .unwrap();
        let mut out = String::new();
        reader.read_to_string(&mut out).unwrap();
        out
    }

    /// What the cd hook sets in `dir`, as `echo`ed by `then`.
    fn loaded(&self, dir: &Path, then: &str) -> String {
        let collated = self.pathrc(dir).arg("collate").output().unwrap();
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

fn toml_at(file: &Path) -> toml::Table {
    toml::from_str(&fs::read_to_string(file).unwrap()).unwrap()
}

#[test]
fn all_three_old_files_become_one_pathrc_toml() {
    let env = Env::new();
    let dir = tempdir();
    let d = dir.path();
    fs::write(
        d.join(".pathrc"),
        "export REGION=eu\nalias ll='ls -l'\n#bash\nexport BASH_ONLY=1\n#fish\nset -gx FISH_ONLY 1\n",
    )
    .unwrap();
    env.seal_file(
        &d.join(".pathrc.age"),
        "export TOKEN='s3cret'\n#sh\nSHELL_VAR=hidden\n",
    );
    env.seal_file(
        &d.join(".pathrc.toml.age"),
        "[common]\nenv.ACCOUNT = \"111111111111\"\npath.prepend = [\"bin\"]\n",
    );
    for name in [".pathrc", ".pathrc.age", ".pathrc.toml.age"] {
        env.pathrc(d).args(["allow", name]).assert().success();
    }

    let out = env.pathrc(d).arg("migrate").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with("converted .pathrc, .pathrc.age and .pathrc.toml.age into "),
        "{stdout}"
    );
    assert!(!stdout.contains("not allowed"), "{stdout}");

    let file = d.join(".pathrc.toml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains("s3cret") && !text.contains("111111111111"),
        "{text}"
    );
    let doc = toml_at(&file);
    assert_eq!(doc["common"]["env"]["REGION"].as_str(), Some("eu"));
    assert_eq!(doc["common"]["alias"]["ll"].as_str(), Some("ls -l"));
    assert_eq!(doc["bash"]["env"]["BASH_ONLY"].as_str(), Some("1"));
    assert_eq!(doc["fish"]["env"]["FISH_ONLY"].as_str(), Some("1"));
    assert_eq!(doc["common"]["path"]["prepend"][0].as_str(), Some("bin"));
    let enc = |table: &str, kind: &str, name: &str| {
        env.open(doc[table]["enc"][kind][name].as_str().unwrap())
    };
    assert_eq!(enc("common", "env", "TOKEN"), "s3cret");
    assert_eq!(enc("sh", "var", "SHELL_VAR"), "hidden");
    assert_eq!(enc("common", "env", "ACCOUNT"), "111111111111");
    assert_eq!(
        env.loaded(
            d,
            "echo \"$REGION|$TOKEN|$SHELL_VAR|$ACCOUNT|$BASH_ONLY|${FISH_ONLY-}\""
        ),
        "eu|s3cret|hidden|111111111111|1|\n",
        "allowed, since every old file was"
    );
    // The old files are left where they were.
    assert!(d.join(".pathrc").exists() && d.join(".pathrc.age").exists());
}

#[test]
fn a_line_only_a_shell_can_run_stops_the_conversion() {
    let env = Env::new();
    let dir = tempdir();
    fs::write(
        dir.path().join(".pathrc"),
        "export A=1\nexport B=\"$HOME/b\"\nmkdir -p build\n",
    )
    .unwrap();
    let out = env.pathrc(dir.path()).arg("migrate").output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("line 2: a value that expands"), "{stderr}");
    assert!(stderr.contains("line 3: 'mkdir' is a command"), "{stderr}");
    assert!(stderr.contains("nothing converted"), "{stderr}");
    assert!(!dir.path().join(".pathrc.toml").exists());
}

#[test]
fn a_key_already_set_stops_it_and_an_unallowed_source_isnt_allowed() {
    let env = Env::new();
    let dir = tempdir();
    let d = dir.path();
    fs::write(d.join(".pathrc.toml"), "[common]\nenv.A = \"mine\"\n").unwrap();
    fs::write(d.join(".pathrc"), "export A=old\n").unwrap();
    let out = env.pathrc(d).arg("migrate").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8(out.stderr)
        .unwrap()
        .contains("already set in .pathrc.toml: [common] env.A"));
    assert_eq!(
        fs::read_to_string(d.join(".pathrc.toml")).unwrap(),
        "[common]\nenv.A = \"mine\"\n"
    );

    // No conflict, but the old file was never allowed: the result isn't.
    fs::write(d.join(".pathrc"), "export B=old\n").unwrap();
    let out = env.pathrc(d).arg("migrate").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .contains("not allowed yet"));
    assert_eq!(env.loaded(d, "echo \"-${A-}${B-}\""), "-\n");
}

#[test]
fn nothing_to_migrate_says_so() {
    let env = Env::new();
    let dir = tempdir();
    let out = env.pathrc(dir.path()).arg("migrate").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout)
        .unwrap()
        .contains("no old pathrc files"));
}
