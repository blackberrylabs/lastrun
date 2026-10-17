#!/usr/bin/env bash
# Build the release binaries and the three Linux packages into dist/.
#
#   packaging/build.sh            # .deb + .rpm + AppImage
#   packaging/build.sh deb rpm    # a subset
#
# Needs: cargo, cargo-deb, cargo-generate-rpm (cargo install cargo-deb cargo-generate-rpm),
# and for the AppImage either `appimagetool` on PATH or network access to fetch it once
# into packaging/.tools/. Nothing here needs root, and nothing is installed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
ARCH="$(uname -m)"
DIST="$ROOT/dist"
TOOLS="$ROOT/packaging/.tools"
mkdir -p "$DIST" "$TOOLS"

targets=("$@")
[ ${#targets[@]} -eq 0 ] && targets=(deb rpm appimage)

echo "== lastrun $VERSION ($ARCH): building release binaries"
cargo build --release --workspace
cargo test --release --workspace --quiet

for t in "${targets[@]}"; do
  case "$t" in
    deb)
      echo "== .deb"
      cargo deb -p lastrun --no-build --quiet
      cp target/debian/lastrun_"$VERSION"-*.deb "$DIST/"
      ;;
    rpm)
      echo "== .rpm"
      cargo generate-rpm -p crates/app
      cp target/generate-rpm/lastrun-"$VERSION"-*.rpm "$DIST/"
      ;;
    appimage)
      echo "== AppImage"
      TOOL="$(command -v appimagetool || true)"
      if [ -z "$TOOL" ]; then
        TOOL="$TOOLS/appimagetool-$ARCH"
        if [ ! -x "$TOOL" ]; then
          curl -sSL -o "$TOOL" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
          chmod +x "$TOOL"
        fi
      fi
      APPDIR="$ROOT/target/appimage/Lastrun.AppDir"
      rm -rf "$APPDIR"
      mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
        "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/icons/hicolor/scalable/apps"
      cp target/release/lastrun-app target/release/lastrun "$APPDIR/usr/bin/"
      cp assets/lastrun.desktop "$APPDIR/usr/share/applications/"
      cp assets/lastrun.desktop "$APPDIR/lastrun.desktop"
      cp assets/icon-256.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/lastrun.png"
      cp assets/icon.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/lastrun.svg"
      cp assets/icon-256.png "$APPDIR/lastrun.png"
      cp assets/icon-256.png "$APPDIR/.DirIcon"
      ln -sf usr/bin/lastrun-app "$APPDIR/AppRun"
      # appimagetool is itself an AppImage; extract-and-run avoids needing FUSE.
      APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$TOOL" --no-appstream \
        "$APPDIR" "$DIST/Lastrun-$VERSION-$ARCH.AppImage" 2>&1 | grep -v '^$' | tail -3
      chmod +x "$DIST/Lastrun-$VERSION-$ARCH.AppImage"
      ;;
    *)
      echo "unknown target: $t (deb, rpm, appimage)" >&2
      exit 2
      ;;
  esac
done

echo
echo "== dist/"
rm -f "$DIST/SHA256SUMS"
ls -la "$DIST"
# Only this version's packages: older ones may still sit in dist/.
(cd "$DIST" && sha256sum ./*"$VERSION"*.deb ./*"$VERSION"*.rpm ./*"$VERSION"*.AppImage 2>/dev/null) | tee "$DIST/SHA256SUMS"
