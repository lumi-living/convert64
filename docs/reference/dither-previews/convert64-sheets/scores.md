<!-- Written by `cargo xtask dither-sheet`; do not edit by hand. -->

OKLab MSE of each render on the test card against the card with png2c64's preprocessing (gamma 1.5), per pixel and after the golden harness's blur, and the gradient-domain error on lightness (× 1000; bands and hard steps where the source is smooth, #156). None of these is a bar: the sheet is (TAD §6).

| png2c64 id | Convert64 id | MSE, Convert64 | MSE, png2c64 | blurred, Convert64 | blurred, png2c64 | gradient, Convert64 | gradient, png2c64 |
|---|---|---|---|---|---|---|---|
| `none` | `none` | 0.0093 | 0.0098 | 0.0084 | 0.0086 | 0.27 | 0.38 |
| `bayer4` | `bayer4` | 0.0158 | 0.0111 | 0.0077 | 0.0067 | 1.37 | 0.67 |
| `bayer8` | `bayer8` | 0.0158 | 0.0111 | 0.0078 | 0.0067 | 1.39 | 0.66 |
| `fs` | `floyd-steinberg` | 0.0128 | 0.0132 | 0.0063 | 0.0064 | 1.18 | 1.31 |
| `atkinson` | `atkinson` | 0.0117 | 0.0123 | 0.0061 | 0.0062 | 1.41 | 1.59 |
| `sierra` | `sierra-lite` | 0.0129 | 0.0132 | 0.0064 | 0.0064 | 1.30 | 1.47 |
| `checker` | `checker` | 0.0146 | 0.0108 | 0.0079 | 0.0071 | 0.31 | 0.22 |
| `bayer2x2` | `bayer2x2-mc` | 0.0161 | 0.0111 | 0.0081 | 0.0067 | 0.98 | 0.49 |
| `h2x4` | `h2x4` | 0.0159 | 0.0111 | 0.0077 | 0.0066 | 1.39 | 0.67 |
| `clustered` | `clustered4` | 0.0160 | 0.0112 | 0.0082 | 0.0069 | 3.30 | 1.60 |
| `fs-wide` | `fs-wide` | 0.0127 | 0.0132 | 0.0063 | 0.0064 | 0.93 | 1.09 |
| `jarvis` | `jarvis-judice-ninke` | 0.0125 | 0.0131 | 0.0063 | 0.0065 | 1.54 | 1.86 |
| `line2` | `line-h2` | 0.0144 | 0.0108 | 0.0081 | 0.0071 | 0.33 | 0.22 |
| `line-checker` | `line-checker` | 0.0159 | 0.0109 | 0.0081 | 0.0068 | 0.83 | 0.45 |
| `line4` | `line-h4` | 0.0160 | 0.0112 | 0.0080 | 0.0067 | 3.48 | 1.65 |
| `line8` | `line-h8` | 0.0159 | 0.0111 | 0.0079 | 0.0067 | 1.70 | 0.87 |
| `line-fs` | `line-fs` | 0.0128 | 0.0132 | 0.0063 | 0.0064 | 0.94 | 0.97 |
| `halftone8` | `halftone8` | 0.0159 | 0.0111 | 0.0087 | 0.0070 | 3.66 | 1.76 |
| `diagonal8` | `diagonal8` | 0.0158 | 0.0111 | 0.0087 | 0.0071 | 3.52 | 1.72 |
| `spiral5` | `spiral5` | 0.0157 | 0.0111 | 0.0084 | 0.0069 | 3.83 | 1.65 |
| `hex8` | `hex8` | 0.0160 | 0.0111 | 0.0078 | 0.0067 | 1.42 | 0.70 |
| `hex5` | `hex5` | 0.0161 | 0.0111 | 0.0078 | 0.0067 | 1.37 | 0.67 |
| `blue-noise` | `blue-noise` | 0.0159 | 0.0111 | 0.0079 | 0.0067 | 2.11 | 0.80 |
| `ign` | `ign` | 0.0159 | 0.0111 | 0.0078 | 0.0067 | 1.77 | 0.81 |
| `r2` | `r2` | 0.0158 | 0.0111 | 0.0078 | 0.0066 | 1.40 | 0.72 |
| `white-noise` | `white-noise` | 0.0159 | 0.0111 | 0.0081 | 0.0068 | 2.57 | 1.18 |
| `crosshatch` | `crosshatch` | 0.0178 | 0.0107 | 0.0100 | 0.0075 | 3.52 | 1.06 |
| `radial` | `radial` | 0.0160 | 0.0111 | 0.0090 | 0.0074 | 3.18 | 1.46 |
| `value-noise` | `value-noise` | 0.0128 | 0.0103 | 0.0100 | 0.0088 | 1.14 | 0.50 |
| `ostromoukhov` | `ostromoukhov` | 0.0127 | 0.0131 | 0.0062 | 0.0063 | 1.19 | 1.36 |
