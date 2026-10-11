# gradient — Gradient chart (solver regression)

Synthetic chart: 0–255 luminance ramp, full hue sweep, radial falloff, and the 16 Pepto palette colours as flat swatches.

## What it tests

Deterministic regression fixture. Any solver change shows immediately as banding, hue drift or a wrong swatch index. The swatch band must quantise to exactly indices 0–15 with no dithering at all — that is a unit test.

## Rights

Generated programmatically by this repository's maintainers (no external source). CC0 / MIT.

## Technical

- `source.png`: 1280×800, 8-bit sRGB, no alpha, metadata stripped on import.
- Supersample factor relative to the 320×200 C64 grid: **4×** (the compositor's default is 4×; anything ≥ 3× is fine for sub-pixel tests).
- Added 2026-10-11 for epic E3 (#3).
