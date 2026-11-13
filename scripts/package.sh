#!/usr/bin/env bash
# Build release packages into dist/: a .deb (cargo-deb) and, if `appimagetool` is installed, an AppImage.
# Usage: scripts/package.sh            (run from anywhere)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
dist="$root/dist"
mkdir -p "$dist"

echo "==> release build"
cargo build --release -p jobpilot

echo "==> .deb"
if cargo deb --version >/dev/null 2>&1; then
    # --no-build: the release binary was just built above.
    cargo deb -p jobpilot --no-build --output "$dist/"
else
    echo "skipped: cargo-deb not installed (cargo install --locked cargo-deb)" >&2
fi

echo "==> AppImage"
if command -v appimagetool >/dev/null 2>&1; then
    app="$dist/JobPilot.AppDir"
    rm -rf "$app"
    mkdir -p "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons/hicolor/256x256/apps"
    install -m755 target/release/jobpilot "$app/usr/bin/jobpilot"
    install -m644 assets/jobpilot.desktop "$app/jobpilot.desktop"
    install -m644 assets/jobpilot.desktop "$app/usr/share/applications/jobpilot.desktop"
    install -m644 assets/icons/jobpilot-256.png "$app/jobpilot.png"
    install -m644 assets/icons/jobpilot-256.png "$app/usr/share/icons/hicolor/256x256/apps/jobpilot.png"
    printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/jobpilot" "$@"\n' >"$app/AppRun"
    chmod +x "$app/AppRun"
    # The AppImage bundles only the binary: GPU/Wayland/X11/dbus libraries come from the host.
    ARCH="$(uname -m)" appimagetool "$app" "$dist/JobPilot-$(uname -m).AppImage"
    ls -1 "$dist"/*.AppImage
else
    echo "skipped: appimagetool not found (https://github.com/AppImage/appimagetool/releases)" >&2
fi

echo "Done. Packages are in $dist/"
echo "Not bundled: the typst CLI (cargo install --locked typst-cli) and Chrome (Settings > Browser sessions can download one)."
