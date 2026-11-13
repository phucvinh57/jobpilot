#!/usr/bin/env bash
# Runs the checks that cover files changed since the merge base with main
# (committed and uncommitted). Usage: check.sh [--all] [base-ref]
set -uo pipefail

all=false
if [[ "${1:-}" == "--all" ]]; then
  all=true
  shift
fi
base="${1:-main}"

root=$(git rev-parse --show-toplevel)
cd "$root"

changed=""
if ! $all; then
  git rev-parse -q --verify "$base" >/dev/null || base="origin/$base"
  if ! mb=$(git merge-base "$base" HEAD); then
    echo "No merge base with $base. Pass a base ref or use --all."
    exit 1
  fi
  changed=$({
    git diff --name-only "$mb"
    git ls-files --others --exclude-standard
  } | sort -u)
fi

touches() {
  $all || grep -qE "$1" <<<"$changed"
}

failed=()
ran=()
run() {
  local name="$1"
  shift
  ran+=("$name")
  echo "::: $name: $*"
  if ! "$@"; then
    failed+=("$name")
    return 1
  fi
}

render_profile() {
  local pdf rc
  pdf=$(mktemp --suffix=.pdf)
  typst compile --root . --input data=profile.yaml assets/cv.typ "$pdf"
  rc=$?
  rm -f "$pdf"
  return $rc
}

if touches '^(Cargo\.(toml|lock)$|crates/|assets/)'; then
  run build cargo build --workspace &&
    run test cargo test --workspace &&
    run clippy cargo clippy --workspace -- -D warnings
fi

if touches '^(profile\.yaml$|assets/)'; then
  run render render_profile
fi

echo
if ((${#ran[@]} == 0)); then
  echo "No checks cover the changed paths."
  exit 0
fi
if ((${#failed[@]} > 0)); then
  echo "FAILED: ${failed[*]} (ran: ${ran[*]})"
  exit 1
fi
echo "PASSED: ${ran[*]}"
