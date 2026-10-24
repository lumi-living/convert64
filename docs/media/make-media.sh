#!/bin/zsh
# Regenerates the README media in docs/media/ (#9) from the `title` fixture only.
# Usage, from the repository root, after `cargo build -p convert64-app -p convert64-cli`:
#   docs/media/make-media.sh [scratch-dir]
# Needs ImageMagick (`magick`), `sips` (macOS) and VICE's `x64sc` on PATH. The screenshot
# windows open on screen and see the real mouse pointer: keep it off them, or a hover
# tooltip lands in the picture. The crops assume a 2× (Retina) display, where
# `--screenshot` writes 3456 × 2010 (`--size` is ignored on macOS).
set -eu
cd "$(git rev-parse --show-toplevel)"
OUT=docs/media
TMP=${1:-$(mktemp -d)}
mkdir -p "$TMP"
APP=./target/debug/convert64-app
CLI=./target/debug/convert64
SRC=fixtures/title/source.png
FONT=crates/convert64-app/assets/fonts/Inter-Medium.ttf

# 1. Per-layer dithering: the picture twice, ordered dither on the sky, Floyd–Steinberg on
#    the foreground (the duplicate, masked to below the tree line by a linear gradient).
LAYERS=(
  --command '{"command":"layer_duplicate","layer":0}'
  --command '{"command":"layer_rename","layer":0,"name":"Sky"}'
  --command '{"command":"layer_rename","layer":1,"name":"Foreground"}'
  --command '{"command":"recipe_set","layer":0,"recipe":{"method":{"kind":"bayer4"},"strength":0.6}}'
  --command '{"command":"recipe_set","layer":1,"recipe":{"method":{"kind":"floyd-steinberg"},"strength":1.0}}'
  --command '{"command":"mask_gradient","layer":1,"gradient":{"kind":"linear","from":{"x":512,"y":230},"to":{"x":512,"y":300},"softness":0.5},"combine":"min"}'
)
for t in dark light; do
  $APP $SRC --screenshot "$TMP/per-layer-$t.png" --theme $t --tab layers --view result $LAYERS
  sips -Z 1280 "$TMP/per-layer-$t.png" --out "$OUT/per-layer-dithering-$t.png" >/dev/null
done

# 2. The method picker on the gradient preview, cropped to the picker.
for t in dark light; do
  $APP $SRC --screenshot "$TMP/picker-$t.png" --theme $t --method-picker gradient
  magick "$TMP/picker-$t.png" -crop 1354x1746+1464+156 +repage -resize 50% "$OUT/method-picker-$t.png"
done

# 3. Sub-pixel nudge: the layer moved right in quarter-pixel steps (canvas pixels; a
#    multicolor pixel is two wide), the Result view at its default 400 % (8 screenshot
#    pixels per canvas pixel at 2 pixels per point), cropped to 160 × 100 canvas pixels
#    around the ski troops and halved with point sampling: 4 × 4 GIF pixels per canvas
#    pixel, so every multicolor pixel is an exact 8 × 4 block.
i=0
for x in 0 0.25 0.5 0.75 1 0.75 0.5 0.25; do
  f=$(printf '%s/nudge-%02d.png' "$TMP" $i)
  $APP $SRC --screenshot "$f.full.png" --theme dark --view result \
    --command "{\"command\":\"layer_nudge\",\"layer\":0,\"by\":{\"x\":$x,\"y\":0}}"
  magick "$f.full.png" -crop 1280x800+768+996 +repage -filter point -resize 50% \
    -background '#1b1b1b' -gravity south -splice 0x36 \
    -font $FONT -pointsize 20 -fill '#f0f0f0' -annotate +0+7 "Layer X  +$(printf '%.2f' $x) px" "$f"
  i=$((i + 1))
done
magick -delay 70 -loop 0 "$TMP"/nudge-0?.png -layers Optimize "$OUT/sub-pixel-nudge.gif"

# 4. The exported Koala in VICE: the CLI's default recipe (Floyd–Steinberg 100 %, Pepto
#    PAL), a Koala and its self-displaying PRG, captured with vice-check's x64sc options.
$CLI convert $SRC -o "$TMP/title.kla" -o "$TMP/title.prg" -o "$TMP/title-preview.png"
$CLI validate "$TMP/title.kla"
x64sc -default -pal -VICIIextpal -VICIIpalette pepto-pal -VICIIfilter 0 \
  -VICIIsaturation 1000 -VICIIcontrast 1000 -VICIIbrightness 1000 -VICIIgamma 1000 \
  -VICIItint 1000 -sounddev dummy -warp -autostartprgmode 1 +autostart-delay-random \
  -limitcycles 30000000 -autostart "$TMP/title.prg" -exitscreenshot "$TMP/vice.png" >/dev/null || true
# VICE's 384 × 272 PAL frame, doubled with point sampling.
magick "$TMP/vice.png" -filter point -resize 200% "$OUT/vice-koala.png"
echo "media in $OUT, intermediates in $TMP"
