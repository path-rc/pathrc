#!/bin/sh
# Installs pathrc, the path-rc command, from its GitHub releases.
#
#   curl -fsSL https://path-rc.com/install.sh | sh
#
# PATHRC_VERSION       a release such as v0.3.0 (default: the latest)
# PATHRC_INSTALL_DIR   where to put pathrc (default: /usr/local/bin)
# PATHRC_DOWNLOAD_URL  where the release files are, instead of GitHub
#                      (a mirror, or file:// for testing)
#
# It picks the archive for your OS and CPU (Linux x86-64 or arm64, macOS
# Intel or Apple Silicon), checks it against its SHA-256 checksum, and
# installs the binary, using sudo only if the directory needs it.

set -eu

repo=path-rc/pathrc
dir=${PATHRC_INSTALL_DIR:-/usr/local/bin}
version=${PATHRC_VERSION:-latest}

say() { printf 'pathrc install: %s\n' "$*" >&2; }
fail() {
  say "$*"
  exit 1
}

case $(uname -s) in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) fail "no release for $(uname -s); build from source: cargo install --git https://github.com/$repo" ;;
esac
case $(uname -m) in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "no release for $(uname -m); build from source: cargo install --git https://github.com/$repo" ;;
esac
# A shell running under Rosetta reports x86_64 on Apple Silicon.
if [ "$os" = apple-darwin ] && [ "$arch" = x86_64 ] &&
  [ "$(sysctl -n hw.optional.arm64 2> /dev/null || true)" = 1 ]; then
  arch=aarch64
fi
archive=pathrc-$arch-$os.tar.gz

if [ -n "${PATHRC_DOWNLOAD_URL:-}" ]; then
  base=${PATHRC_DOWNLOAD_URL%/}
elif [ "$version" = latest ]; then
  base=https://github.com/$repo/releases/latest/download
else
  base=https://github.com/$repo/releases/download/v${version#v}
fi

fetch() { # url file
  if command -v curl > /dev/null 2>&1; then
    curl -fsSL -o "$2" "$1"
  elif command -v wget > /dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    fail "needs curl or wget"
  fi
}

tmp=$(mktemp -d 2> /dev/null || mktemp -d -t pathrc)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $archive ($version)"
fetch "$base/$archive" "$tmp/$archive" || fail "cannot download $base/$archive"
fetch "$base/$archive.sha256" "$tmp/$archive.sha256" || fail "cannot download its checksum"

expected=$(cut -d ' ' -f 1 < "$tmp/$archive.sha256")
if command -v sha256sum > /dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$archive" | cut -d ' ' -f 1)
elif command -v shasum > /dev/null 2>&1; then
  actual=$(shasum -a 256 "$tmp/$archive" | cut -d ' ' -f 1)
else
  fail "needs sha256sum or shasum to check the download"
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] ||
  fail "checksum mismatch for $archive (expected $expected, got $actual)"

tar -xzf "$tmp/$archive" -C "$tmp" pathrc

if [ -d "$dir" ] && [ -w "$dir" ] || { [ ! -e "$dir" ] && mkdir -p "$dir" 2> /dev/null; }; then
  sudo=
elif command -v sudo > /dev/null 2>&1; then
  say "$dir is not writable; using sudo"
  sudo=sudo
else
  fail "cannot write to $dir; set PATHRC_INSTALL_DIR, e.g. PATHRC_INSTALL_DIR=\$HOME/.local/bin"
fi
$sudo mkdir -p "$dir"
$sudo install -m 755 "$tmp/pathrc" "$dir/pathrc"

say "installed $("$dir/pathrc" --version) to $dir/pathrc"
case :$PATH: in
  *":$dir:"*) ;;
  *) say "$dir is not on your PATH; add it, or set PATHRC_INSTALL_DIR" ;;
esac
say "next: add eval \"\$(pathrc init)\" to your shell's rc file (fish: pathrc init fish | source)"
