#!/usr/bin/env bash
# Build release packages into release/ with the Tauri bundler: a .deb and an AppImage.
# Usage: scripts/package.sh            (run from anywhere; BUNDLES=deb for the .deb only)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
dist="$root/release"
bundles="${BUNDLES:-deb,appimage}"
mkdir -p "$dist"

echo "==> web app dependencies"
bun install --frozen-lockfile

echo "==> release build and packages ($bundles)"
# The Tauri CLI finds `src-tauri/tauri.conf.json` from the repository root. It builds the web app
# and the extension (`beforeBuildCommand`; `extension/dist` is bundled as a resource), the release
# binary, then the bundles. The AppImage step downloads
# linuxdeploy on first use.
# A bundle that fails (the AppImage needs linuxdeploy to work on this machine) does not hide the
# ones that were built: they are still copied, and the script ends with the failure.
status=0
bun run tauri build --bundles "$bundles" || status=$?

echo "==> copy to release/"
found=0
for pkg in "$root"/target/release/bundle/deb/*.deb "$root"/target/release/bundle/appimage/*.AppImage; do
    [[ -e $pkg ]] || continue
    cp -f "$pkg" "$dist/"
    found=$((found + 1))
done
if ((found == 0)); then
    echo "no package was produced (see the output above)" >&2
    exit 1
fi
# The extension also as a zip, for "Load unpacked" after unzipping, or a store upload later.
(cd "$root/extension/dist" && rm -f "$dist/jobpilot-extension.zip" && zip -qr "$dist/jobpilot-extension.zip" .)
ls -1 "$dist"

if ((status != 0)); then
    echo "some packages failed (see the output above); the ones listed are in $dist/" >&2
    exit "$status"
fi
echo "Done. Packages are in $dist/"
echo "Not bundled: the typst CLI (cargo install --locked typst-cli) and Chrome (install the JobPilot extension in it: Settings > Browser)."
