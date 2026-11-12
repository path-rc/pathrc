//! `.enc` values inside a `.pathrc.toml`'s text (RFC 0003): shown decrypted
//! for `edit`, encrypted again on save, added by `secret`, re-encrypted by
//! `rekey`. The text is edited with `toml_edit`, so the file keeps its
//! layout and comments; only the values change.

use crate::crypto::Plain;
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};
use zeroize::Zeroizing;

/// The tables that can have an `.enc`, and the keys an `.enc` holds.
pub const TABLES: [&str; 5] = ["common", "sh", "bash", "zsh", "fish"];
const KINDS: [&str; 2] = ["env", "var"];

/// One `.enc` value as revealed for editing.
pub struct Revealed {
    table: &'static str,
    kind: &'static str,
    name: String,
    plain: Zeroizing<String>,
    sealed: String,
}

fn document(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| e.to_string())
}

fn kind_table_mut<'a>(
    doc: &'a mut DocumentMut,
    table: &str,
    kind: &str,
) -> Option<&'a mut dyn TableLike> {
    doc.get_mut(table)?
        .as_table_like_mut()?
        .get_mut("enc")?
        .as_table_like_mut()?
        .get_mut(kind)?
        .as_table_like_mut()
}

/// Replaces a string value, keeping its spacing and trailing comment.
fn replace(value: &mut Value, text: &str) {
    let decor = value.decor().clone();
    *value = Value::from(text);
    *value.decor_mut() = decor;
}

/// Every `.enc` value in `doc`, with a closure that may replace it.
fn each_value(
    doc: &mut DocumentMut,
    mut f: impl FnMut(&'static str, &'static str, &str, &mut Value) -> Result<(), String>,
) -> Result<(), String> {
    for table in TABLES {
        for kind in KINDS {
            let Some(values) = kind_table_mut(doc, table, kind) else {
                continue;
            };
            for (name, item) in values.iter_mut() {
                if let Some(value) = item.as_value_mut() {
                    f(table, kind, name.get(), value)?;
                }
            }
        }
    }
    Ok(())
}

/// `text` with every `.enc` value decrypted, and what each was.
pub fn reveal(
    text: &str,
    decrypt: &mut dyn FnMut(&str) -> Result<Plain, String>,
) -> Result<(Zeroizing<String>, Vec<Revealed>), String> {
    let mut doc = document(text)?;
    let mut revealed = Vec::new();
    each_value(&mut doc, |table, kind, name, value| {
        let Some(sealed) = value.as_str().map(String::from) else {
            return Ok(());
        };
        let plain = decrypt(&sealed).map_err(|e| format!("[{table}.enc] {kind}.{name}: {e}"))?;
        let plain =
            Zeroizing::new(String::from_utf8(plain.to_vec()).map_err(|_| {
                format!("[{table}.enc] {kind}.{name}: decrypted, but not UTF-8 text")
            })?);
        replace(value, &plain);
        revealed.push(Revealed {
            table,
            kind,
            name: name.to_string(),
            plain,
            sealed,
        });
        Ok(())
    })?;
    Ok((Zeroizing::new(doc.to_string()), revealed))
}

/// `text` (as edited, `.enc` values in plain text) with every `.enc` value
/// encrypted: unchanged values get their old ciphertext back, so the file's
/// diff shows only what changed; new and changed ones go through `seal`.
/// Returns the text and how many values were encrypted afresh.
pub fn seal(
    text: &str,
    revealed: &[Revealed],
    seal: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let mut doc = document(text)?;
    let mut fresh = 0;
    each_value(&mut doc, |table, kind, name, value| {
        let Some(plain) = value.as_str() else {
            return Ok(());
        };
        let unchanged = revealed
            .iter()
            .find(|r| r.table == table && r.kind == kind && r.name == name && *r.plain == plain);
        let sealed = match unchanged {
            Some(r) => r.sealed.clone(),
            None => {
                fresh += 1;
                seal(plain).map_err(|e| format!("[{table}.enc] {kind}.{name}: {e}"))?
            }
        };
        replace(value, &sealed);
        Ok(())
    })?;
    Ok((doc.to_string(), fresh))
}

/// `text` with every `.enc` value decrypted and encrypted again, and how
/// many there were.
pub fn rekey(
    text: &str,
    decrypt: &mut dyn FnMut(&str) -> Result<Plain, String>,
    seal: &mut dyn FnMut(&[u8]) -> Result<String, String>,
) -> Result<(String, usize), String> {
    let mut doc = document(text)?;
    let mut count = 0;
    each_value(&mut doc, |table, kind, name, value| {
        let Some(sealed) = value.as_str() else {
            return Ok(());
        };
        let what = |e: String| format!("[{table}.enc] {kind}.{name}: {e}");
        let plain = decrypt(sealed).map_err(what)?;
        let resealed = seal(&plain).map_err(what)?;
        replace(value, &resealed);
        count += 1;
        Ok(())
    })?;
    Ok((doc.to_string(), count))
}

