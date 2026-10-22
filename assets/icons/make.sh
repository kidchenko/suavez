#!/usr/bin/env bash
#
# Rasterises the menu's icons from Lucide (ISC, https://lucide.dev) into the
# PNGs next to this script, which src/art.rs embeds in the binary: black on
# clear, tinted at runtime for the state and the menu's theme.
#
# Needs npm (to fetch the pinned package) and rsvg-convert (brew install
# librsvg). The output is committed, so building Sua Vez needs neither.
#
#   assets/icons/make.sh
set -euo pipefail
cd "$(dirname "$0")"

LUCIDE=0.545.0
EDGE=32   # menus draw icons at 16 pt: 32 px is sharp on Retina and at 200%
PAD=2     # breathing room, so glyphs sit at the size of the OS's own icons

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
(cd "$work" && npm pack --silent "lucide-static@$LUCIDE" >/dev/null)
mkdir -p "$work/lucide"
tar xzf "$work/lucide-static-$LUCIDE.tgz" -C "$work/lucide"

raster() { # svg png
  local inner=$((EDGE - 2 * PAD))
  rsvg-convert -w "$inner" -h "$inner" --keep-aspect-ratio \
    --page-width "$EDGE" --page-height "$EDGE" --left "$PAD" --top "$PAD" \
    -o "$2" "$1"
}

# Sua Vez name = Lucide name.
GLYPHS=(
  # a session's state, beside its row
  circle-alert circle-check circle-dashed circle
  # actions
  check-check eraser file-cog power
)

rm -rf glyph
mkdir -p glyph
for name in "${GLYPHS[@]}"; do
  raster "$work/lucide/package/icons/$name.svg" "glyph/$name.png"
done

echo "$(ls glyph | wc -l | tr -d ' ') glyphs"
