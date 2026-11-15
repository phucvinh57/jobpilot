#!/usr/bin/env bash
# Runs the checks that cover files changed since the merge base with main (committed and
# uncommitted). The checks come in independent groups, each run only when its paths changed:
#   web     the web app: typecheck, Biome (lint + format), vitest, vite build
#   rust    the workspace: clippy, cargo test
#   render  the CV: a Typst render of profile.yaml
# A change to the web app alone never builds Rust, and a Rust change never needs bun: the desktop
# crate embeds `dist/` at compile time, so a placeholder is written when there is no web build.
#
# Usage: check.sh [--all] [--only <group>]... [--plan] [base-ref]
#   --all     every group, whatever changed
#   --only    run only these groups (still filtered by the changed paths unless --all)
#   --plan    print `<group>=true|false` for each group (for CI's GITHUB_OUTPUT) and exit
set -uo pipefail

all=false
plan=false
only=()
while (($#)); do
  case "$1" in
    --all) all=true ;;
    --plan) plan=true ;;
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

# What each group covers. A change to this script or to the CI workflow runs everything.
self='scripts/check\.sh$|\.github/workflows/ci\.yml$'
declare -A paths=(
  [web]="^(src/|assets/fonts/|(package|tsconfig)\.json$|bun\.lock$|index\.html$|vite\.config\.ts$|biome\.jsonc?$|$self)"
  [rust]="^(Cargo\.(toml|lock)$|src-tauri/|assets/|profile\.yaml$|$self)"
  [render]="^(profile\.yaml$|assets/|$self)"
)
groups=(web rust render)

wanted() {
  local group="$1"
  if ((${#only[@]} > 0)) && [[ " ${only[*]} " != *" $group "* ]]; then
    return 1
  fi
  $all || grep -qE "${paths[$group]}" <<<"$changed"
}

if $plan; then
  for group in "${groups[@]}"; do
    if wanted "$group"; then echo "$group=true"; else echo "$group=false"; fi
  done
  exit 0
fi

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

# GitHub Actions: `biome ci` annotates the diff; locally `biome check` reads the same config.
biome_check() {
  if [[ -n ${GITHUB_ACTIONS:-} ]]; then
    bun x biome ci --error-on-warnings
  else
    bun x biome check --error-on-warnings
  fi
}

# The desktop crate embeds `dist/` (tauri.conf.json `frontendDist`) and does not compile without
# it; Rust checks do not need the real web app, so a placeholder stands in when there is none.
ensure_dist() {
  [[ -f dist/index.html ]] && return 0
  echo "No web build in dist/: writing a placeholder (run \`bun run build\` for the real app)."
  mkdir -p dist &&
    printf '%s\n' '<!doctype html><title>JobPilot</title><p>Placeholder from scripts/check.sh: run <code>bun run build</code>.</p>' >dist/index.html
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

if wanted web; then
  run web-install bun install --frozen-lockfile &&
    run web-typecheck bun run typecheck &&
    run web-lint biome_check &&
    run web-test bun run test &&
    run web-build bun x vite build
fi

if wanted rust; then
  run dist ensure_dist &&
    # --all-features: also the `dev-tools` feature of the desktop crate (UI checks).
    run clippy cargo clippy --workspace --all-targets --all-features ${prof[@]+"${prof[@]}"} -- -D warnings &&
    run test cargo test --workspace ${prof[@]+"${prof[@]}"}
fi

if wanted render; then
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
