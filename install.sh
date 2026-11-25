#!/usr/bin/env bash
# Build and install Maprisma Clipboard for the current user (no root needed):
#   ~/.local/bin/maprisma-clipboard(-tray)             the applet / window and the KDE+Hyprland tray
#   ~/.local/share/applications/…desktop               the COSMIC applet and the window entry
#   ~/.config/autostart/….Tray.desktop                 the tray & clipboard watcher on KDE and Hyprland
#   ~/.local/share/icons/hicolor/scalable/apps/…svg    symbolic and full-color clipboard icons
# On COSMIC add it in Settings › Desktop › Panel › Applets.
set -euo pipefail
cd "$(dirname "$0")"
id=io.github.ubruckhaus.CosmicMaprismaClipboard
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin"

cargo build --release
install -Dm755 target/release/maprisma-clipboard "$bin/maprisma-clipboard"
install -Dm755 target/release/maprisma-clipboard-tray "$bin/maprisma-clipboard-tray"

install -Dm644 "data/icons/$id-symbolic.svg" "$data/icons/hicolor/scalable/apps/$id-symbolic.svg"
install -Dm644 "data/icons/clipboard-paused-symbolic.svg" "$data/icons/hicolor/scalable/apps/clipboard-paused-symbolic.svg"
install -Dm644 "data/icons/maprisma-clipboard.svg" "$data/icons/hicolor/scalable/apps/maprisma-clipboard.svg"

install -d "$data/applications"
desktop_arg() {
    local v=$1
    v=${v//\\/\\\\}
    v=${v//\"/\\\"}
    v=${v//\`/\\\`}
    v=${v//\$/\\\$}
    v="\"$v\""
    v=${v//\\/\\\\}
    v=${v//\%/%%}
    printf '%s' "$v"
}
app_exec=$(desktop_arg "$bin/maprisma-clipboard")
tray_exec=$(desktop_arg "$bin/maprisma-clipboard-tray")

# COSMIC panel applet desktop entry
while IFS= read -r line; do
    if [[ $line == Exec=* ]]; then
        printf 'Exec=%s --applet\n' "$app_exec"
    else
        printf '%s\n' "$line"
    fi
done < "data/$id.desktop" > "$data/applications/$id.desktop"

# Standalone window desktop entry
cat > "$data/applications/$id.Window.desktop" <<EOF
[Desktop Entry]
Type=Application
$(sed -n '/^Name/p; /^Comment/p' "data/$id.desktop")
Exec=$app_exec --window
Icon=maprisma-clipboard
Terminal=false
NoDisplay=false
Categories=Utility;
StartupWMClass=$id.Window
EOF

# Autostart tray + background clipboard watcher on KDE & Hyprland
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
install -d "$autostart"
cat > "$autostart/$id.Tray.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Maprisma Clipboard Tray
Name[de]=Maprisma Clipboard – Tray
Name[fr]=Maprisma Clipboard – Zone de notification
Name[es]=Maprisma Clipboard – Bandeja
Name[it]=Maprisma Clipboard – Area di notifica
Name[pt]=Maprisma Clipboard – Bandeja
Name[nl]=Maprisma Clipboard – Systeemvak
Name[pl]=Maprisma Clipboard – Zasobnik
Exec=$tray_exec
Icon=$id-symbolic
Terminal=false
OnlyShowIn=KDE;Hyprland;
EOF

gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
echo "Installed Maprisma Clipboard. On COSMIC add “Maprisma Clipboard” in Settings › Desktop › Panel › Applets."
