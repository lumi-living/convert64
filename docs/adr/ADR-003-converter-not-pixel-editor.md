# ADR-003: Converter, not a pixel editor

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

The scene already has excellent C64 pixel editors (Pixcen, Multipaint, CharPad, SpritePad, ArcPaint64). The missing tool is the precision *conversion* stage before them. Scope creep into painting would multiply UI effort and dilute the differentiator.

## Decision

Convert64 contains **no freehand pixel painting**. In scope is everything that steers conversion: cell colour pins (full or partial), cell locks, painted per-region overrides of dither/metric/colour subsets, Force vs Adaptive colour fitting, layer masks and feathering, per-layer adjustments. The export is meant to be opened in a pixel editor for touch-ups.

## Consequences
- Easier: a sharp product; a solver-centric codebase; fewer widgets.
- Harder: users will ask for a pencil tool. Answer: export, open in ArcPaint64/Pixcen, and (later) a round-trip import of edited bitmaps as a locked layer.
- Clarified by [ADR-008](ADR-008-layer-masks-raster-plus-parametric.md): the mask brush is the one freehand tool and paints weight only, never colour.
- Revisit when: a round-trip "import C64 file as locked layer" feature exists and users still need single-pixel fixes inside Convert64.
