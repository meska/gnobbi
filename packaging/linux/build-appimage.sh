#!/bin/sh
# Builds Gnobbi-x86_64.AppImage from the dist binary (cargo build --profile dist -p matrix-desktop --features live,linux-vault).
# Only the binary is bundled: its direct libraries are base-system ones; GPU/Wayland/Vulkan libs are dlopen'd from the host.
# Usage: packaging/linux/build-appimage.sh [output-dir]   (needs mksquashfs; appimagetool is fetched once into $APPIMAGETOOL_DIR)
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
src="$root/packaging/linux"
APP_ID=net.meskatech.gnobbi
NAME=Gnobbi-x86_64.AppImage
out=${1:-$root/artifacts/appimage}
bin="${CARGO_TARGET_DIR:-$root/target}/dist/gnobbi"
[ -x "$bin" ] || { echo "build first: cargo build --locked -p matrix-desktop --features live,linux-vault --profile dist" >&2; exit 1; }
command -v mksquashfs >/dev/null || { echo "mksquashfs (squashfs-tools) is required" >&2; exit 1; }
tooldir=${APPIMAGETOOL_DIR:-$HOME/.local/share/appimagetool}; tool="$tooldir/appimagetool"
if [ ! -x "$tool" ]; then
  mkdir -p "$tooldir"
  curl -fsSL -o "$tool" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod 755 "$tool"; sha256sum "$tool" > "$tooldir/appimagetool.sha256"
fi
mkdir -p "$out"; d="$out/AppDir"; rm -rf "$d"
mkdir -p "$d/usr/bin" "$d/usr/share/metainfo" "$d/usr/share/applications"
for s in 256 512; do mkdir -p "$d/usr/share/icons/hicolor/${s}x${s}/apps"; cp "$src/icon-$s.png" "$d/usr/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"; done
cp "$src/icon-512.png" "$d/$APP_ID.png"
cp "$bin" "$d/usr/bin/gnobbi"
cp "$src/$APP_ID.desktop" "$d/$APP_ID.desktop"; cp "$src/$APP_ID.desktop" "$d/usr/share/applications/"
cp "$src/$APP_ID.metainfo.xml" "$d/usr/share/metainfo/"
cat > "$d/AppRun" <<'L'
#!/bin/sh
# Native Wayland when the session has one; the profile lives in the XDG data directory.
HERE=$(dirname "$(readlink -f "$0")")
[ -z "${WAYLAND_DISPLAY:-}" ] || unset DISPLAY
exec "$HERE/usr/bin/gnobbi" --signin "$@"
L
chmod 755 "$d/AppRun"
rm -f "$out/$NAME"
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$d" "$out/$NAME" > "$out/package.log" 2>&1 || { tail -5 "$out/package.log"; exit 1; }
ldd "$bin" > "$out/ldd.txt"; grep -c 'not found' "$out/ldd.txt" || true
ls -l "$out/$NAME"
