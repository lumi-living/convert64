# C64 image converters: dithering and colour-fitting survey

Compiled 2026-10-11. Claims come from READMEs and source files fetched via a summarising fetch tool, so exact line-level claims are second-hand. "Not verified" means I could not find or read the source. GitHub HTML pages were blocked, so I read raw files and the jsDelivr file listings.

## Coverage

| Tool | Depth reached |
|---|---|
| png2c64 (tinic) | README and source (`quantize.cpp`, `dither.cpp`) |
| png2prg (staD020) | README and `convert.go` |
| Retropixels (micheldebree) and Retropixels+ | CLI README, core source (`Quantizer.ts`, `Converter.ts`, `OrderedDither.ts`) |
| Retropic (marlow75) | README, `C64Renderer.java` |
| Disco Volante | Store page and a news article. No source. |
| tiledither (Go), Dithertron, Convertron3000, HermIRES, ALBERT, DTKoala, Spectrumizer, Yliluoma | README or source, as noted below |
| Multipaint, Pixcen, Timanthes, Project One, c64cc, HermiCUBE, Photoshop/GIMP actions, "koalaconverter", Image2ZX, ZX-Paintbrush, Convimg, Retro Image Tool, Ditherista, academic papers | Not verified (see end) |

## Per-tool findings

### png2c64 (tinic), the most sophisticated open implementation found
Sources: https://raw.githubusercontent.com/tinic/png2c64/main/README.md, https://raw.githubusercontent.com/tinic/png2c64/main/src/quantize.cpp, https://raw.githubusercontent.com/tinic/png2c64/main/src/dither.cpp, https://raw.githubusercontent.com/tinic/png2c64/main/CLAUDE.md

- **Dither methods.** The README lists 30, via `--dither`. The default is `checker`.
  - Ordered: `none`, `bayer4`, `bayer8`, `checker`, `bayer2x2`, `h2x4`, `clustered`, `line2`, `line-checker`, `line4`, `line8`.
  - Halftone and analytical: `halftone8`, `diagonal8`, `spiral5`, `hex8`, `hex5`, `blue-noise`, `ign`, `r2`, `white-noise`, `crosshatch`, `radial`, `value-noise`.
  - Error diffusion: `fs`, `atkinson`, `sierra`, `fs-wide`, `jarvis`, `line-fs`, `ostromoukhov`.
  - Related options: `--dither-strength` (0.7), `--error-clamp` (0.1), `--adaptive`, `--no-serpentine`.
  - The page metadata says "17 dither modes", which conflicts with the README's 30.
- **Colour distance.** Squared OKLab distance everywhere, precomputed into per-cell tables. `--metric mse|blur|ssim`.
  - MSE is the default.
  - Blur uses a Pappas-Neuhoff style pre-blur of the source to avoid seams at cell edges.
  - SSIM scores `-ssim + 1.0*mse`.
  - The README says blur and SSIM suit PETSCII and charset modes.
- **Per-cell colour set.** Exhaustive brute force.
  - Hires tries all 120 pairs.
  - Multicolour tries 16 backgrounds x 455 triples.
  - FLI picks a colour-RAM colour, then brute-forces pairs per 4-pixel row.
  - AFLI picks the best pair per 8-pixel row.
  - PETSCII scans 256 glyphs x foreground colours.
  - Cells are independent, with no look-ahead, and run in parallel.
- **Error-diffusion clamping at cell boundaries.** Two stages, per the source summary.
  - A global Floyd-Steinberg pass (serpentine, error clamped to about +-0.12 per OKLab channel) runs over the full image. It is used only to build a per-cell colour histogram that refines the cell palette. In multicolour the FS result replaces the MSE-optimal palette without checking it is better. That is a source observation, not a documented design.
  - The real dithering pass (`apply_error_diffusion`) restricts each pixel to its own cell's colour set. The error buffer is not reset at cell edges, so error flows into neighbouring cells that may hold different colours. A code comment indicates this is intentional.
  - Error is clamped in two places: on the incoming error and on each neighbour update. `--adaptive` scales the incoming error by `1 - adaptive*detail`, where detail is the mean absolute L difference to the 4 neighbours divided by 0.15 and capped at 1.
  - Ostromoukhov is implemented as FS scaled per pixel by `0.6 + 0.8*t`.
  - Atkinson keeps 6/8 of the error.
