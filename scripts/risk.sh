#!/usr/bin/env bash
# Prints the risk tier (docs/REVIEW.md "Risk tiers") that the paths changed since the merge base
# with main require: the highest of their tiers. With --label, fails when that label is lower than
# the required tier. There is no CI: `/pre-pr` runs it to pick the label and again before merging.
#
# Usage: risk.sh [--label low|medium|high] [base-ref]   (base-ref defaults to origin/main, else main)
set -uo pipefail

label=""
while (($#)); do
  case "$1" in
    --label)
      [[ "${2:-}" =~ ^(low|medium|high)$ ]] || {
        echo "--label takes low, medium or high"
        exit 2
      }
      label="$2"
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
if ! mb=$(git merge-base "$base" HEAD); then
  echo "No merge base with $base. Pass a base ref."
  exit 1
fi
# Renames count under both names.
changed=$({
  git diff --name-only --no-renames "$mb"
  git ls-files --others --exclude-standard
} | sort -u)

# Keep in step with docs/REVIEW.md.
high_paths='^(src-tauri/src/engine/profile/|src-tauri/tests/fixtures/profile\.yaml$|assets/cv\.typ$|assets/prompts/|scripts/|(\.config/)?\.?lefthook(-local)?\.(ya?ml|jsonc?|toml)$|\.githooks/|\.github/|\.claude/|\.agents/|AGENTS\.md$|CLAUDE\.md$|docs/REVIEW\.md$)'
low_paths='^(README\.md$|docs/)'

rank() { case "$1" in low) echo 1 ;; medium) echo 2 ;; high) echo 3 ;; *) echo 0 ;; esac }

if [[ -z $changed ]]; then
  echo "No changed paths since $base."
  exit 0
fi

high=$(grep -E "$high_paths" <<<"$changed" || true)
medium=$(grep -vE "$high_paths|$low_paths" <<<"$changed" || true)
if [[ -n $high ]]; then
  tier=high
elif [[ -n $medium ]]; then
  tier=medium
else
  tier=low
fi

echo "risk: $tier"
if [[ -n $high ]]; then
  echo "High paths:"
  sed 's/^/  /' <<<"$high"
fi

if [[ -n $label ]] && (($(rank "$label") < $(rank "$tier"))); then
  echo "FAILED: labelled risk: $label, but these paths need risk: $tier."
  exit 1
fi
