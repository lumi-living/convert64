<!-- Written by `cargo xtask dither-sheet`; do not edit by hand. -->

OKLab MSE of each render on the test card against the card with png2c64's preprocessing (gamma 1.5), per pixel and after the golden harness's blur.

| png2c64 id | Convert64 id | MSE, Convert64 | MSE, png2c64 | blurred, Convert64 | blurred, png2c64 |
|---|---|---|---|---|---|
| `none` | `none` | 0.0093 | 0.0098 | 0.0084 | 0.0086 |
| `bayer4` | `bayer4` | 0.0119 | 0.0111 | 0.0064 | 0.0067 |
| `bayer8` | `bayer8` | 0.0120 | 0.0111 | 0.0064 | 0.0067 |
| `fs` | `floyd-steinberg` | 0.0128 | 0.0132 | 0.0063 | 0.0064 |
| `atkinson` | `atkinson` | 0.0117 | 0.0123 | 0.0061 | 0.0062 |
| `sierra` | `sierra-lite` | 0.0129 | 0.0132 | 0.0064 | 0.0064 |
| `checker` | `checker` | 0.0115 | 0.0108 | 0.0082 | 0.0071 |
| `bayer2x2` | `bayer2x2-mc` | 0.0117 | 0.0111 | 0.0065 | 0.0067 |
| `h2x4` | `h2x4` | 0.0119 | 0.0111 | 0.0064 | 0.0066 |
| `clustered` | `clustered4` | 0.0119 | 0.0112 | 0.0067 | 0.0069 |
| `fs-wide` | `fs-wide` | 0.0127 | 0.0132 | 0.0063 | 0.0064 |
| `jarvis` | `jarvis-judice-ninke` | 0.0125 | 0.0131 | 0.0063 | 0.0065 |
| `line2` | `line-h2` | 0.0115 | 0.0108 | 0.0082 | 0.0071 |
| `line-checker` | `line-checker` | 0.0116 | 0.0109 | 0.0066 | 0.0068 |
| `line4` | `line-h4` | 0.0116 | 0.0112 | 0.0066 | 0.0067 |
| `line8` | `line-h8` | 0.0119 | 0.0111 | 0.0064 | 0.0067 |
| `line-fs` | `line-fs` | 0.0128 | 0.0132 | 0.0063 | 0.0064 |
| `halftone8` | `halftone8` | 0.0119 | 0.0111 | 0.0070 | 0.0070 |
| `diagonal8` | `diagonal8` | 0.0119 | 0.0111 | 0.0070 | 0.0071 |
| `spiral5` | `spiral5` | 0.0121 | 0.0111 | 0.0069 | 0.0069 |
| `hex8` | `hex8` | 0.0119 | 0.0111 | 0.0064 | 0.0067 |
| `hex5` | `hex5` | 0.0118 | 0.0111 | 0.0066 | 0.0067 |
| `blue-noise` | `blue-noise` | 0.0119 | 0.0111 | 0.0064 | 0.0067 |
| `ign` | `ign` | 0.0119 | 0.0111 | 0.0064 | 0.0067 |
| `r2` | `r2` | 0.0119 | 0.0111 | 0.0064 | 0.0066 |
| `white-noise` | `white-noise` | 0.0119 | 0.0111 | 0.0067 | 0.0068 |
| `crosshatch` | `crosshatch` | 0.0119 | 0.0107 | 0.0076 | 0.0075 |
| `radial` | `radial` | 0.0119 | 0.0111 | 0.0075 | 0.0074 |
| `value-noise` | `value-noise` | 0.0111 | 0.0103 | 0.0092 | 0.0088 |
| `ostromoukhov` | `ostromoukhov` | 0.0127 | 0.0131 | 0.0062 | 0.0063 |