- **Global background.** Multicolour tries all 16 and keeps the lowest total error. PETSCII does the same. Hires fixes it at 0. The README line "each cell is tested against all 16 backgrounds" is ambiguous, but the source shows the choice is global. FLI/AFLI are called with background 0. Sprite multicolour uses the most frequent colour.
- **Gamma.** `--gamma` (0.1-8.0, default 1.5) is a preprocess step, not a linear-light dither. Other preprocessing: brightness, contrast, saturation, OKLab hue adjust, sharpen/blur, and `match_palette_range()`, which remaps the image's OKLab extent onto the palette's extent using percentiles. Error diffusion itself runs in OKLab.
- **Palettes.** Pepto (default), VICE, Colodore.
- **PAL blend and luma-pair awareness.** None found.
- **Interlace and FLI.** FLI (`--mode fli`) and AFLI (`afli`). Interlace is not mentioned. The FLI bug columns are handled by forcing background-only.
- **User opinion.** None found. The project is new and the gallery is the author's own.

### png2prg (staD020)
Sources: https://raw.githubusercontent.com/staD020/png2prg/master/convert.go, https://github.com/staD020/png2prg (README text as fetched)

This is a packer and formatter, not a converter. It does no dithering, no quantisation and no nearest-colour mapping. The README says it expects input that already meets C64 limits and recommends Retropixels for full-colour images.

- It detects the palette from `palettes.yaml`.
- It guesses bitpair assignments so identical colours keep the same bitpair across neighbouring cells, using per-colour bitpair counters and the previous char or the cell above. Flags: `-nbc`, `-npcc`, `-bpc`, `-bf`.
- `-brute-force` tries bitpair-colour permutations and keeps the smallest TSCrunch output.
- Bitpair 00 is the background and is forced via `-bitpair-colors`.
- Clashes are reported in verbose mode (`-v`) and refused with errors.
- Interlace: `-i` for two Koala frames or one image. It writes drazlace or true-paint .mci with a D016 offset (`-d016`, default 1). FLI is not described.
- **Worth copying:** bitpair-stability heuristics, so that colour RAM and screen RAM stay consistent between neighbours. This helps packing and also reduces raster-visible differences.

### Retropixels / Retropixels+ (micheldebree)
Sources: https://www.npmjs.com/package/retropixels, https://raw.githubusercontent.com/micheldebree/retropixels/master/retropixels-core/src/conversion/{Quantizer,Converter,OrderedDither}.ts (summaries), https://github.com/micheldebree/retropixels (archived; points to https://meatloaf.cc/tools/retropixels+/), https://theoasisbbs.com/browser-c64-image-converter-retropixels-in-your-tab/

- **Dither methods.** `--ditherMode`: `none`, `bayer2x2`, `bayer4x4` (default), `bayer8x8`. `--ditherRadius` runs 0-64, default 32. The OrderedDither source has extra `test`..`test9` presets. There is no error diffusion in core. The Retropixels+ page lists "error-diffusion styles", Bayer and none, but does not name them. The Retropixels+ code was not read.
- **Colour distance.** Plain Euclidean distance in a chosen colour space (`--colorspace`): `xyz` (default), `yuv`, `rainbow`, `rgb`. Palettes: `PALette` (default), `colodore`, `pepto`, `deekay`. The README says `colodore` is default in one place and `PALette` in another.
- **Per-cell colour selection.** Nearest-colour quantise every pixel against the full 16-colour palette first. Then, for each colour map (screen RAM nibble, colour RAM, etc.), take the most frequent quantised colour in each cell among pixels not already claimed (`extractColorMap`). Leftover pixels go to whichever map's colour is nearest to the original pixel (`mapToExistingColorMap`). This is a greedy frequency pick. It is not an error-minimising search.
- **Error diffusion at cell boundaries.** Not applicable.
- **Background.** Not handled in the three files read. The README only says the output format has a background byte.
- **Gamma.** Not mentioned.
- **Luma pair or PAL blend.** None.
- **FLI/interlace.** FLI and AFLI via `-m fli` (`--hires` for AFLI). The first 3 columns are blank. Interlace is not mentioned. Retropixels+ lists NUFLI as incomplete.
- **User opinion.** The Aseprite forum poster found its hires output better than Aseprite's palette matching and said it avoided "red error boxes" (https://community.aseprite.org/t/c64-to-an-image/22423). png2prg's author recommends it for full-colour input.

### Disco Volante (Megastyle, 2026)
Sources: https://megastyle.itch.io/disco-volante, https://www.generationamiga.com/2026/07/12/a-new-windows-tool-makes-c64-image-conversion-surprisingly-easy/

