# ADR-014: png2c64 stays the reference; png2amiga becomes a comparison solver (reverses ADR-013)

**Status:** Accepted
**Date:** 2026-10-18
**Deciders:** Jaakko Haaparanta

## Context

ADR-013 moved the reference solver from our png2c64 port (ADR-010) to png2amiga's C64 path, on the expectation that the same author's newer code would give better pictures. #273 ported it faithfully: 224 of 224 parity cases are 100 % identical to png2amiga's own renders.

By eye, it does not give better pictures. Jaakko reviewed four sheets on #273 (2026-10-18):

- **Baselines:** png2c64 is better on 5 of 6 fixtures. png2amiga wins only on lineart, with more even lines that are grey and bleed green.
- **Match range:** no variant beats the png2c64 baseline, and png2amiga's hull never wins.
- **Refine:** worse on 4 of 6 fixtures, mixed on photo; it shows cell blocks.
- **Large sources:** #257's Mitchell mip beats png2amiga's scaler on the zone plate.

The cause sheet (`sheet-cause.png`, `cargo xtask cause-sheet`) swapped png2amiga's differences one at a time at equal flags (checker 0.7, error clamp 0.1, gamma 1.5, Pepto). Three account for the gap; swapping all three back gives the png2c64 baseline to within 0.1 % on every fixture:

1. **The cell-colour stage.** png2amiga takes each cell's colours from a global Floyd–Steinberg pass (the top 3 per cell). png2c64 brute-forces them, biased by the ordered threshold.
2. **The scaler.** png2amiga's Lanczos-3 is widened by the shrink. png2c64 uses a 4-tap Mitchell.
3. **The ordered amplitude.** png2c64 applies the strength twice (0.7 × 0.7); png2amiga applies it once.

The fixed black background and the colour-space maths each moved the result by about a point or less.

Jaakko judged tiles 1 (the png2c64 baseline) and 11 (png2amiga with all three swapped back) the best. For lineart he preferred tile 13: png2amiga's scaler, cell colours limited to the greys, and black point 0.5.

SSIMULACRA2 disagreed with the eye on every sheet. On the 28 eye-ranked pairs (`fixtures/eye-rankings.toml`, `cargo xtask metric-check`), the best candidate metric agrees with the eye on 57–60 %.

## Decision

1. **The png2c64 port stays the reference (ADR-010).** `SolverKind::Reference` is the png2c64 port again. Its parity test against png2c64's renders is the CI gate again, and the png2c64 fixture baselines are the gated baselines again.
2. **png2amiga's C64 path is kept as a comparison solver,** behind `--solver png2amiga` (project `solve.solver`, MCP, the app's solver picker). It brings png2amiga's own front end (its scaler and its preprocessing maths for every layer), so it gives png2amiga's picture end to end. Its parity test against png2amiga's own renders (committed; CI never builds png2amiga) stays in CI as a test of the port, not of quality.
3. **The compositor keeps ADR-012's Mitchell and #257's mip chain** for every layer by default. png2amiga's widened Lanczos-3 is a per-layer opt-in resampler (*Resample: Mitchell · Lanczos*), since it makes thin lines more even.
4. **Line art gets an opt-in recipe, chosen by eye** (cause sheet tile 13): Lanczos resampling, cell colours limited to the greys (0, 11, 12, 15, 1), and black point 0.5 (png2c64's own `--black-point`), in hi-res `none` at gamma 1.5, on the png2c64 path. It is a named divergence, off by default. *Amended by #276:* the recipe runs on the png2amiga solver (§2), which makes it tile 13 to the pixel; on the png2c64 path it was 87.6 %, and the whole gap was png2c64's global Floyd–Steinberg replacement of each hi-res cell's pair (`sheet-lineart-recipe.md`).
5. **png2amiga's other features stay with its solver, off by default:**
   - Refine is experimental and is not in the app's full solve.
   - The hull Match range and its Amount, experimental (our path's default stays png2c64's box; the hull is a front-end step, so it can be chosen on any solver).
   - The Background rule (*Search · Black*). The png2c64 path searches all 16 backgrounds as before.
6. **png2amiga's dither methods** are candidates for our pipeline. Each is adopted only after a sheet judged by eye (follow-up issue).
7. **No score decides quality.** SSIMULACRA2 stays in `--json` as information only. It is not a gate and not in the UI (#267 on hold). A metric may become a gate or appear in the UI only if it agrees with the eye on ≥ 90 % of `fixtures/eye-rankings.toml`.

## Options considered

| Option | Verdict |
|---|---|
| Keep png2amiga as the reference and tune it toward png2c64 | Rejected: the tuning is exactly undoing its three differences, so it would become png2c64 by another name |
| **png2c64 stays the reference; png2amiga is a comparison solver; adopt pieces by eye (this ADR)** | **Chosen** |
| Drop the png2amiga port | Rejected: it keeps a reproducible comparison against the author's newer line of work, and the source of candidate methods and of the line-art fix |

## Consequences

- **Easier:** defaults and goldens are png2c64's again, so what users see is unchanged from before #273. png2amiga's pieces come in one at a time, each with a sheet.
- **Harder:** two ported solvers to keep building; png2amiga's pin moves only by deliberate PR.
- **Measured on the line-art sheet** (`sheet-lineart.png`, #273): the recipe on our path is 87.6 % identical to tile 13. On a coloured logo the black point turns dark fills black and the colour limit greys the strokes, so the recipe is for black-and-white line art only.
- **Revisit when:** png2amiga's C64 output beats png2c64's by eye on the fixtures, or a metric passes the eye-ranked set.

## Action items

- [x] #273 reworked to this ADR: png2c64 as `Reference`, `--solver png2amiga`, Mitchell by default with Lanczos opt-in, the line-art recipe, png2c64 baselines gated again.
- [x] ADR-013 marked "Superseded by ADR-014"; ADR-010 and ADR-012 unchanged (never marked superseded on main).
- [x] Follow-up issue: png2amiga's dither methods on the png2c64 pipeline, judged on a sheet (#275).
