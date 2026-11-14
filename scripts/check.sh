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

# Extra cargo args, e.g. CARGO_PROFILE_ARG="--profile ci" (see Cargo.toml).
read -ra prof <<<"${CARGO_PROFILE_ARG:-}"

web=crates/desktop/web

# The web app first: the Tauri shell embeds its build (`web/dist`) at compile time, so clippy and
# the tests of the desktop crate need it. Fonts come from assets/fonts.
if touches '^(Cargo\.(toml|lock)$|crates/|assets/|scripts/check\.sh$|\.github/workflows/ci\.yml$)'; then
  run web-install pnpm --dir "$web" install --frozen-lockfile &&
    run web-typecheck pnpm --dir "$web" typecheck &&
    run web-lint pnpm --dir "$web" lint &&
    run web-test pnpm --dir "$web" test &&
    run web-build pnpm --dir "$web" exec vite build &&
    # --all-features: also the `dev-tools` feature of the desktop crate (UI checks).
    run clippy cargo clippy --workspace --all-targets --all-features ${prof[@]+"${prof[@]}"} -- -D warnings &&
    run test cargo test --workspace ${prof[@]+"${prof[@]}"}
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