- **Dither methods.** None, Ordered (Bayer 8x8), Floyd-Steinberg, Atkinson, plus a strength slider.
- **Colour distance.** RGB or Perceptual (CIE Lab). The formulas are not stated.
- **Per-cell strategy (multicolour only).** Four named strategies: Frequency, Luminance, Contrast, Error-Min. The page does not explain how they work. No source was found.
- **Background.** Auto-selected by default, or manual. Only colours present in the source can be picked.
- **Gamma, PAL, FLI, interlace.** Not mentioned.
- **User opinion.** Comments are brief and positive, with no technical content.

### Retropic (marlow75)
Sources: https://raw.githubusercontent.com/marlow75/retropic/main/README.md, https://raw.githubusercontent.com/marlow75/retropic/main/src/pl/dido/image/c64/C64Renderer.java, https://theoasisbbs.com/retropic-image-converter-for-retro-platforms/

- **Dither methods.** Bayer, Blue Noise, Floyd-Steinberg, Noise, Atkinson. The README says an error tolerance applies.
- **Colour distance.** Euclidean, Redmean, Mahalanobis (for arbitrary palettes) and nearest luminance.
- **Hires cell colours.** The brightest pixel in the cell becomes the foreground and the darkest becomes the background (optionally filtered by a `luma_pixels` setting). Each pixel then takes the nearer of the two. An ordered variant instead scores colours by frequency x closeness to the other colours present.
- **Multicolour.** Pairs of hires pixels are merged (average or luma-weighted). The background is the average colour of the whole image mapped to the palette. Each 4x8 cell gets the background plus its three most frequent colours.
- **Error diffusion at cell boundaries.** Clamped at the cell edge. Neighbour updates are bounded by cell dimensions (`x0 < 21`, `x0 < 9`), so error does not cross into the next cell.
- **Palette.** A picture-derived 8-colour palette (`getPictureColors(8)`) replaces the calculated C64 palette in `setupPalette()`. The README describes "first pass dithering" without explaining it.
- **Other.** PAL preview with bloom, moire and scanlines. Interlaced hires and multicolour are supported. FLI is not mentioned. The tool is promoted as sophisticated, but no benchmarks exist.

### tiledither (Michael Martin, Go)
Sources: https://pkg.go.dev/github.com/michaelcmartin/tiledither, https://raw.githubusercontent.com/michaelcmartin/tiledither/master/c64mcbmp.go

- **Cell colour selection.** The cell palette is `{bg, top-3 most frequent colours}`. The source comment itself suggests median cut as an improvement.
- **Background.** Tries all 16. The cost for a background is the number of pixels that fall outside each cell's top 3 non-background colours. The lowest total wins.
- **Dithering.** Floyd-Steinberg, run after the per-cell palettes are fixed. Error crosses cell boundaries freely, and each pixel is quantised only against its own cell's palette.
- **Pre-processing.** The 320 to 160 reduction averages raw RGBA pairs, so there is no gamma handling.

### Dithertron (Hugg) and the ZX-family converters
Sources: https://raw.githubusercontent.com/sehugg/dithertron/master/src/dither/{basecanvas,canvas,dithertron,color}.ts, https://timexsinclair.com/blog/a-constellation-of-retro-pixel-converters/

