#!/bin/sh
# Adds leaf9000 to this account's app menu, with its icon, and offers it
# for books and images under "Open With". Comic archives (.cbz, .cbr, .cb7,
# .cbt) open in leaf9000 by double-click, unless another app already opens
# them. Leaf's entry from before the rename is replaced. Nothing outside
# your home folder is touched.
#
#   ./install.sh            add leaf9000 (again, after moving this folder)
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
  rm -f "$apps/leaf9000.desktop" "$icons/leaf9000.png"
  refresh
  echo "leaf9000 is out of the app menu. (This folder is untouched; delete it to remove leaf9000.)"
  exit 0
fi

[ -x "$here/leaf9000" ] || { echo "leaf9000 isn't next to install.sh" >&2; exit 1; }
mkdir -p "$apps" "$icons"
cp "$here/leaf9000.png" "$icons/leaf9000.png"
# The path goes in quotes, escaped as a desktop file wants: \ " ` $ take
# a backslash (written twice, since the file's own strings unescape one),
# and % is doubled.
exec_path=$(printf '%s' "$here/leaf9000" | sed -e 's/[\\"`$]/\\\\&/g' -e 's/%/%%/g')
LEAF_EXEC="Exec=\"$exec_path\" %f" LEAF_MIME="MimeType=$(printf '%s;' $comics $others)" \
  awk '/^Exec=/ { print ENVIRON["LEAF_EXEC"]; next } /^MimeType=/ { print ENVIRON["LEAF_MIME"]; next } { print }' \
  "$here/leaf9000.desktop" > "$apps/leaf9000.desktop"
chmod 644 "$apps/leaf9000.desktop"

# Leaf, before it joined the 9000 series: its entry goes, and what it
# opened by double-click, leaf9000 opens now.
had_leaf=
if [ -f "$apps/leaf.desktop" ]; then
  had_leaf=1
  rm -f "$apps/leaf.desktop" "$icons/leaf.png"
fi

if command -v xdg-mime >/dev/null; then
  for type in $comics; do
    current=$(xdg-mime query default "$type" 2>/dev/null || true)
    if [ -z "$current" ] || [ "$current" = leaf9000.desktop ] || [ "$current" = leaf.desktop ]; then
      xdg-mime default leaf9000.desktop "$type"
    else
      echo "$type stays with $current (leaf9000 is in its Open With list)"
    fi
  done
fi
refresh
if [ -n "$had_leaf" ]; then echo "Leaf's menu entry was replaced by leaf9000's."; fi
echo "leaf9000 is in the app menu. Run ./install.sh again if you move this folder."
