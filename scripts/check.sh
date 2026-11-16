#!/usr/bin/env bash
# Runs the checks that cover files changed since the merge base with main (committed and
# uncommitted). The checks come in independent groups, each run only when its paths changed:
#   web     the web app and the browser extension: typecheck, Biome (lint + format), vitest, vite build, extension build
#   rust    the workspace: clippy, cargo test
#   render  the CV template: a Typst render of the example CV
# A change to the web app alone never builds Rust, and a Rust change never needs bun: the desktop
# crate embeds `dist/` at compile time, so a placeholder is written when there is no web build.
#
# There is no CI: this script, run by the pre-push hook and `/pre-pr`, is the gate before a merge.
#
# Usage: check.sh [--all] [--only <group>]... [base-ref]   (base-ref defaults to origin/main, else main)
#   --all     every group, whatever changed
#   --only    run only these groups (still filtered by the changed paths unless --all)
set -uo pipefail

all=false
only=()
while (($#)); do
  case "$1" in
    --all) all=true ;;
    --only)
      [[ "${2:-}" =~ ^(web|rust|render)$ ]] || {
        echo "--only takes web, rust or render"
        exit 2
      }
      only+=("$2")
      shift
      ;;
    -*)
      echo "Unknown option $1"
      exit 2
      ;;
    *) break ;;
  esac
  shift
done
root=$(git rev-parse --show-toplevel)
cd "$root"

# origin/main first: a local `main` is often stale and would count other merged work.
if [[ -n ${1:-} ]]; then
  base="$1"
elif git rev-parse -q --verify origin/main >/dev/null; then
  base=origin/main
else
  base=main
fi

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

# What each group covers. A change to this script runs everything.
self='scripts/check\.sh$'
declare -A paths=(
  [web]="^(src/|extension/|assets/fonts/|(package|tsconfig)\.json$|bun\.lock$|index\.html$|vite\.config\.ts$|biome\.jsonc?$|$self)"
  [rust]="^(Cargo\.(toml|lock)$|src-tauri/|assets/|$self)"
  [render]="^(assets/|$self)"
)
groups=(web rust render)

wanted() {
  local group="$1"
  if ((${#only[@]} > 0)) && [[ " ${only[*]} " != *" $group "* ]]; then
    return 1
  fi
  $all || grep -qE "${paths[$group]}" <<<"$changed"
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

biome_check() {
  bun x biome check --error-on-warnings
}

# The desktop crate embeds `dist/` (tauri.conf.json `frontendDist`) and does not compile without
# it; Rust checks do not need the real web app, so a placeholder stands in when there is none.
ensure_dist() {
  [[ -f dist/index.html ]] && return 0
  echo "No web build in dist/: writing a placeholder (run \`bun run build\` for the real app)."
  mkdir -p dist &&
    printf '%s\n' '<!doctype html><title>JobPilot</title><p>Placeholder from scripts/check.sh: run <code>bun run build</code>.</p>' >dist/index.html
}

render_example_cv() {
  local pdf rc
  pdf=$(mktemp --suffix=.pdf)
  typst compile --root . --input data=assets/prompts/example-cv.yaml assets/cv.typ "$pdf"
  rc=$?
  rm -f "$pdf"
  return $rc
}

# Extra cargo args, e.g. CARGO_PROFILE_ARG="--profile ci" (see Cargo.toml).
read -ra prof <<<"${CARGO_PROFILE_ARG:-}"

if wanted web; then
  run web-install bun install --frozen-lockfile &&
    run web-typecheck bun run typecheck &&
    run web-lint biome_check &&
    run web-test bun run test &&
    run web-build bun x vite build &&
    run ext-build bun run ext:build
fi

if wanted rust; then
  run dist ensure_dist &&
    # --all-features: also the `dev-tools` feature of the desktop crate (UI checks).
    run clippy cargo clippy --workspace --all-targets --all-features ${prof[@]+"${prof[@]}"} -- -D warnings &&
    run test cargo test --workspace ${prof[@]+"${prof[@]}"}
fi

if wanted render; then
  run render render_example_cv
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
