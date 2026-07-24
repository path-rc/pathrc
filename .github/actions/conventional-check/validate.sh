#!/usr/bin/env bash
set -euo pipefail

check_message="${INPUT_CHECK_MESSAGE:-true}"
check_branch="${INPUT_CHECK_BRANCH:-true}"
commit_types="${INPUT_COMMIT_TYPES:-build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test}"
branch_types="${INPUT_BRANCH_TYPES:-feature|feat|fix|bugfix|hotfix|release|chore|docs|refactor|test|build|ci|perf|style|revert}"
allowed_branches="${INPUT_ALLOWED_BRANCHES:-^(main|master|develop|dev)$}"

commit_regex="^(${commit_types})(\([a-z0-9.,/_-]+\))?!?: .{1,100}$"
branch_regex="^(${branch_types})/[a-z0-9._-]+$"

failures=0
summary="${GITHUB_STEP_SUMMARY:-/dev/null}"

fail() {
  echo "::error::$1"
  echo "- ❌ $1" >>"$summary"
  failures=$((failures + 1))
}

pass() {
  echo "$1"
  echo "- ✅ $1" >>"$summary"
}

echo "## Conventional Commit & Branch Check" >>"$summary"

# --- Branch name ---------------------------------------------------------
if [ "$check_branch" = "true" ]; then
  branch_name="${GITHUB_HEAD_REF:-}"
  if [ -z "$branch_name" ]; then
    branch_name="${GITHUB_REF_NAME:-}"
  fi

  if [ -z "$branch_name" ]; then
    fail "Could not determine branch name to validate."
  elif [[ "$branch_name" =~ $allowed_branches ]]; then
    pass "Branch '$branch_name' is exempt from conventional branch naming."
  elif [[ "$branch_name" =~ $branch_regex ]]; then
    pass "Branch name '$branch_name' matches conventional format."
  else
    fail "Branch name '$branch_name' does not match '<type>/<description>' (allowed types: $branch_types)."
  fi
fi

# --- Commit messages ------------------------------------------------------
if [ "$check_message" = "true" ]; then
  commits=()

  if [ -n "${GITHUB_BASE_REF:-}" ]; then
    # PR context: check every commit introduced on top of the base branch.
    git fetch --no-tags --depth=50 origin "$GITHUB_BASE_REF" >/dev/null 2>&1 || true
    base_sha="$(git merge-base "origin/${GITHUB_BASE_REF}" HEAD 2>/dev/null || true)"
    if [ -n "$base_sha" ]; then
      while IFS= read -r sha; do
        commits+=("$sha")
      done < <(git rev-list "${base_sha}..HEAD")
    fi
  fi

  if [ ${#commits[@]} -eq 0 ]; then
    # Push (or fallback) context: just check the most recent commit.
    commits=("$(git rev-parse HEAD)")
  fi

  for sha in "${commits[@]}"; do
    subject="$(git log -1 --format=%s "$sha")"
    short_sha="${sha:0:7}"
    if [[ "$subject" =~ $commit_regex ]]; then
      pass "Commit $short_sha: '$subject'"
    else
      fail "Commit $short_sha: '$subject' does not match '<type>(<scope>)!: <description>' (allowed types: $commit_types)."
    fi
  done
fi

if [ "$failures" -gt 0 ]; then
  echo "passed=false" >>"${GITHUB_OUTPUT:-/dev/null}"
  echo "::error::$failures conventional check(s) failed."
  exit 1
fi

echo "passed=true" >>"${GITHUB_OUTPUT:-/dev/null}"
echo "All conventional commit and branch checks passed."
