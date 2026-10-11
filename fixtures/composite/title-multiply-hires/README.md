# title-multiply-hires — Multiply blend with a snapped gradient mask and a region (hi-res, 4×)

## What it tests

- A full-screen portrait desaturated in OKLCh (`hsl.saturation = −0.4`).
- The title on top with `multiply` at 70 % opacity.
- The title is faded in from the bottom by a linear gradient mask (`multiply` on a full raster), snapped to whole 8×8 cells (`nearest`) before a 2-target-pixel feather.
- A region (canvas rectangle 200,20–300,80) that overrides the recipe with no dither at 50 %: it changes the attribute map and leaves the colour composite untouched.

## Rights

Built from the `portrait` and `title` fixtures (MIT, see their READMEs); nothing else.
