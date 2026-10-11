# Dithering practice on the C64 — what the research says and what Convert64 does with it

Distilled from three surveys (`docs/research/2026-10-11-*.md`): scene
practice, converter internals, and dithering theory. Each line here is
traceable to one of those; where the evidence is one person's opinion the
report says so and this page says "one artist". Items marked **→** are the
consequences for Convert64.

## 1. How graphicians actually dither

- **Checker ("chessboard") is the workhorse**, used both as an optical mix
  and, on PAL, as a real signal blend. Line alternation of equal-luma colours
  "looked awkward" to one artist, who dithered instead and accepted moiré on
  real hardware. → `checker` stays the default for multicolor (as png2c64).
- **Diagonal line dithering** was done by skewing in Photoshop, dithering in
  Retropaint, deskewing by hand, because no tool offered it. → `diag-down` /
  `diag-up` exist for exactly this; worth a skew-angle parameter later.
- **"Loose" / irregular dither** is a recognised style that one artist says
  halved his working time; a broken mouse leaving random gaps was kept on
  purpose. → blue-noise at reduced strength is the automatic equivalent;
  `white-noise` with a seed is the honest version of "random".
- **Dither with black as little as possible** (one artist); **no dither at
  all** (another). → `allowed_set` per layer and a per-region `strength` of
  0 are first-class, not edge cases.
- **Draw first, fix the attribute clash afterwards** is the stated workflow.
  → the clash overlay, pins and locks are the product; a converter that
  hides clashes is wrong.
- **Multicolor dominates** pictures; hi-res is rarely used for art by the
  artists quoted. → multicolor is the default mode and gets the tuning
  effort first.
- Not found anywhere: scene names for "stipple", "crosshatch", "grain";
  numeric viewing-distance rules; written tactics for colour sharing across
  cells. Those remain things the converter has to decide for itself.

## 2. Same-luma pairs and PAL blending (settled)

Widely stated across independent sources: the PAL delay line averages chroma
between adjacent scanlines but not luma, so alternating rows of equal-luma
colours become a real new colour on PAL hardware; NTSC flickers instead;
emulators without CRT emulation do not show it. The only pair artists
describe using is blue + brown → dark grey, with monitor-dependent tint. The
nine luma groups are in `colour-pairs.md`.

→ `pair` method and pair bias (#115); PAL shader must model the delay line
(E14); a **`max_luma_diff`** modifier (Colodore's "Max Luma Diff") restricts
any dither to pairs within N luma levels — the single knob that turns speckle
into blend. Added to `dither-methods.md` §5.

## 3. What converters do, and where they disagree

| Question | Consensus | Outlier | Convert64 |
|---|---|---|---|
| Per-cell colour set | top-3 by frequency (Retropixels, tiledither, Retropic) | png2c64: brute force all 455/120 in OKLab | brute force (TAD §3.1); frequency heuristics lose small bright details |
| Background | try all 16, keep lowest error (png2c64, tiledither) | Retropic: image mean | try all 16, or pinned |
| Error across cell borders | **allowed** (png2c64 intentionally, Dithertron, tiledither) | Retropic: clamped | clamped by default (TAD), **`cell_clamp: false`** as a recipe option — png2c64's results show it is viable |
| Metric | perceptual (OKLab, CIELAB, redmean, luma-weighted) | Retropixels: Euclidean XYZ | OKLab default; CIELAB, Pepto-YUV, RGB |
| Gamma | preprocess step (png2c64 1.5) | — | composite and dither in linear light, gamma 1.0 default |
| Dither default | ordered (Bayer 4×4 Retropixels, checker png2c64) | — | checker |
| Luma-pair awareness | **none** in any converter read | Yliluoma's luma-spread cap | pair bias + `max_luma_diff` — a real differentiator |
| Refinement | fixed sets then dither | Dithertron: iterate to convergence with hysteresis (accept a set change only if error drops past a threshold, sub-palette ratio 0.97) | 1–2 refinement passes **with hysteresis** so cell sets do not flip-flop |

Techniques worth copying by name:

- **png2c64:** `--adaptive` (scale incoming error by `1 − adaptive × local
  detail`, detail = mean |ΔL| to 4 neighbours / 0.15), `--error-clamp` on
  both incoming and outgoing error, percentile range-matching of source to
  palette extent, `blur`/`ssim` cell metrics for charsets.
- **Dithertron:** convergence loop with acceptance threshold and hysteresis.
- **png2prg:** bitpair-stability heuristics — keep a colour on the same
  bitpair as its neighbour cell so screen/colour RAM stay consistent; smaller
  packed files and fewer raster-visible seams. → E6 export option.
- **Yliluoma:** mix in linear light, choose in a perceptual space, cap the
  luma spread of a mix.
- **Disco Volante / Retro Pixel Converter:** expose the cell strategy as a
  user choice (Frequency / Luminance / Contrast / Error-Min; weighted pair
  fit, exhaustive, k-means, PCA gradient). → brute force is Error-Min; the
  others are not needed while brute force is fast enough.

Known failure modes (all seen in several tools): speckle where diffused error
reaches a cell lacking the colour; neighbour palettes flip-flopping when
chosen from a diffused image; frequency-only choice dropping a small bright
feature; heavy dither that looks fine at full resolution and turns to mush
once split into cells (→ `adaptive`, per-region strength).

## 4. Theory — what to trust

- **Dither in linear light; choose colours in OKLab; do not blend in OKLab.**
  Three independent sources. Our pair-bias "mean chroma" is computed from
  linear RGB, then scored in OKLab.
- **Blue noise over Bayer** for photos (Bayer has spectral spikes → visible
  crosshatch; brightens near-black). 64×64 void-and-cluster tile, toroidal.
  Bayer stays for the look people expect on C64.
- **Positional (ordered / Yliluoma) dithers are animation- and flicker-safe**
  because each pixel depends only on itself and its position; diffusion is
  not. → interlace/temporal modes (E15) use ordered or Yliluoma only, never
  diffusion across frames.
- **Ostromoukhov / Zhou–Fang** are tuned for 1-bit grey; the theory survey
  rates them marginal for 16 colours, but png2c64's `ostromoukhov` gave the
  cleanest ramps in our renders (`dither-previews/`). Keep; cheap.
- **Riemersma** is the one diffusion that is spatially local (≈16-pixel
  history), so it is the diffusion to try when flicker or frame-to-frame
  stability matters.
- Artists avoid dithering areas narrower than ~8–10 px and tiny or animated
  sprites; dither smooth areas, not detail. → `adaptive` default > 0 for
  photos; sprite solver (E11) defaults to strength 0.

## 5. Open questions the research could not settle

- Written scene tactics for sharing colours across cell borders and for
  choosing the background — none found; our solver's global background search
  and the clash report are the substitute.
- Pattern orientation rules for multicolor vs hi-res beyond "2:1 pixels make
  horizontal dither coarse" — the 2:1 matrix forms are our answer.
- Which patterns flicker on IFLI/MCI — only the luma-difference rule is
  supported; test on hardware in E15.
- CSDb forum threads and the Pixcen/Multipaint manuals were not reachable by
  search; reading them directly is the next step if more is needed.
