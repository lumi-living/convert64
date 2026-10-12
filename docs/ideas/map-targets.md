# Idea: game-map targets — from a painted 16-bit level map to charset + tile dictionary + map

**Status:** Idea, not scheduled. Extends E10 (charset & tile solver) and E16
(CharPad / round-trip). Written 2026-10-12 after working through the *No
Glory* case; to be developed into an ADR and issues when E10 starts.

## The problem

A tile-based game does not want a picture; it wants a **charset**, a **tile
dictionary** (n×n chars + colours per tile), a **map of tile indices**, and
per-tile **gameplay attributes** (collision, cover, LOS). The artist wants to
paint the level in 16-bit with layers and masks, or at least paint the tiles
that way. Convert64's compositor and solver already produce legal chars; what
is missing is the budgeted vocabulary above the char level and the game-side
exports.

## Worked case: *No Glory* (first user)

| Constraint | Value | Source |
|---|---|---|
| Screen mode | multicolor char mode, `$d021/$d022/$d023` global, per-char colour 0–7 | GDD_Technical_Core §1 |
| Map | 60 × 80 tiles, tile = 2 × 2 chars (16 × 16 px) → painted map is 960 × 1280 px (480 wide in MC pixels) | GDD_Technical_Core §1, §4 |
| Tile dictionary | 256 entries × (4 chars + 4 colours) | GDD_Technical_Core §4 |
| Charset | **full 256 chars for the map**: the HUD uses its own charset, switched via `$d018` at the HUD raster line (and optionally `$d022/$d023` too). Resident set at `$2000–$2FFF` holds both. | decision 2026-10-12 |
| Reserved in the map set | 2 × 2 animated enemy tiles, cursor/markers drawn in the map area — a parameter, not a constant | GDD_Technical_Systems §1 |
| Per-mission | charset delta loaded at `$6601+` on top of the resident base | GDD_Technical_Core §2 |
| Attributes | per tile: impassable, blocks-LOS, cover none/light/heavy/foxhole, packed 2 tiles per byte | GDD_Technical_Core §4 |
| Build | deterministic, re-run from a Makefile per mission | PDD persona 1 |

## What Convert64 would do

1. **Composite** the painted map like any image: layers, masks, nudge,
   attribute map, at 960 × 1280 with the 2:1 `SuperGrid`. Nothing new.
2. **Global colours**: search or pin `$d021/$d022/$d023` per mission (snow
   and forest want different triples); per 8 × 8 char brute-force the fourth
   colour from 0–7.
3. **Char vocabulary under budget**: 19 200 char cells → ≤ `256 − reserved`
   unique **bitmaps**. Char identity is the bitmap only; the colour lives in
   the colour dictionary, so a tree-top in dark green and the same shape in
   light grey is one char. Greedy merge by perceptual damage, then k-means
   (TAD §3.2), live counter.
4. **Tile vocabulary under budget**: 2 × 2 char groups → ≤ 256 tile types.
   **This is the binding constraint**: 4 800 map tiles cannot become 256
   arbitrary ones. The solver needs a *tile-pressure heatmap* and the usual
   steering one level up — regions pinned as *unique* (objective, landmarks)
   keep their tiles; fill areas (forest, swamp) are homogenised toward the
   vocabulary with a tolerance slider.
5. **Seam-safe dithering**: ordered dithers only, `phase = ScreenLocked` to
   the 16-px tile grid so a repeated tile tiles seamlessly; no diffusion,
   because a tile's pixels must not depend on neighbours (positional-dither
   rule from `reference/dithering-practice.md` §4). `cell_clamp` becomes
   char-clamp.
6. **Base + delta**: import the resident charset as **locked chars**; the
   mission solve fills only free slots and exports the delta. Same mechanism
   as "import C64 file as locked layer" (ADR-003).
7. **Attributes** are not derivable from pixels: one mask layer per attribute
   (impassable, blocks-LOS, cover light/heavy/foxhole) painted with the mask
   tools or imported from a Tiled layer; exported as packed nibbles.
8. **Raster-split screens**: the project describes the screen as N raster
   regions, each with its own charset and `$d021–$d023`; the HUD is a second
   target in the same project, solved and exported alongside. Generalises to
   split-screen demos.
9. **Exports**: `charset.bin`, `tile_dict`, `color_dict`, `map_tiles`
   (+ RLE), `collision.bin`, a KickAssembler/ca65 include using the game's
   constants, CharPad `.ctm` for hand touch-ups (E16), and a PNG of the
   reconstructed map to diff against the painting.

## Recommended workflow (what makes the budget reachable)

- Paint a **16-bit tileset** (pine dense/sparse, birch, swamp, snow, trench,
  cabin…) as a Convert64 project; convert once per terrain family.
- Lay out maps in **Tiled** with the 16-bit tiles; Tiled gives collision and
  zone layers for free and its JSON reads back as tile indices.
- Paint **hero areas** freely as an extra layer; Convert64 solves them into
  the remaining tile budget and shows the pressure.
- Snow variants of a tile cost **no chars** (bitmap shared, colours differ),
  only dictionary entries — the lever the winter missions need.

## Open questions

- Does the HUD split also switch `$d022/$d023`? Decides whether the HUD
  palette is free or constrained by the mission's pair. (Convert64 handles
  both; the GDD has to say.)
- How many chars do enemy animation frames reserve? (Types × frames × 4.)
- Tile budget when enemies occupy map tiles: do enemy frames live in the
  tile dictionary (costing entries) or are they drawn char-by-char by the
  engine? Decides whether "reserved" is chars only or chars + tiles.
- Tiled import scope: tile indices only, or also object layers (enemy
  spawns, objectives) as JSON pass-through for the game's own tooling?

## When this becomes work

- ADR: "Map targets: budgeted tile vocabulary over the charset solver".
- E10: tile-level budget with pressure heatmap and unique-region pins;
  base-charset locking and delta export; raster-region profiles.
- E16: Tiled JSON import (tiles + attribute layers); `.ctm` round trip.
- Fixture: a 960 × 1280 painted map plus a Tiled layout, with the *No Glory*
  profile as `profiles/noglory.toml`.