/// A sub-table, made if missing: implicit for the shell table (so the file
/// gains `[common.enc]`, not an empty `[common]`), dotted for `env`/`var`
/// (so the entry reads `env.NAME = "…"`).
fn sub_table<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
    how: Make,
) -> Result<&'a mut dyn TableLike, String> {
    let item = parent.entry(key).or_insert_with(|| {
        let mut table = Table::new();
        match how {
            Make::Implicit => table.set_implicit(true),
            Make::Header => {}
            Make::Dotted => table.set_dotted(true),
        }
        Item::Table(table)
    });
    item.as_table_like_mut()
        .ok_or_else(|| format!("'{key}' is not a table here"))
}

#[derive(Clone, Copy)]
enum Make {
    Implicit,
    Header,
    Dotted,
}

/// `text` with `[table.enc] kind.name` set to `sealed`, replacing it if
/// there, and removing the same name of the other kind from that `.enc`
/// (a name is either exported or not).
pub fn set(
    text: &str,
    table: &str,
    kind: &str,
    name: &str,
    sealed: &str,
) -> Result<String, String> {
    let mut doc = document(text)?;
    let root = doc.as_table_mut() as &mut dyn TableLike;
    let shell = sub_table(root, table, Make::Implicit)?;
    let enc = sub_table(shell, "enc", Make::Header)?;
    let other = if kind == "env" { "var" } else { "env" };
    if let Some(other) = enc.get_mut(other).and_then(|i| i.as_table_like_mut()) {
        other.remove(name);
    }
    let values = sub_table(enc, kind, Make::Dotted)?;
    match values.get_mut(name).and_then(|i| i.as_value_mut()) {
        Some(value) => replace(value, sealed),
        None => {
            values.insert(name, Item::Value(Value::from(sealed)));
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Stand-ins for age: "sealed" text is "S(" + plain + ")".
    fn fake_decrypt(sealed: &str) -> Result<Plain, String> {
        let plain = sealed
            .strip_prefix("S(")
            .and_then(|s| s.strip_suffix(')'))
            .ok_or("not sealed")?;
        Ok(Zeroizing::new(plain.as_bytes().to_vec()))
    }

    const FILE: &str = "\
# a comment that stays
[common]
env.REGION = \"eu\"

[common.enc]
env.SECRET = \"S(one)\"   # trailing comment
var.TOKEN = \"S(two)\"

[zsh.enc]
env.Z = \"S(z)\"
";

    #[test]
    fn reveal_shows_values_in_place_keeping_the_layout() {
        let (text, revealed) = reveal(FILE, &mut fake_decrypt).unwrap();
        assert_eq!(revealed.len(), 3);
        assert_eq!(
            *text,
            FILE.replace("S(one)", "one")
                .replace("S(two)", "two")
                .replace("S(z)", "z")
        );
    }

    #[test]
    fn seal_keeps_the_ciphertext_of_unchanged_values() {
        let (text, revealed) = reveal(FILE, &mut fake_decrypt).unwrap();
        let edited = text.replace("\"two\"", "\"changed\"") + "\n[bash.enc]\nvar.NEW = \"fresh\"\n";
        let mut sealed_count = Vec::new();
        let (sealed, fresh) = seal(&edited, &revealed, &mut |plain| {
            sealed_count.push(plain.to_string());
            Ok(format!("N({plain})"))
        })
        .unwrap();
        assert_eq!(fresh, 2);
        assert_eq!(sealed_count, ["changed", "fresh"]);
        assert!(
            sealed.contains("env.SECRET = \"S(one)\"   # trailing comment"),
            "{sealed}"
        );
        assert!(sealed.contains("var.TOKEN = \"N(changed)\""), "{sealed}");
        assert!(sealed.contains("env.Z = \"S(z)\""), "{sealed}");
        assert!(sealed.contains("var.NEW = \"N(fresh)\""), "{sealed}");
        assert!(sealed.starts_with("# a comment that stays\n"));
    }

    #[test]
    fn rekey_reseals_every_value() {
        let (text, count) = rekey(FILE, &mut fake_decrypt, &mut |p| {
            Ok(format!("R({})", String::from_utf8_lossy(p)))
        })
        .unwrap();
        assert_eq!(count, 3);
        assert!(text.contains("env.SECRET = \"R(one)\"   # trailing comment"));
        assert!(text.contains("env.Z = \"R(z)\""));
    }

    #[test]
    fn set_adds_a_value_where_it_belongs() {
        let text = set(
            "[common]\nenv.REGION = \"eu\"\n",
            "common",
            "env",
            "SECRET",
            "S(x)",
        )
        .unwrap();
        assert_eq!(
            text,
            "[common]\nenv.REGION = \"eu\"\n\n[common.enc]\nenv.SECRET = \"S(x)\"\n"
        );
        // A new shell table appears only as its .enc header.
        let text = set("", "bash", "var", "T", "S(t)").unwrap();
        assert_eq!(text, "[bash.enc]\nvar.T = \"S(t)\"\n");
        // Replacing keeps the place; the other kind of the same name goes.
        let text = set(FILE, "common", "env", "TOKEN", "S(new)").unwrap();
        assert!(text.contains("env.TOKEN = \"S(new)\""), "{text}");
        assert!(!text.contains("var.TOKEN"), "{text}");
        let text = set(FILE, "common", "env", "SECRET", "S(again)").unwrap();
        assert!(
            text.contains("env.SECRET = \"S(again)\"   # trailing comment"),
            "{text}"
        );
    }
}
