#!/usr/bin/env sh
# Installs the Rlex binary, desktop entry and icons for the current user, so
# launchers find the app and Wayland compositors can show its icon (they look
# it up by app_id, "rlex", through the desktop entry).
#
#   cargo build --release && scripts/install-desktop.sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bin="$here/target/release/rlex"
[ -x "$bin" ] || { echo "build first: cargo build --release" >&2; exit 1; }

data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$bin" "$HOME/.local/bin/rlex"
install -Dm644 "$here/assets/linux/rlex.desktop" "$data/applications/rlex.desktop"
install -Dm644 "$here/assets/logo/rlex.svg" "$data/icons/hicolor/scalable/apps/rlex.svg"
for s in 16 32 48 64 128 256 512; do
  install -Dm644 "$here/assets/logo/rlex-$s.png" "$data/icons/hicolor/${s}x${s}/apps/rlex.png"
done

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
echo "installed: ~/.local/bin/rlex, rlex.desktop, hicolor icons"
