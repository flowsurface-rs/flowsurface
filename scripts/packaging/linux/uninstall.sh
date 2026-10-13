#!/usr/bin/env bash
set -euo pipefail
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
if [[ "$HOME" != /* || "$DATA" != /* ]]; then
    echo "HOME and XDG_DATA_HOME must be absolute paths" >&2
    exit 1
fi
rm -f -- "$HOME/.local/bin/flowsurface" \
    "$DATA/applications/com.flowsurface.flowsurface.desktop" \
    "$DATA/icons/hicolor/512x512/apps/com.flowsurface.flowsurface.png"
if [[ -d "$DATA/applications" ]] && command -v update-desktop-database >/dev/null; then update-desktop-database "$DATA/applications"; fi
if [[ -d "$DATA/icons/hicolor" ]] && command -v gtk-update-icon-cache >/dev/null; then gtk-update-icon-cache -f -t "$DATA/icons/hicolor"; fi
printf 'Uninstalled Flowsurface. Settings, indicators and credentials are preserved.\n'
