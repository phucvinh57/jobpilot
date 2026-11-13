#!/usr/bin/env bash
file=$(jq -r '.tool_input.file_path // empty')
[[ -f "$file" ]] || exit 0
root=$(git -C "$(dirname "$file")" rev-parse --show-toplevel 2>/dev/null) || exit 0
rel=${file#"$root"/}

case "$rel" in
  cvs/*.yaml | profile.yaml) data=$rel ;;
  template/*.typ) data=profile.yaml ;;
  *) exit 0 ;;
esac

cd "$root" || exit 0
command -v typst >/dev/null || exit 0
pdf=$(mktemp)
trap 'rm -f "$pdf"' EXIT
if ! out=$(typst compile --format pdf --root "$root" --input "data=$data" template/cv.typ "$pdf" 2>&1); then
  echo "$out" >&2
  exit 2
fi
