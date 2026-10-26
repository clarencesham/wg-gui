#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "error: run as root: sudo $0" >&2
    exit 1
fi

cd "$(dirname "$0")"

BIN=target/release/wg-gui
if [ ! -f "$BIN" ]; then
    echo "error: $BIN not found; build it first with: cargo build --release" >&2
    exit 1
fi

install -Dm755 "$BIN" /usr/bin/wg-gui
install -Dm644 deb/usr/share/applications/wg-gui.desktop /usr/share/applications/wg-gui.desktop
install -Dm644 deb/usr/share/polkit-1/actions/wg-gui.policy /usr/share/polkit-1/actions/wg-gui.policy

mkdir -p /usr/share/wg-gui/Icon
cp -r assets/Icon/. /usr/share/wg-gui/Icon/

update-desktop-database /usr/share/applications >/dev/null 2>&1 || true

echo "wg-gui installed."
