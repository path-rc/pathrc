//! age encryption: identities, recipients, and the two directions.
//!
//! Decrypted text only ever lives in memory (`Zeroizing`, wiped on drop).

use age::cli_common::{read_identities, read_recipients, StdinGuard};
use age::{Identity, Recipient};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// The recipients file looked for up the hierarchy when encrypting.
pub const RECIPIENTS: &str = ".pathrc-recipients";

/// Plaintext, wiped from memory when dropped.
pub type Plain = Zeroizing<Vec<u8>>;

/// The identity file: `PATHRC_IDENTITY`, then
/// `$XDG_CONFIG_HOME/pathrc/identity`, then `~/.ssh/id_ed25519`.
pub fn identity_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("PATHRC_IDENTITY").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!("PATHRC_IDENTITY {}: no such file", path.display()))
        };
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let candidates = [
        config.map(|c| c.join("pathrc").join("identity")),
        home.map(|h| h.join(".ssh").join("id_ed25519")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "no age identity: set PATHRC_IDENTITY, or create ~/.config/pathrc/identity \
             (age-keygen -o ~/.config/pathrc/identity), or use ~/.ssh/id_ed25519"
                .to_string()
        })
}

/// The user's identities, read once per run. Passphrase-protected ones
/// prompt on the terminal.
pub struct Identities(Vec<Box<dyn Identity>>);

impl Identities {
    pub fn load() -> Result<Identities, String> {
        let path = identity_path()?;
        let name = path.to_string_lossy().into_owned();
        read_identities(vec![name.clone()], None, &mut StdinGuard::new(false))
            .map(Identities)
            .map_err(|error| format!("identity {name}: {error}"))
    }

    /// Decrypts an armored or binary age file held in memory.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Plain, String> {
        let reader = age::armor::ArmoredReader::new(ciphertext);
        let decryptor = age::Decryptor::new(reader).map_err(|e| e.to_string())?;
        let mut stream = decryptor
            .decrypt(self.0.iter().map(|i| i.as_ref() as &dyn Identity))
            .map_err(|e| e.to_string())?;
        let mut plain = Zeroizing::new(Vec::new());
        stream.read_to_end(&mut plain).map_err(|e| e.to_string())?;
        Ok(plain)
    }
}

/// Who a file in `dir` is encrypted to: the nearest `.pathrc-recipients`
/// from `dir` up to `/` (age or SSH public keys, one per line, `#`
/// comments), else the public key of the user's own identity.
pub fn recipients_for(dir: &Path) -> Result<(Vec<Box<dyn Recipient + Send>>, String), String> {
    let mut guard = StdinGuard::new(false);
    let nearest = dir
        .ancestors()
        .map(|d| d.join(RECIPIENTS))
        .find(|p| p.is_file());
    let (files, identities, source) = match nearest {
        Some(file) => {
            let name = file.to_string_lossy().into_owned();
            (vec![name.clone()], vec![], name)
        }
        None => {
            let name = identity_path()?.to_string_lossy().into_owned();
            (vec![], vec![name.clone()], format!("your identity {name}"))
        }
    };
    let recipients = read_recipients(vec![], files, identities, None, &mut guard)
        .map_err(|error| format!("recipients from {source}: {error}"))?;
    if recipients.is_empty() {
        return Err(format!("no recipients in {source}"));
    }
    Ok((recipients, source))
}

/// Encrypts to `recipients`, ASCII-armored so it diffs sanely in git.
pub fn encrypt(plain: &[u8], recipients: &[Box<dyn Recipient + Send>]) -> Result<Vec<u8>, String> {
    let encryptor =
        age::Encryptor::with_recipients(recipients.iter().map(|r| r.as_ref() as &dyn Recipient))
            .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let armor = age::armor::ArmoredWriter::wrap_output(&mut out, age::armor::Format::AsciiArmor)
        .map_err(|e| e.to_string())?;
    let mut writer = encryptor.wrap_output(armor).map_err(|e| e.to_string())?;
    writer.write_all(plain).map_err(|e| e.to_string())?;
    writer
        .finish()
        .and_then(|armor| armor.finish())
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_an_x25519_identity() {
        let identity = age::x25519::Identity::generate();
        let recipients: Vec<Box<dyn Recipient + Send>> = vec![Box::new(identity.to_public())];
        let sealed = encrypt(b"export SECRET=1\n", &recipients).unwrap();
        assert!(sealed.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));

        let identities = Identities(vec![Box::new(identity)]);
        assert_eq!(
            &identities.decrypt(&sealed).unwrap()[..],
            b"export SECRET=1\n"
        );
    }

    #[test]
    fn the_wrong_identity_is_an_error_not_a_panic() {
        let right = age::x25519::Identity::generate();
        let recipients: Vec<Box<dyn Recipient + Send>> = vec![Box::new(right.to_public())];
        let sealed = encrypt(b"x", &recipients).unwrap();
        let wrong = Identities(vec![Box::new(age::x25519::Identity::generate())]);
        assert!(wrong.decrypt(&sealed).is_err());
        assert!(wrong.decrypt(b"not age at all").is_err());
    }
}
