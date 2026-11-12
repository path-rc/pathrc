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

# Resolved: on macOS the temp directory is under /var, a symlink to
# /private/var, and pathrc reports resolved paths.
scratch=$(cd "$(mktemp -d)" && pwd -P)
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

# A tree: top/.pathrc.toml, top/project/.pathrc.toml, top/project/sub/.pathrc.toml
tree=$scratch/top
mkdir -p "$tree/project/sub"
cat > "$tree/.pathrc.toml" <<'PATHRC'
[common]
env.LEVEL = "top"
[sh]
env.TOP_SH = "yes"
[fish]
env.TOP_FISH = "yes"
PATHRC
cat > "$tree/project/.pathrc.toml" <<'PATHRC'
[bash]
env.LEVEL = "project-bash"
[zsh]
env.LEVEL = "project-zsh"
[fish]
env.LEVEL = "project-fish"
PATHRC
printf '[common]\nenv.LEVEL = "sub"\nenv.SUB = "yes"' > "$tree/project/sub/.pathrc.toml" # no final newline
pathrc allow "$tree/.pathrc.toml" "$tree/project/.pathrc.toml" "$tree/project/sub/.pathrc.toml" > /dev/null

run_posix() { # shell init-arg script
  (cd "$tree" && "$1" -c "eval \"\$(pathrc init $2)\"; $3" 2>&1)
}

if command -v bash > /dev/null; then
  check "bash: nested cd reads the [bash] table" "project-bash yes" \
    "$(run_posix bash bash 'cd project; echo "$LEVEL $TOP_SH"')"
  check "bash: innermost wins, missing newline ok" "sub yes yes" \
    "$(run_posix bash bash 'cd project/sub; echo "$LEVEL $SUB $TOP_SH"')"
  check "sh: shared init reads [common] and [sh] only" "top yes" \
    "$(run_posix bash sh 'cd project; echo "$LEVEL $TOP_SH"')"
else
  echo "skip: bash not installed"
fi

if command -v zsh > /dev/null; then
  check "zsh: nested cd reads the [zsh] table" "project-zsh yes" \
    "$(run_posix zsh zsh 'cd project; echo "$LEVEL $TOP_SH"')"
else
  echo "skip: zsh not installed"
fi

if command -v fish > /dev/null; then
  check "fish: nested cd reads [fish], not [sh]" "project-fish yes -" \
    "$(cd "$tree" && fish --no-config -c 'pathrc init fish | source; cd project; echo "$LEVEL $TOP_FISH -$TOP_SH"' 2>&1)"
else
  echo "skip: fish not installed"
fi

# collate never breaks the hook: an unreadable file is reported, the rest load.
if [[ $(id -u) != 0 ]]; then
  chmod 000 "$tree/project/.pathrc.toml"
  check "bash: unreadable middle file is skipped" "sub yes" \
    "$(run_posix bash bash 'cd project/sub 2>/dev/null; echo "$LEVEL $TOP_SH"')"
  chmod 644 "$tree/project/.pathrc.toml"
fi

# Nothing is applied until allowed, and a change needs allowing again.
mkdir -p "$tree/cloned"
printf '[common]\nenv.CLONED = "ran"\n' > "$tree/cloned/.pathrc.toml"
check "bash: an untrusted file is not applied" "-" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"
(cd "$tree/cloned" && pathrc allow > /dev/null)
check "bash: it applies once allowed" "-ran" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"
printf '[common]\nenv.CLONED = "changed"\n' > "$tree/cloned/.pathrc.toml"
check "bash: a changed file is not applied" "-" \
  "$(run_posix bash bash 'cd cloned 2>/dev/null; echo "-$CLONED"')"

# An old-format file is not read.
mkdir -p "$tree/old"
echo 'export OLD=ran' > "$tree/old/.pathrc"
(cd "$tree/old" && pathrc allow .pathrc > /dev/null)
check "bash: an old-format .pathrc is not read" "-" \
  "$(run_posix bash bash 'cd old; echo "-$OLD"')"

# A .pathrc.toml.age from pathrc 0.3 to 0.5 isn't read either; list says so.
echo 'not read' > "$tree/old/.pathrc.toml.age"
check "list marks an old .pathrc.toml.age" "1" \
  "$(cd "$tree/old" && PATHRC_SHELL=bash PATHRC_START=$tree pathrc list | grep -c 'pathrc.toml.age  \[old format: pathrc migrate\]')"

# pathrc migrate turns an old .pathrc into a .pathrc.toml that applies.
mig=$scratch/migrate
mkdir -p "$mig"
printf 'export MIGRATED=yes\nalias mg=true\n#zsh\nexport MZ=z\n' > "$mig/.pathrc"
(cd "$mig" && pathrc allow .pathrc > /dev/null && pathrc migrate > /dev/null)
check "bash: a migrated .pathrc applies" "yes|" \
  "$(cd "$mig" && bash -c 'eval "$(pathrc init bash)"; echo "$MIGRATED|${MZ-}"' 2>&1)"

