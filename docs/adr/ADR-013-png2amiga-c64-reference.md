# ADR-013: png2amiga's C64 path as the reference (supersedes ADR-010's choice of png2c64)

**Status:** Superseded by [ADR-014](ADR-014-png2c64-stays-reference.md) (2026-10-18)
**Date:** 2026-10-13
**Deciders:** Jaakko Haaparanta

## Context

ADR-010 made a faithful port of png2c64 the default bitmap solver and made pixel parity with png2c64 the quality bar. ADR-012 extended that bar to the compositor's resampling. Since then we have learned three things (`docs/research/2026-10-13-png2amiga.md`):

- **png2c64 has stopped moving.** Its last commit (2026-04-15) is the `1265329a` we pinned.
- **The same author continues the work in png2amiga** (MIT, v1.111.2 on 2026-10-05). Its C64 encoder began as a merge of png2c64 (`src/c64.hpp`). It covers multicolor, hi-res, FLI, AFLI, PETSCII and charsets, with its own VICE pixel checker.
- **png2amiga has fixed things png2c64 still does:**
  - It scales with Lanczos-3 widened by the shrink ratio, where png2c64 uses a 4-tap Mitchell that aliases on large sources (#235).
  - It scores results with SSIMULACRA2.
  - It has an opt-in cross-cell refinement step and more dither methods.

A frozen reference keeps parity tests meaningful but stops quality where png2c64 stopped. Following the author's active line of work keeps Convert64 on the improving baseline.

## Decision

1. **The reference becomes png2amiga's C64 path** (`--mode c64-multicolor` / `c64-hires`) at a **pinned commit**, recorded in `fixtures/README.md`; start at `4a87776` or the latest release when the work begins. Moving the pin is a deliberate PR that carries a regenerated sheet.
2. **ADR-010's method stays.**
   - Port faithfully, including odd choices.
   - Keep a parity test in CI (≥ 99 % identical per case; aim for 100 %).
   - Our extras are hooks or named options, off by default.
   - Nothing that changes what users see merges until a human has looked at the sheet.
   - Only the identity of the reference changes.
3. **What gets re-ported** from png2amiga where it differs from our png2c64 port:
   - the scaler (widened Lanczos-3; this replaces ADR-012's Mitchell and resolves #235);
   - the preprocessing defaults and the C64 cell metric default;
   - the multicolor background handling;
   - every dither method png2amiga offers for C64, so parity covers them.

   Port `--cell-refine` as an **opt-in Refine step**, as png2amiga has it.
4. **SSIMULACRA2 becomes the quality score**: on every sheet, in `--json`, and as the app's quality number (usability review M1). The clash count stays as a diagnostic. Port it, or add it as a TAD §4 dependency after asking.
5. **Fixtures.** The six fixtures get png2amiga baselines with their exact flags recorded. The png2c64 baselines move to `fixtures/archive/png2c64/` for history and are no longer gated.
6. **png2c64 parity is retired as a gate.** `SolverKind::Reference` now names the png2amiga port. The png2c64 port stays in the tree, behind `--solver png2c64`, until the new path passes its parity test. It is then removed in a separate PR.

## Options considered

| Option | Verdict |
|---|---|
| Keep png2c64 as the frozen bar and take png2amiga's ideas one by one | Rejected: every improvement becomes a divergence to justify against a reference that will never improve |
| **Move the reference to png2amiga's C64 path (this ADR)** | **Chosen** |
| No external reference; judge only by eye and SSIMULACRA2 | Rejected: #142 and #157 showed that without a pixel reference, regressions and deliberate changes look the same |

## Consequences

- **Easier:**
  - Large sources stop aliasing.
  - Sheets get a trustworthy score.
  - New C64 modes (FLI, AFLI, PETSCII, charsets) have a reference to port from when E10/E13 start.
  - Upstream improvements can be pulled in by moving the pin.
- **Harder:**
  - A re-baselining: goldens, parity data, the front-end sheet and the VICE fingerprints all change.
  - png2amiga needs a C++26 toolchain for the reference build (`cargo xtask png2amiga`, like the existing png2c64 one; expect the same `<print>` shims on older clang).
  - png2amiga's default flags differ from png2c64's (FS 1.0, error clamp 0.35, gamma 1.0 against checker 0.7, 0.1, 1.5). The baselines must record their flags, and Convert64's own defaults need a separate, explicit decision.
- **Revisit when:** png2amiga stops being maintained, or its C64 path diverges from what *No Glory* needs.

## Action items

- [ ] The reference move (issue linked from this ADR): `cargo xtask png2amiga`, the re-port, the parity test, re-baselined fixtures and the sheet. Leave the PR open for review.
- [ ] #235 is re-scoped to "port png2amiga's scaler" and folded into the move.
- [ ] SSIMULACRA2 on sheets, `--json` and the status bar.
- [ ] Mark ADR-010 and ADR-012 "Superseded in part by 013".
