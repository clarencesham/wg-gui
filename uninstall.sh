#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "error: run as root: sudo $0" >&2
    exit 1
fi

rm -f /usr/bin/wg-gui
rm -f /usr/share/applications/wg-gui.desktop
rm -f /usr/share/polkit-1/actions/wg-gui.policy
rm -rf /usr/share/wg-gui

update-desktop-database /usr/share/applications >/dev/null 2>&1 || true

echo "wg-gui uninstalled."