# Secrets added with `pathrc secret`, given an age identity (made with the
# age CLI if it is installed).
if command -v age-keygen > /dev/null && command -v age > /dev/null; then
  mkdir -p "$XDG_CONFIG_HOME/pathrc" "$tree/secret"
  age-keygen -o "$XDG_CONFIG_HOME/pathrc/identity" 2> /dev/null
  (cd "$tree/secret" && printf from-age | pathrc secret TOKEN > /dev/null &&
    printf zsh-age | pathrc secret TOKEN --table zsh > /dev/null)
  check "bash: a value from pathrc secret is decrypted" "from-age" \
    "$(run_posix bash bash 'cd secret; echo "$TOKEN"')"
  if command -v zsh > /dev/null; then
    check "zsh: [zsh.enc] overrides [common.enc]" "zsh-age" \
      "$(run_posix zsh zsh 'cd secret; echo "$TOKEN"')"
  fi
else
  echo "skip: age CLI not installed"
fi

# Leaving scope (bash and zsh): a three-level tree mixing plain and encrypted
# files, siblings, an inner override of an outer variable, aliases, PATH,
# named unsets of the user's own values, and a file that starts afresh.
uw=$scratch/unload
mkdir -p "$uw/client/api/deep" "$uw/client/web" "$uw/client/fresh" "$uw/other"
cat > "$uw/.pathrc.toml" <<'PATHRC'
[common]
env.REGION = "eu"
path.prepend = ["bin"]
[sh]
alias.where = "echo top"
PATHRC
cat > "$uw/client/.pathrc.toml" <<'PATHRC'
[common]
env.REGION = "us"
env.CLIENT = "acme"
PATHRC
cat > "$uw/client/api/.pathrc.toml" <<'PATHRC'
[common]
env.API = "1"
env.REGION = "api"
unset.env = ["MINE"]
unset.alias = ["where"]
PATHRC
printf '[common]\nenv.WEB = "1"\n' > "$uw/client/web/.pathrc.toml"
printf '[common]\nunset.all = true\nenv.FRESH = "1"\n' > "$uw/client/fresh/.pathrc.toml"
if [[ -f $XDG_CONFIG_HOME/pathrc/identity ]]; then
  (cd "$uw/client" && printf secret | pathrc secret CLIENT_TOKEN > /dev/null)
  token=secret
else
  token= # no age CLI: the tree is plain only
fi
for d in "$uw" "$uw/client" "$uw/client/api" "$uw/client/web" "$uw/client/fresh"; do
  (cd "$d" && pathrc allow > /dev/null)
done

