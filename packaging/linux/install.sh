#!/bin/sh
# Adds Leaf to this account's app menu, with its icon, and offers it for
# books and images under "Open With". Comic archives (.cbz, .cbr, .cb7,
# .cbt) open in Leaf by double-click, unless another app already opens
# them. Nothing outside your home folder is touched.
#
#   ./install.sh            add Leaf (again, after moving this folder)
#   ./install.sh --remove   take it out of the menu
set -eu

here=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
apps="$data/applications"
icons="$data/icons/hicolor/256x256/apps"

comics="application/vnd.comicbook+zip application/vnd.comicbook-rar application/x-cb7 application/x-cbt"
others="application/zip application/epub+zip application/pdf image/jpeg image/png image/webp image/gif image/bmp inode/directory"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$apps" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
}

if [ "${1:-}" = "--remove" ]; then
  rm -f "$apps/leaf.desktop" "$icons/leaf.png"
  refresh
  echo "Leaf is out of the app menu. (This folder is untouched; delete it to remove Leaf.)"
  exit 0
fi

[ -x "$here/leaf" ] || { echo "leaf isn't next to install.sh" >&2; exit 1; }
mkdir -p "$apps" "$icons"
cp "$here/leaf.png" "$icons/leaf.png"
# The path goes in quotes, escaped as a desktop file wants: \ " ` $ take
# a backslash (written twice, since the file's own strings unescape one),
# and % is doubled.
exec_path=$(printf '%s' "$here/leaf" | sed -e 's/[\\"`$]/\\\\&/g' -e 's/%/%%/g')
LEAF_EXEC="Exec=\"$exec_path\" %f" LEAF_MIME="MimeType=$(printf '%s;' $comics $others)" \
  awk '/^Exec=/ { print ENVIRON["LEAF_EXEC"]; next } /^MimeType=/ { print ENVIRON["LEAF_MIME"]; next } { print }' \
  "$here/leaf.desktop" > "$apps/leaf.desktop"
chmod 644 "$apps/leaf.desktop"

if command -v xdg-mime >/dev/null; then
  for type in $comics; do
    current=$(xdg-mime query default "$type" 2>/dev/null || true)
    if [ -z "$current" ] || [ "$current" = leaf.desktop ]; then
      xdg-mime default leaf.desktop "$type"
    else
      echo "$type stays with $current (Leaf is in its Open With list)"
    fi
  done
fi
refresh
echo "Leaf is in the app menu. Run ./install.sh again if you move this folder."
