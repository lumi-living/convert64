#!/bin/zsh
# The E8 exit workflow, native (#210): two layered sources, a mask, a pin, Koala export.
# Usage: native-workflow.sh <out-dir>  (after `cargo build -p convert64-app`)
cd "$(git rev-parse --show-toplevel)"
OUT=$1
APP=./target/debug/convert64-app
MASK1='{"command":"mask_gradient","layer":1,"gradient":{"kind":"radial","from":{"x":500,"y":300},"to":{"x":500,"y":630},"softness":0.5},"combine":"min"}'
MASK2='{"command":"mask_set_modifier","layer":1,"modifier":{"feather":2.0}}'
PIN='{"command":"cell_pin","cells":[{"x":19,"y":10},{"x":20,"y":10},{"x":19,"y":11},{"x":20,"y":11}],"colours":[15],"full":false}'
run() { $APP fixtures/title/source.png fixtures/portrait/source.png "$@" & P=$!; ( sleep 90; kill $P 2>/dev/null ) & wait $P; }
for t in light dark; do
  run --screenshot $OUT/01-layers-$t.png --theme $t --tab layers
  run --screenshot $OUT/02-mask-$t.png --theme $t --command "$MASK1" --command "$MASK2" --mask
  run --screenshot $OUT/03-pin-$t.png --theme $t --command "$MASK1" --command "$MASK2" --command "$PIN" --view result --zoom 4 --centre 160,90 --overlay grid
  run --screenshot $OUT/04-export-$t.png --theme $t --command "$MASK1" --command "$MASK2" --command "$PIN" --view result --dialog export --export $OUT/export
done
