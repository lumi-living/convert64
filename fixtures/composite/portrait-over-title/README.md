# portrait-over-title — masked portrait over a backdrop (multicolor, 4×)

## What it tests

- Lanczos-3 downscaling of 1024×637 sources (scale 0.3125 and 0.15) onto the 640×800 multicolor super grid, with the 2:1 aspect.
- A sub-pixel position (150.25, 52.5) for the top layer.
- A procedural radial gradient mask (softness 0.6) combined by `max` with an empty raster, then a 4-target-pixel feather.
- Brightness/contrast on the top layer only.
- The attribute map: Floyd–Steinberg at 100 % inside the cut-out, fading through the feather to the backdrop's Bayer 4×4 at 60 %.

## Rights

Built from the `portrait` and `title` fixtures (MIT, see their READMEs); nothing else.
