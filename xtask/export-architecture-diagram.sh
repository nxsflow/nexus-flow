#!/usr/bin/env bash
# Re-export the published architecture diagram from its drawio source.
#
# WHY A SCRIPT AND NOT A LINE IN A README: the chain is not obvious and the wrong link in it costs
# a factor of four in bytes. draw.io's own SVG export is NOT used, and that is the whole point —
# every label in this diagram is an HTML-rich label (`html=1`, mixed sizes plus mono spans inside
# one paragraph), and draw.io rasterises those into embedded PNGs at 4x rather than emitting real
# `<text>`. The "SVG" was therefore 1214 KB of which 897 KB was bitmap, with none of the things an
# SVG is chosen for: no selectable text, no free scaling. Measured against it, one WebP at the SAME
# 4x text resolution is 314 KB — a quarter of the bytes for the same picture (nxf 6j6v.jepw).
#
# NOT A GATE. Nothing checks that the committed .webp matches the .drawio beside it; a real check
# would need draw.io in CI. The convention that stands in its place: BOTH FILES TRAVEL IN THE SAME
# COMMIT. Run this whenever you touch the .drawio, and commit what it writes.
set -euo pipefail

cd "$(dirname "$0")/.."

# Every published diagram, by basename. A `.drawio` here without an entry is a SOURCE that nothing
# ships, and there are two of those on purpose: `nexus-flow-high-level` and
# `the-journey-of-one-op-detailed`. Both are the long form of a picture the docs carry in a short
# one — kept because the next view grows out of them, not because anything renders them. Add a name
# here and to DOCS_ASSETS in content/lib/docs.mjs; the publish trigger and the content tests derive
# the rest.
diagrams=(nexus-flow-simplified the-journey-of-one-op a-reply-across-two-machines)

scale=4          # matches what draw.io rasterises labels at, so text is no softer than before
quality=82       # visually indistinguishable from lossless on this diagram; lossless is 355 KB

drawio="${DRAWIO:-/Applications/draw.io.app/Contents/MacOS/draw.io}"
[ -x "$drawio" ] || drawio="$(command -v drawio || true)"
if [ ! -x "${drawio:-}" ]; then
  echo "draw.io not found — install it (brew install --cask drawio) or set DRAWIO=<path>" >&2
  exit 1
fi
command -v cwebp >/dev/null || { echo "cwebp not found — brew install webp" >&2; exit 1; }

# The fonts must be ON THIS MACHINE: the labels are rasterised here, so a missing family is
# silently substituted into the shipped picture rather than failing.
#   brew install --cask font-merriweather font-merriweather-sans font-jetbrains-mono
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for name in "${diagrams[@]}"; do
  src="docs/architecture/$name.drawio"
  out="docs/architecture/$name.webp"
  [ -f "$src" ] || { echo "no such diagram source: $src" >&2; exit 1; }
  "$drawio" -x -f png -s "$scale" -b 16 -o "$tmp/$name.png" "$src" >/dev/null
  cwebp -quiet -q "$quality" "$tmp/$name.png" -o "$out"
  printf 'wrote %s (%s KB) from %s\n' "$out" "$(( $(wc -c < "$out") / 1024 ))" "$src"
done

echo 'commit them TOGETHER WITH their .drawio sources — nothing else keeps them in step.'
