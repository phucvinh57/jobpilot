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
  changed=$({
    git diff --name-only "$(git merge-base "$base" HEAD)"
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

slug() {
  basename "$1" .yaml
}

build_cvs() {
  local rc=0 f
  for f in "$@"; do
    make -B cv NAME="$(slug "$f")" || rc=1
  done
  return $rc
}

check_pages() {
  local rc=0 f pdf pages
  for f in "$@"; do
    pdf="build/$(slug "$f").pdf"
    pages=$(pdfinfo "$pdf" 2>/dev/null | awk '/^Pages:/{print $2}')
    if [[ -z $pages ]]; then
      echo "$pdf: cannot read the page count"
      rc=1
    elif ((pages > 2)); then
      echo "$pdf: $pages pages (limit 2)"
      rc=1
    else
      echo "$pdf: $pages page(s)"
    fi
  done
  return $rc
}

check_headers() {
  local rc=0 f
  for f in "$@"; do
    if ! head -n 1 "$f" | grep -q '^# Tailored for:'; then
      echo "$f: first line must start with '# Tailored for:'"
      rc=1
    fi
  done
  return $rc
}

shopt -s nullglob
all_cvs=(cvs/*.yaml)
changed_cvs=()
for f in "${all_cvs[@]}"; do
  if $all || grep -qxF "$f" <<<"$changed"; then
    changed_cvs+=("$f")
  fi
done

if touches '^(profile\.yaml$|template/|Makefile$)'; then
  run build make -B all && run page-limit check_pages "${all_cvs[@]}"
elif ((${#changed_cvs[@]} > 0)); then
  run build build_cvs "${changed_cvs[@]}" && run page-limit check_pages "${changed_cvs[@]}"
fi

if ((${#changed_cvs[@]} > 0)); then
  run tailored-header check_headers "${changed_cvs[@]}"
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
