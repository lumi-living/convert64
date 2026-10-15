# ADR-010: png2c64's pipeline, ported, as the default bitmap solver

**Status:** Accepted — the choice of png2c64 as the reference is superseded by [ADR-013](ADR-013-png2amiga-c64-reference.md); the method (faithful port, parity test, divergence by decision) stands.
**Date:** 2026-10-12
**Deciders:** Jaakko Haaparanta

## Context

png2c64 is the solver-quality bar named in `docs/PDD.md`. Two rounds of work (#142, #157) reasoned about what png2c64 does and reimplemented it inside Convert64's own solver (E5). Each time the side-by-side sheet still showed png2c64 winning by eye: after #157, 20 of the 30 shared method ids were worse, every one of them ordered, and the per-pixel and blurred MSE bars had said we were winning. The defect was in the cell colour-set search, and reasoning about another program's behaviour had not found it. One example: png2c64's command line never runs its ordered "picker" (`apply_ordered_matrix`) for the ordered methods, so #157 ported a function png2c64's output does not use.

png2c64 is MIT-licensed and its source was in hand (#181).

## Decision

- **Port png2c64's bitmap pipeline faithfully** as `convert64_core::solve::png2c64_ref`: colour conversions, preprocessing, bicubic scaling, the cell search with its global Floyd–Steinberg pass, and every dither. Odd choices are ported too. Attribution: `THIRD_PARTY.md`.
- **Prove it with a parity test in CI**: on png2c64's 30 ids on the test card and the portrait, and on all six fixtures, at least 99 % of pixels must be identical to png2c64's own renders (`docs/reference/png2c64-reference.md`). The reference is png2c64 built with strict IEEE arithmetic, which a portable Rust port can match bit for bit; the test reaches 100 % on 65 of the 66 cases.
- **Make it the default solver for the bitmap modes** (`SolverKind::Reference`). Convert64's own solver stays as `SolverKind::Native` (project `solve.solver`, CLI `--solver native`), with its goldens. The reference solves the compositor's frame; layers, masks, feathering and regions stay upstream and reach it per pixel through the attribute map.
- **Diverge only by decision.** Our extras become named options on the reference path, off by default. A human judges each on the sheet before it can become a default. Our extras that png2c64 has no notion of are hooks on the reference path, inactive by default: pins, locks, allow/deny, the recipes' allowed colours, per-pixel recipes from the attribute map, and the interactive fast path. A frame that needs something the reference path does not have falls back to the native solver, and `Solution::fallback` says why: methods png2c64 lacks, non-OKLab metrics, Force fit mode, and the native solver's own knobs.

## Options considered

### Option A: keep tuning the native solver toward png2c64
| Dimension | Assessment |
|---|---|
| Complexity | Unbounded: each round adds heuristics on top of the last |
| Performance | Native full multicolor solve: 49–95 ms single-threaded |
| Maintenance | No way to tell a regression from a deliberate difference |
| Fit | Failed twice by eye (#142, #157) |

### Option B: port png2c64 and make it the default (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | One module, ported operation for operation; extras become explicit hooks |
| Performance | Full multicolor solve 24–27 ms single-threaded, 5–8 ms with `parallel` (portrait) |
| Maintenance | The parity test is a hard, objective bar; divergences are named and reviewed |
| Fit | Equal to png2c64 by construction on every shared id |

### Option C: call png2c64 as an external program
| Dimension | Assessment |
|---|---|
| Complexity | Low to start |
| Performance | Process spawn per solve; no interactive path |
| Maintenance | C++ toolchain in every build; no WASM; no per-pixel recipes, pins or locks |
| Fit | Breaks CLAUDE.md non-negotiables 2 and 4 |

## Trade-off analysis

Option B gives up the native solver's refinements as defaults (cell clamping, mix levels, the chroma guard, rendered re-picks). None of them won on the sheet; they can come back one at a time, as options a human has compared. It also ties exact parity to png2c64's float semantics. The reference build fixes those (strict IEEE, correctly rounded `cbrt` and `pow`), and diffusion and png2c64's global Floyd–Steinberg pass are chaotic enough that a looser float model would not reproduce png2c64's own output either (GCC's default build differs from the strict one on up to 15 % of the card's pixels).

## Consequences
- **Easier:** answering "is it as good as png2c64?", which the parity test now does; telling deliberate divergences from regressions; speed, since the reference is faster than the native solver.
- **Harder:** keeping the parity when png2c64 changes, since the port is pinned to commit `1265329a`. The native recipe knobs no longer act by default: they route a frame to the native solver.
- **Revisit when:** a divergence is judged better on the sheet (make it a default); png2c64 changes upstream (re-pin, regenerate the references); the native solver beats the reference by eye.

## Action items
- [x] Port and parity test (#182–#185)
- [x] Reference as the default; native selectable, its goldens kept
- [x] Steering, interactive mode and allow/deny as hooks on the reference path (#181 step 6)
- [ ] Divergences as named options with sheet rows (#181 step 7)