- **Algorithm.** Iterative. Each pass runs `commit()`, which re-guesses the colours of every block. Block colours are chosen by histogram vote over the previous pass's error-diffused image (`alt`), with the current assignment weighted 100. Then an error-diffusion pass runs where each pixel may use only its block's colours.
- **Cross-block diffusion.** Error is not reset at block borders. It crosses them freely.
- **Convergence controls.** Diffusion defaults to 0.8. `errorThreshold` rises after pass 10, and a change of index is accepted only if the residual magnitude exceeds the threshold. Iteration stops after 100 passes or when no pixel changes.
- **Colour distance.** `perceptual` (redmean-style weighting), `dist` (Euclidean RGB), `hue` (marked "TODO???" in the source) and `max`.
- **Palette reduction.** `reducePalette` uses error-feedback clustering with a `diversity` parameter.
- **Spectrum.** The ZX variant picks the dark half or the bright half of the palette by lower total score.
- **C64.** Dithertron has a VICII canvas with FLI-bug blanking.
- **Spectrumizer** (https://solhsa.com/spectrumizer/). The pipeline is: nearest colour, pick the bright or dark subset per cell, re-map, take the top 2 colours as ink and paper, final re-map. The author says results are often unpleasing and "most source images just don't map well", and recommends preprocessing the source.
- **Retro Pixel Converter** (via the Timex Sinclair article). 12 dithers including Yliluoma. Four colour-search strategies: weighted-average pair fit, exhaustive per-block, global k-means pre-quantise, and PCA gradient fit. It also does OkLab and Redmean, sRGB-linear dithering, Gaussian pre-blur and Gauss-Seidel refinement. Only the article was read, not the source.

### Convertron3000 (fieserWolF)
Source: https://github.com/fieserWolF/convertron3000

It offers five dither methods, all from PIL or hitherdither: Floyd-Steinberg, Bayer-ordered, line, dot, and yliluoma1 (described as painfully slow). There is a "brightness palette" mode with presets. Colour distance, per-cell selection, gamma and FLI are not documented.

### HermIRES (Hermit)
Source: https://se.aminet.net/gfx/edit/HermIRES.readme

Import maps each pixel to the closest C64 colour (metric unnamed, default Pepto palette). Clashes are detected, not fixed: the TestClash modes are OFF, TEST, DENY, REUSE and SET. Dithering in the importer is listed as "may add later." It supports AFLI with the FLI-bug caveat. HermiCUBE was not found.

### ALBERT (albertpixels.com)
Source: https://albertpixels.com

The import page lists nearest-colour matching, Floyd-Steinberg and ordered Bayer. It argues plain Euclidean distance is unsuitable and offers a luminance-aware weighted alternative, plus per-channel curves. Per-cell handling and background were not documented. The 3.1.0 release article does not mention dithering (https://theoasisbbs.com/albert-3-1-0-release-brings-updated-c64-pixel-art-editor/).

### Others, brief
- **DTKoala (Amiga).** It averages source pixel pairs, uses the colour approximation code by Pasi "Albert" Ojala, and does no dithering. The default background is black. The author suggests limiting the source to the 16 C64 colours, and scaling to 160x200 and back for sharp edges (https://se.aminet.net/gfx/conv/DTKoala.readme).
- **c64img.** `fix` mode for clashes is described as "pretty naive", and unfixable colours fall back to the background (https://pypi.org/project/c64img/).
- **Pixcen Deluxe.** Converts to and from an "unrestricted working format" and lists true-colour conversion and block-wise colour selection. Details are not given (https://theoasisbbs.com/pixcen-deluxe-graphics-editor-brings-new-c64-tools/).
- **Multipaint.** An author reports it flags rule-breaking cells for manual cleanup after import (https://marincomics.com/c64-graphics.html). Another notes it offers no colour-reduction or dithering controls on import (https://itch.io/post/15871947).
- **Project One.** One developer says there are no plans for image conversion (https://itch.io/post/9717391). I could not confirm which editor that thread covers.

### PAL blending and luma pairs (reference, not a converter)
Source: https://pagetable.com/c64ref/colors/

- On PAL, the TV delay line averages chroma between adjacent scanlines, so alternating-line mixes are real signal blends.
- Pairs with similar luma blend seamlessly. Large luma differences show the pattern.
- The page defines "Max Luma Diff" (lower gives smoother mixes) and warns about "GEOS effect" vertical stripes from column or checker patterns.
- 6569R1 has 5 luma levels, later chips have 9.
- Of the converters read, none uses a luma-pair filter in its colour search. Retropic's luma filter selects pixels, not pairs. Yliluoma's page offers a luminance-spread limit on candidate mixes (see below).

### Yliluoma's arbitrary-palette ordered dithering
Source: https://bisqwit.iki.fi/story/howto/dither/jy/

- Per-pixel mixing plans for arbitrary palettes, with a threshold matrix choosing between mixes.
- Mixing is done in linear light at gamma 2.2.
- Weighted RGB with a luma-difference term. CIEDE2000 "did not go as well as anticipated."
- Each pixel depends only on its input value and position, which suits animation.

## Synthesis

### Consensus (several tools agree)
1. **Pixel pairs are averaged for multicolour.** tiledither, Retropic and DTKoala all do it. png2c64 treats multicolour as a 2:1 pixel ratio. Only png2c64 and Retropixels offer an alternative colour space for the averaging, and no tool does the averaging in linear light.
2. **Global background searched or derived.** tiledither and png2c64 try all 16 and keep the lowest error. Retropic uses the image mean. Disco Volante auto-picks from colours in the source. Nobody picks it per cell.
3. **The top 3 colours per cell is the baseline heuristic.** tiledither, Retropic and Retropixels all use frequency. png2c64 is the exception, with brute force over all triples.
4. **Perceptual or weighted distance beats plain Euclidean RGB.** png2c64 (OKLab), Disco Volante (CIE Lab), ALBERT (luminance-weighted), Retropic (redmean), Dithertron (redmean-style) and Retropixels (xyz default) all move away from raw RGB. Yliluoma reports that CIEDE2000 was not clearly better, so this is not unanimous.
5. **Palette choice is exposed.** Pepto and Colodore appear in png2c64, Retropixels and HermIRES.
6. **Ordered dithering is the safe default, error diffusion the alternative.** Retropixels defaults to Bayer 4x4 and png2c64 to checker. Floyd-Steinberg and Atkinson appear in most of the others.
7. **Cell colour sets are fixed before or independently of dithering, then dithering is restricted to them.** This holds in tiledither, png2c64, Retropic and Dithertron. Dithertron differs in re-guessing colours iteratively from the diffused image.

### Techniques unique to one tool, worth copying
- **png2c64:** brute-force cell search with a perceptual metric, and a global dithered histogram used only to refine palette choice. `--error-clamp` plus `--adaptive`, which gives less diffusion in detailed areas. A source-versus-palette percentile range match before quantisation. `blur` and `ssim` cell metrics to score against the continuous source.
- **Dithertron:** an iterate-to-convergence loop with an acceptance threshold. Its comment says block palettes flip-flop when chosen from the diffused image. It uses hysteresis in sub-palette switching (ratio 0.97).
- **png2prg:** bitpair-stability heuristics and brute-force bitpair permutations for smaller packed output.
- **Retropic:** cell-edge error clamping, which prevents cross-cell colour bleed.
- **Yliluoma:** mixing in linear light, with a luma-spread cap on allowed mixes.
- **Disco Volante:** luminance-based mono dither, so changing colours does not change the pattern (as described). Colour-picking strategies exposed as user choices.
- **Spectrumizer:** forced background and restricting to a bright or dark subset (the analogue of luma pairs).

### Failure modes
- **Speckle.** Error diffusion where the neighbouring cell's colour set lacks the diffused colour. The cell then renders the nearest available colour, producing noise. Mitigations seen: png2c64's error clamp and adaptive scaling, Retropic's cell-edge clamp, Dithertron's 0.8 diffuse default.
- **Palette flip-flop between neighbours.** Dithertron's comment says picking from the diffused image makes palettes unstable. Its iteration and threshold exist to settle it.
- **Wrong or unstable background.** tiledither's cost counts only pixels that miss the top 3, which ignores error magnitude. Retropic's global mean as background can be a poor fit. Disco Volante restricts the choice to colours already in the source.
- **Frequency-only cell choice loses small bright features.** Retropixels takes the most frequent quantised colour, and tiledither takes the top 3. Both can drop a small but high-contrast colour. png2c64's brute-force MSE avoids this at a cost in time.
- **Worms and stripes.** I found no tool describing worm artefacts. Pagetable warns about vertical stripes from checker and column patterns (the "GEOS effect"). Not verified for the converters.
- **Edge loss at the cell level.** Retropic's hires picks the brightest and darkest pixel as the two colours, which can lose mid-tone edges. png2c64 offers a blur metric partly to avoid seams. DTKoala's author advises pair-aligned source pixels for sharp edges.
- **Averaging in non-linear space.** tiledither averages raw values. Most others do not say. Gamma is a plain preprocess step in png2c64.
- **Stale or unchecked refinement.** png2c64's FS-derived palette replaces the MSE-optimal one without comparing errors (source observation).

## Could not verify
- **Not found or no technical details:** Timanthes, c64cc, HermiCUBE, "koalaconverter" in Python, Image2ZX, ZX-Paintbrush, Convimg, Retro Image Tool, Ditherista's C64 handling, Photoshop/GIMP C64 palette actions (the GIMP cbmplugs README required JavaScript), and any academic paper on VIC-II attribute-constrained quantisation. Searches returned no relevant primary source.
- **Pixcen, Multipaint and ALBERT import internals:** not documented in what I could read.
- **Disco Volante internals:** closed source, with no algorithm description.
- **Retropixels+ internals:** only the product page was read (https://theoasisbbs.com/browser-c64-image-converter-retropixels-in-your-tab/). It is described as built on Retropixels, and it also lists NUFLI as incomplete.
- **Quality reports:** I found no systematic comparison of converter output. User opinion comes from a handful of forum posts and blog tool descriptions.
