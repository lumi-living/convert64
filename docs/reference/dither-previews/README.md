# Dither previews (png2c64 reference renders)

Every png2c64 method rendered on two inputs, multicolor, Pepto palette, so the
look of each method in `dither-methods.md` can be checked without building
anything. These are *png2c64's* results, our reference quality bar — not
Convert64 output. Regenerate Convert64's own sheet with `cargo xtask
dither-sheet` once E5 lands.

| File | What |
|---|---|
| `testcard.png` | 320×200 synthetic input: grey ramp, colour ramp, radial blob, mixed ramp |
| `sheet-card.png` | all 30 methods on the test card (dither strength 1.0) |
| `sheet-zoom.png` | the grey ramp mid-tones of each method at 4× — pattern structure |
| `sheet-portrait.png` | all 30 methods on `fixtures/portrait/source.png` (png2c64 defaults, strength 0.7) |
| `png2c64-card/`, `png2c64-portrait/` | the individual 320×200 renders |

Generated with png2c64 1.0.0 (commit from 2026-10-11) built with clang 18:

```
png2c64 --mode multicolor --palette pepto --dither <method> --dither-strength 1.0 testcard.png card/<method>.png
png2c64 --mode multicolor --palette pepto --dither <method> portrait.png portrait/<method>.png
```

Observations worth carrying into E5:
- `radial`, `crosshatch` and `value-noise` are artistic effects, not quantisation aids; keep them, but group them as *Effects* in the recipe, not alongside Bayer.
- `ostromoukhov` is the cleanest diffusion on the ramps (no worms); `atkinson` the most contrasty; `fs-wide`/`jarvis` the softest.
- Line methods show why 2:1 forms matter: `line4`/`line8` cost nothing on multicolor; the vertical variants (ArcPaint64) are not in png2c64 at all.
- `blue-noise`, `ign` and `r2` are nearly indistinguishable at 320×200; one good blue-noise tile covers the family, the others are cheap to keep.
