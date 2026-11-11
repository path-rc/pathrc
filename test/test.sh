#!/usr/bin/env bash
# End-to-end tests: real shells, a real `cd` hook, real .pathrc files.
#
#   bash test/test.sh [path/to/pathrc]
#
# Runs every shell that is installed (bash, zsh, fish) and skips the rest.
# HOME and the XDG directories point into a scratch directory, never yours.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bin=${1:-$repo/target/debug/pathrc}
[[ -x $bin ]] || { echo "no pathrc binary at $bin; run cargo build" >&2; exit 2; }

scratch=$(mktemp -d)
trap 'chmod -R u+rwx "$scratch" 2>/dev/null; rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/home"
ln -s "$bin" "$scratch/bin/pathrc"
export PATH="$scratch/bin:$PATH" HOME="$scratch/home"
export XDG_CONFIG_HOME="$scratch/home/.config" XDG_DATA_HOME="$scratch/home/.local/share"
export XDG_RUNTIME_DIR="$scratch/run"
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
unset PATHRC_SHELL PATHRC_START PATHRC_LOADED

pass=0 fail=0
check() { # name expected actual
  if [[ $2 == "$3" ]]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    printf 'FAIL %s\n  expected: %q\n  actual:   %q\n' "$1" "$2" "$3"
  fi
}

# A tree: top/.pathrc, top/project/.pathrc, top/project/sub/.pathrc
tree=$scratch/top
mkdir -p "$tree/project/sub"
cat > "$tree/.pathrc" <<'PATHRC'
export LEVEL=top
#sh
export TOP_SH=yes
#fish
set -gx TOP_FISH yes
PATHRC
cat > "$tree/project/.pathrc" <<'PATHRC'
#bash
export LEVEL=project-bash
#zsh
export LEVEL=project-zsh
#fish
set -gx LEVEL project-fish
PATHRC
printf 'export LEVEL=sub\nexport SUB=yes' > "$tree/project/sub/.pathrc" # no final newline
pathrc allow "$tree/.pathrc" "$tree/project/.pathrc" "$tree/project/sub/.pathrc" > /dev/null

run_posix() { # shell init-arg script
  (cd "$tree" && "$1" -c "eval \"\$(pathrc init $2)\"; $3" 2>&1)
}

if command -v bash > /dev/null; then
  check "bash: nested cd picks the bash section" "project-bash yes" \
    "$(run_posix bash bash 'cd project; echo "$LEVEL $TOP_SH"')"
  check "bash: innermost wins, missing newline ok" "sub yes yes" \
    "$(run_posix bash bash 'cd project/sub; echo "$LEVEL $SUB $TOP_SH"')"
  check "sh: shared init reads common and #sh only" "top yes" \
    "$(run_posix bash sh 'cd project; echo "$LEVEL $TOP_SH"')"
else
  echo "skip: bash not installed"
fi

if command -v zsh > /dev/null; then
  check "zsh: nested cd picks the zsh section" "project-zsh yes" \
    "$(run_posix zsh zsh 'cd project; echo "$LEVEL $TOP_SH"')"
else
  echo "skip: zsh not installed"
fi

if command -v fish > /dev/null; then
  check "fish: nested cd picks the fish section, not #sh" "project-fish yes -" \
    "$(cd "$tree" && fish --no-config -c 'pathrc init fish | source; cd project; echo "$LEVEL $TOP_FISH -$TOP_SH"' 2>&1)"
else
  echo "skip: fish not installed"
fi

# collate never breaks the hook: an unreadable file is reported, the rest load.
if [[ $(id -u) != 0 ]]; then
  chmod 000 "$tree/project/.pathrc"
  check "bash: unreadable middle file is skipped" "sub yes" \
    "$(run_posix bash bash 'cd project/sub 2>/dev/null; echo "$LEVEL $TOP_SH"')"
  chmod 644 "$tree/project/.pathrc"
fi

# Nothing is evaluated until allowed, and a change needs allowing again.
mkdir -p "$tree/cloned"
echo 'export CLONED=ran' > "$tree/cloned/.pathrc"
check "bash: an untrusted file is not evaluated" "-" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"
(cd "$tree/cloned" && pathrc allow > /dev/null)
check "bash: it runs once allowed" "-ran" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"
echo 'export CLONED=changed' > "$tree/cloned/.pathrc"
check "bash: a changed file is not evaluated" "-" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"

echo "$pass passed, $fail failed"
[[ $fail == 0 ]]
