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

# An encrypted file, made with the age CLI if it is installed.
if command -v age-keygen > /dev/null && command -v age > /dev/null; then
  mkdir -p "$XDG_CONFIG_HOME/pathrc" "$tree/secret"
  age-keygen -o "$XDG_CONFIG_HOME/pathrc/identity" 2> /dev/null
  printf 'export TOKEN=from-age\n#zsh\nexport TOKEN=zsh-age\n' |
    age -a -r "$(age-keygen -y "$XDG_CONFIG_HOME/pathrc/identity")" > "$tree/secret/.pathrc.age"
  (cd "$tree/secret" && pathrc allow > /dev/null)
  check "bash: .pathrc.age is decrypted" "from-age" \
    "$(run_posix bash bash 'cd secret; echo "$TOKEN"')"
  if command -v zsh > /dev/null; then
    check "zsh: sections apply inside .pathrc.age" "zsh-age" \
      "$(run_posix zsh zsh 'cd secret; echo "$TOKEN"')"
  fi
else
  echo "skip: age CLI not installed"
fi

# Unloading on leaving scope (bash and zsh): a three-level tree mixing plain
# and encrypted files, siblings, an inner override of an outer variable, and
# aliases and functions.
uw=$scratch/unload
mkdir -p "$uw/client/api" "$uw/client/web" "$uw/other"
cat > "$uw/.pathrc" <<'PATHRC'
export REGION=eu
#sh
alias where='echo top'
PATHRC
cat > "$uw/client/.pathrc" <<'PATHRC'
export REGION=us CLIENT=acme
#sh
client_fn() { echo client; }
PATHRC
echo 'export API=1 REGION=api' > "$uw/client/api/.pathrc"
echo 'export WEB=1' > "$uw/client/web/.pathrc"
if command -v age > /dev/null && [[ -f $XDG_CONFIG_HOME/pathrc/identity ]]; then
  printf 'export CLIENT_TOKEN=secret\n' |
    age -a -r "$(age-keygen -y "$XDG_CONFIG_HOME/pathrc/identity")" > "$uw/client/.pathrc.age"
  token=secret
else
  token= # no age CLI: the tree is plain only
fi
for d in "$uw" "$uw/client" "$uw/client/api" "$uw/client/web"; do (cd "$d" && pathrc allow > /dev/null); done

state='echo "${REGION-}|${CLIENT-}|${CLIENT_TOKEN-}|${API-}|${WEB-}|$(alias where 2>/dev/null | sed "s/.*=//")|$(type client_fn > /dev/null 2>&1 && echo fn)"'
for sh in bash zsh; do
  command -v "$sh" > /dev/null || continue
  out=$(cd "$uw" && "$sh" -c "eval \"\$(pathrc init $sh)\"; s() { $state; }
    s; cd client/api; s; cd ../web; s; cd ../..; s; cd other; s; cd ..; cd ..; s" 2>&1)
  expected="eu|||||'echo top'|
api|acme|$token|1||'echo top'|fn
us|acme|$token||1|'echo top'|fn
eu|||||'echo top'|
eu|||||'echo top'|
||||||"
  check "$sh: leaving scope unloads, sideways swaps, overrides restore" "$expected" "$out"
done

# cd within the same scope decrypts nothing; entering it decrypts once.
if [[ -n $token ]]; then
  out=$(cd "$uw/client" && PATHRC_DEBUG=1 bash -c 'eval "$(pathrc init bash)"; cd api; cd ..; cd api; cd web 2>/dev/null; cd ../web' 2>&1)
  check "bash: no decryption while staying in scope" "1" "$(grep -c 'decrypting' <<< "$out")"
fi

echo "$pass passed, $fail failed"
[[ $fail == 0 ]]