# REGION|CLIENT|CLIENT_TOKEN|API|WEB|FRESH|MINE|alias where|bin first on PATH
state='echo "${REGION-}|${CLIENT-}|${CLIENT_TOKEN-}|${API-}|${WEB-}|${FRESH-}|${MINE-}|$(alias where 2>/dev/null | sed "s/.*=//")|$([[ $PATH == "$PWD_TOP/bin:"* ]] && echo bin)"'
for sh in bash zsh; do
  command -v "$sh" > /dev/null || continue
  out=$(cd "$uw" && PWD_TOP=$uw MINE=own "$sh" -c "eval \"\$(pathrc init $sh)\"; s() { $state; }
    s; cd client/api; s; cd ../web; s; cd ../fresh; s; cd ../..; s; cd other; s; cd ..; cd ..; s" 2>&1)
  expected="eu||||||own|'echo top'|bin
api|acme|$token|1|||||bin
us|acme|$token||1||own|'echo top'|bin
|||||1|own||
eu||||||own|'echo top'|bin
eu||||||own|'echo top'|bin
||||||own||"
  check "$sh: leaving scope restores, sideways swaps, unsets and unset.all" "$expected" "$out"
done
if command -v fish > /dev/null; then
  # Each substitution prints something: in fish, text joined to an empty
  # substitution is an empty list, and echo would print nothing.
  fstate='echo "$REGION|$CLIENT|$API|$WEB|$FRESH|$MINE|"(functions -q where; and echo where; or echo)"|"(string match -q -- "$PWD_TOP/bin" $PATH[1]; and echo bin; or echo)'
  out=$(cd "$uw" && PWD_TOP=$uw MINE=own fish --no-config -c "pathrc init fish | source; function s; $fstate; end
    s; cd client/api; s; cd ../web; s; cd ../fresh; s; cd ../..; s; cd ..; s" 2>&1)
  # No `where`: the top file defines it in [sh], which fish doesn't read.
  expected="eu|||||own||bin
api|acme|1|||||bin
us|acme||1||own||bin
||||1|own||
eu|||||own||bin
|||||own||"
  check "fish: leaving scope restores, sideways swaps, unsets and unset.all" "$expected" "$out"
fi

# Only a newly entered file is read: going deeper doesn't decrypt the outer
# levels again, and a cd within scope decrypts nothing. (client has one
# encrypted value, so entering it decrypts once.)
if [[ -n $token ]]; then
  out=$(cd "$uw" && PATHRC_DEBUG=1 bash -c 'eval "$(pathrc init bash)"; cd client; cd api; cd deep; cd ..; cd deep; cd ../../web' 2>&1)
  check "bash: client's secret is decrypted once, entering client" "1" "$(grep -c 'decrypting' <<< "$out")"
fi

# var: a shell variable that child programs don't see, even when it shadows
# one the user exported; leaving restores the user's, still exported.
vp=$scratch/varproj
mkdir -p "$vp/in"
printf '[common]\nvar.SHELL_ONLY = "yes"\nenv.EXPORTED = "yes"\n' > "$vp/in/.pathrc.toml"
(cd "$vp/in" && pathrc allow > /dev/null)
child='$(sh -c "echo \${SHELL_ONLY-none}")'
for sh in bash zsh; do
  command -v "$sh" > /dev/null || continue
  check "$sh: var is set, not exported, and the user's export comes back" \
    "yes|none|yes
mine|mine" \
    "$(cd "$vp" && SHELL_ONLY=mine "$sh" -c "eval \"\$(pathrc init $sh)\"; cd in
      echo \"\$SHELL_ONLY|$child|\$EXPORTED\"; cd ..; echo \"\$SHELL_ONLY|$child\"" 2>&1)"
done
if command -v fish > /dev/null; then
  check "fish: var is set but not exported" "yes|none" \
    "$(cd "$vp" && fish --no-config -c 'pathrc init fish | source; cd in; echo "$SHELL_ONLY|"(sh -c "echo \${SHELL_ONLY-none}")' 2>&1)"
fi

# An .enc value made with the age CLI, as the error message suggests.
if command -v age > /dev/null && [[ -f $XDG_CONFIG_HOME/pathrc/identity ]]; then
  ep=$scratch/encproj
  mkdir -p "$ep"
  sealed=$(printf %s 'enc-secret' | age -r "$(age-keygen -y "$XDG_CONFIG_HOME/pathrc/identity")" | base64 | tr -d '\n')
  printf '[common]\nenv.VISIBLE = "plain"\n[common.enc]\nenv.HIDDEN = "%s"\n' "$sealed" > "$ep/.pathrc.toml"
  (cd "$ep" && pathrc allow > /dev/null)
  check "bash: an .enc value from the age CLI is decrypted" "plain|enc-secret" \
    "$(cd "$ep" && bash -c 'eval "$(pathrc init bash)"; echo "$VISIBLE|$HIDDEN"' 2>&1)"
  check "list counts it without decrypting" "1" \
    "$(cd "$ep" && PATHRC_SHELL=bash PATHRC_START=$ep PATHRC_IDENTITY=/nonexistent pathrc list | grep -c '\[1 encrypted value\]')"
fi

# show prints the cascade as plain code that can be evaluated.
check "bash: eval \$(pathrc show) gives the same values" "sub yes" \
  "$(cd "$tree/project/sub" && PATHRC_SHELL=bash PATHRC_START=$tree bash -c 'eval "$(pathrc show)"; echo "$LEVEL $TOP_SH"' 2>&1)"

# The installer, against a release archive made from this build.
case $(uname -s)/$(uname -m) in
  Linux/x86_64) target=x86_64-unknown-linux-musl ;;
  Linux/aarch64) target=aarch64-unknown-linux-musl ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  *) target= ;;
esac
if [[ -n $target ]] && command -v curl > /dev/null; then
  release=$scratch/release
  archive=pathrc-$target.tar.gz
  mkdir -p "$release"
  tar -czf "$release/$archive" -C "$(dirname "$bin")" pathrc
  if command -v sha256sum > /dev/null; then
    (cd "$release" && sha256sum "$archive" > "$archive.sha256")
  else
    (cd "$release" && shasum -a 256 "$archive" > "$archive.sha256")
  fi
  check "install.sh installs the release for this platform" "$("$bin" --version)" \
    "$(PATHRC_DOWNLOAD_URL="file://$release" PATHRC_INSTALL_DIR="$scratch/installed" \
      sh "$repo/docs/install.sh" 2> /dev/null && "$scratch/installed/pathrc" --version)"
  echo "0000  $archive" > "$release/$archive.sha256"
  check "install.sh refuses a checksum mismatch" "refused" \
    "$(PATHRC_DOWNLOAD_URL="file://$release" PATHRC_INSTALL_DIR="$scratch/refused" \
      sh "$repo/docs/install.sh" > /dev/null 2>&1 || echo refused)"
else
  echo "skip: install.sh (no release target or curl)"
fi

echo "$pass passed, $fail failed"
[[ $fail == 0 ]]
