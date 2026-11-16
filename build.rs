//! Sets `PATHRC_BUILD_VERSION`, what `pathrc --version` reports.
//!
//! Cargo.toml's version isn't bumped in commits: releases are tagged by the
//! release workflow. So the version comes from, in order:
//!
//! 1. `PATHRC_RELEASE_VERSION`, which the release workflow sets to the tag;
//! 2. `git describe` of the source: `0.8.0` on a release tag, or
//!    `0.8.0-2-gabc1234` two commits past it, with `-dirty` for uncommitted
//!    changes;
//! 3. `unknown, commit abc1234` from a checkout without the tags, such as
//!    `cargo install --git`'s;
//! 4. `unknown`, for a build with no git history to go on.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PATHRC_RELEASE_VERSION");
    let version = std::env::var("PATHRC_RELEASE_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .or_else(git_version)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=PATHRC_BUILD_VERSION={version}");
}

fn git_version() -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // Only pathrc's own repository: a copy of the source inside some other
    // repository (vendored, say) must not report that repository's tags.
    let top = git(&["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(&top).ok()? != std::fs::canonicalize(&dir).ok()? {
        return None;
    }
    // Rebuild when HEAD, the tags or the working tree's index change.
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for file in ["HEAD", "index", "packed-refs", "refs/tags"] {
            println!("cargo:rerun-if-changed={git_dir}/{file}");
        }
    }
    match git(&["describe", "--tags", "--match", "v[0-9]*", "--dirty"]) {
        Some(described) => Some(described.trim_start_matches('v').to_string()),
        // A checkout without the tags, such as `cargo install --git`'s:
        // the commit at least says what was built.
        None => git(&["rev-parse", "--short", "HEAD"]).map(|c| format!("unknown, commit {c}")),
    }
}
