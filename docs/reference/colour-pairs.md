# Same-luminance colour pairs

The VIC-II palette has nine distinct luminance levels (Pepto's measurement;
Colodore keeps the same grouping). Two colours on the same level differ only
in chroma, so a 50 % dither between them reads as a new flat colour instead of
a texture, and on PAL hardware alternating them on successive scanlines makes
the TV's decoder blend them into one. This is the data behind the metric's
*pair bias* and the `pair` dither method (`dither-methods.md` §4).

| Luma level (Pepto, 0–32) | Colours | Blend reads as |
|---|---|---|
| 0 | 0 Black | — |
| 8 | 6 Blue · 9 Brown | dark olive / earth |
| 10 | 2 Red · 11 Dark Grey | muted reddish grey |
| 12 | 4 Purple · 8 Orange | warm mauve |
| 15 | 12 Grey · 14 Light Blue | cool grey |
| 16 | 5 Green · 10 Light Red | olive / khaki |
| 20 | 3 Cyan · 15 Light Grey | pale blue-grey |
| 24 | 7 Yellow · 13 Light Green | bright lime |
| 32 | 1 White | — |

Adjacent levels (8→10, 10→12, 15→16) are close enough that dithers across
them are still smooth; the big gaps (0→8, 12→15, 24→32) are where banding
shows and where diffusion earns its keep.

## How the solver uses this

- **Pair bias (metric):** when a cell's candidate set contains a same-luma
  pair, the dithered mix between them is scored as a flat colour at their
  mean chroma rather than as two speckled colours. Configurable weight; off
  gives plain per-pixel distance.
- **`pair` dither:** chooses, per pixel, the two set colours whose mix is
  nearest in OKLab, preferring same-luma pairs; the weave is a 2×1 (hi-res)
  or 1×2 (multicolor, so it survives the 2:1 pixel) checker.
- **Flicker warning:** alternating *different*-luma colours per scanline
  flickers on NTSC and interlaced displays; the clash report flags cells whose
  dither crosses more than one luma level when the user has asked for
  "PAL-blend safe" output.

## PAL scanline blending (hardware, not perceptual)

On a PAL C64 and a real PAL display, lines N and N+1 with same-luma colours
are averaged by the decoder's delay line, giving roughly 28 additional stable
colours. Emulators differ: VICE with CRT emulation on reproduces it; the plain
palette renderer does not. Convert64's PAL preview shader (E14) must model the
delay line for this to be visible in the app; without it the technique looks
like a dither. NTSC flickers; not for animation.

Source: VIC-II measurements by Pepto (colodore.com, pepto.de/projects/colorvic),
scene practice as summarised in Multipaint and Pixcen documentation.
