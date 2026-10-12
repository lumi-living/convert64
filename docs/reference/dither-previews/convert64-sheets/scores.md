<!-- Written by `cargo xtask dither-sheet`; do not edit by hand. -->

## Convert64 next to png2c64

Each of png2c64's 30 methods on the test card at strength 1.0: Convert64's reference solver (the default since #181) on png2c64's preprocessed card, and png2c64's render. Identical pixels, then OKLab MSE against the prepared card, per pixel and after the golden harness's blur, and the gradient-domain error on lightness (× 1000). The parity test is the bar (≥ 99 % identical); the sheet is for the eye (TAD §6).

| png2c64 id | Convert64 id | identical | MSE, Convert64 | MSE, png2c64 | blurred, Convert64 | blurred, png2c64 | gradient, Convert64 | gradient, png2c64 |
|---|---|---|---|---|---|---|---|---|
| `none` | `none` | 100.000 % | 0.0094 | 0.0094 | 0.0086 | 0.0086 | 0.30 | 0.30 |
| `bayer4` | `bayer4` | 100.000 % | 0.0108 | 0.0108 | 0.0066 | 0.0066 | 0.61 | 0.61 |
| `bayer8` | `bayer8` | 100.000 % | 0.0108 | 0.0108 | 0.0066 | 0.0066 | 0.60 | 0.60 |
| `fs` | `floyd-steinberg` | 100.000 % | 0.0131 | 0.0131 | 0.0064 | 0.0064 | 1.29 | 1.29 |
| `atkinson` | `atkinson` | 100.000 % | 0.0121 | 0.0121 | 0.0062 | 0.0062 | 1.54 | 1.54 |
| `sierra` | `sierra-lite` | 100.000 % | 0.0131 | 0.0131 | 0.0064 | 0.0064 | 1.45 | 1.45 |
| `checker` | `checker` | 100.000 % | 0.0105 | 0.0105 | 0.0071 | 0.0071 | 0.16 | 0.16 |
| `bayer2x2` | `bayer2x2-mc` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 0.43 | 0.43 |
| `h2x4` | `h2x4` | 100.000 % | 0.0108 | 0.0108 | 0.0066 | 0.0066 | 0.60 | 0.60 |
| `clustered` | `clustered4` | 100.000 % | 0.0108 | 0.0108 | 0.0069 | 0.0069 | 1.51 | 1.51 |
| `fs-wide` | `fs-wide` | 100.000 % | 0.0131 | 0.0131 | 0.0064 | 0.0064 | 1.06 | 1.06 |
| `jarvis` | `jarvis-judice-ninke` | 100.000 % | 0.0130 | 0.0130 | 0.0065 | 0.0065 | 1.84 | 1.84 |
| `line2` | `line-h2` | 100.000 % | 0.0105 | 0.0105 | 0.0071 | 0.0071 | 0.16 | 0.16 |
| `line-checker` | `line-checker` | 100.000 % | 0.0107 | 0.0107 | 0.0068 | 0.0068 | 0.40 | 0.40 |
| `line4` | `line-h4` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 1.57 | 1.57 |
| `line8` | `line-h8` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 0.82 | 0.82 |
| `line-fs` | `line-fs` | 100.000 % | 0.0131 | 0.0131 | 0.0064 | 0.0064 | 0.95 | 0.95 |
| `halftone8` | `halftone8` | 100.000 % | 0.0108 | 0.0108 | 0.0070 | 0.0070 | 1.70 | 1.70 |
| `diagonal8` | `diagonal8` | 100.000 % | 0.0108 | 0.0108 | 0.0071 | 0.0071 | 1.66 | 1.66 |
| `spiral5` | `spiral5` | 100.000 % | 0.0109 | 0.0109 | 0.0069 | 0.0069 | 1.64 | 1.64 |
| `hex8` | `hex8` | 100.000 % | 0.0108 | 0.0108 | 0.0066 | 0.0066 | 0.64 | 0.64 |
| `hex5` | `hex5` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 0.61 | 0.61 |
| `blue-noise` | `blue-noise` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 0.75 | 0.75 |
| `ign` | `ign` | 100.000 % | 0.0108 | 0.0108 | 0.0067 | 0.0067 | 0.75 | 0.75 |
| `r2` | `r2` | 100.000 % | 0.0109 | 0.0109 | 0.0066 | 0.0066 | 0.67 | 0.67 |
| `white-noise` | `white-noise` | 100.000 % | 0.0108 | 0.0108 | 0.0068 | 0.0068 | 1.12 | 1.12 |
| `crosshatch` | `crosshatch` | 100.000 % | 0.0104 | 0.0104 | 0.0075 | 0.0075 | 1.01 | 1.01 |
| `radial` | `radial` | 100.000 % | 0.0109 | 0.0109 | 0.0074 | 0.0074 | 1.40 | 1.40 |
| `value-noise` | `value-noise` | 100.000 % | 0.0100 | 0.0100 | 0.0088 | 0.0088 | 0.44 | 0.44 |
| `ostromoukhov` | `ostromoukhov` | 100.000 % | 0.0130 | 0.0130 | 0.0063 | 0.0063 | 1.30 | 1.30 |

## Divergences

Each named option of the reference solver (`solve::Divergences`, all off by default) next to the reference, at strength 1.0: OKLab MSE and gradient error against the prepared input, reference then option.

| option | method | card MSE, ref | card MSE, on | card grad, ref | card grad, on | portrait MSE, ref | portrait MSE, on | portrait grad, ref | portrait grad, on |
|---|---|---|---|---|---|---|---|---|---|
| `chroma-guard` | `bayer4` | 0.0108 | 0.0110 | 0.61 | 0.68 | 0.0223 | 0.0225 | 3.24 | 3.33 |
| `max-luma-diff=0` | `floyd-steinberg` | 0.0131 | 0.0096 | 1.29 | 0.30 | 0.0285 | 0.0177 | 5.23 | 0.85 |
| `pair-bias=0.05` | `checker` | 0.0105 | 0.0105 | 0.16 | 0.16 | 0.0173 | 0.0173 | 0.61 | 0.61 |
| `cell-clamp` | `floyd-steinberg` | 0.0131 | 0.0130 | 1.29 | 1.25 | 0.0285 | 0.0285 | 5.23 | 5.16 |
| `mix-levels=4` | `bayer8` | 0.0108 | 0.0108 | 0.60 | 0.43 | 0.0220 | 0.0223 | 3.27 | 3.17 |
| `bridging` | `bayer4` | 0.0108 | 0.0108 | 0.61 | 0.61 | 0.0223 | 0.0223 | 3.24 | 3.24 |
