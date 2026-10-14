#!/usr/bin/env bash
set -euo pipefail
SOURCE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
BINARY="$HOME/.local/bin/flowsurface"
if [[ "$HOME" != /* || "$DATA" != /* || "$BINARY" == *$'\n'* || "$BINARY" == *$'\r'* || "$DATA" == *$'\n'* || "$DATA" == *$'\r'* ]]; then
    echo "HOME and XDG_DATA_HOME must be absolute paths without line breaks" >&2
    exit 1
fi
install -Dm755 "$SOURCE/bin/flowsurface" "$BINARY"
install -Dm644 "$SOURCE/share/icons/hicolor/512x512/apps/com.flowsurface.flowsurface.png" "$DATA/icons/hicolor/512x512/apps/com.flowsurface.flowsurface.png"
# Desktop entries decode string escapes before parsing the quoted Exec argument.
EXEC=${BINARY//\\/\\\\\\\\}
EXEC=${EXEC//\"/\\\\\"}
EXEC=${EXEC//\$/\\\\\$}
EXEC=${EXEC//\`/\\\\\`}
EXEC=${EXEC//%/%%}
mkdir -p "$DATA/applications"
while IFS= read -r line; do
    if [[ "$line" == Exec=* ]]; then printf 'Exec="%s"\n' "$EXEC"; else printf '%s\n' "$line"; fi
done < "$SOURCE/share/applications/com.flowsurface.flowsurface.desktop" > "$DATA/applications/com.flowsurface.flowsurface.desktop"
if command -v update-desktop-database >/dev/null; then update-desktop-database "$DATA/applications"; fi
if command -v gtk-update-icon-cache >/dev/null; then gtk-update-icon-cache -f -t "$DATA/icons/hicolor"; fi
printf 'Installed Flowsurface. Launch it from your application menu or %s\n' "$BINARY"
