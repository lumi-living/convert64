# Reference

The hardware truth and byte layouts Convert64 is built and tested against. These are plain facts about the Commodore 64, written down so nobody needs any particular tool, book or AI assistant to contribute.

| File | Contents |
|---|---|
| [vic-ii-constraints.md](vic-ii-constraints.md) | Palette (Pepto/Colodore values, luminance groups), every display mode's geometry and colour rules, memory alignment, VIC banks, char-ROM shadow, validator rules |
| [file-formats.md](file-formats.md) | Byte layouts for Koala, Art Studio, Doodle, FLI, raw bitmap sets, sprite blocks, SpritePad `.spd`, CharPad `.ctm` header and export bins, PRG, and the assembler include syntax we emit for |
| [cli.md](cli.md) | The `convert64` command line: commands, options, exit codes and the `--json` report |
| [project-file.md](project-file.md) | The `.convert64` project file: schema version, layers, linked or embedded sources, masks, regions, steering, solve options |
| [autosave.md](autosave.md) | The app's autosave and recovery files: where they live (native folder, OPFS, IndexedDB), session records, sources stored once by content, what is kept, restore and failure |

When the solver, validator or a format writer disagrees with these files, these files win until a VICE screenshot or real hardware proves them wrong — then fix the file in the same PR.
- [dither-methods.md](dither-methods.md) — every dithering method in scope (ordered, noise, diffusion, palette-aware), parameters, and how they map to the attribute map.
- [colour-pairs.md](colour-pairs.md) — the nine luminance levels, same-luma colour pairs, pair bias, PAL scanline blending.
- [dithering-practice.md](dithering-practice.md) — what graphicians and converters actually do, and the Convert64 decisions that follow (evidence in `docs/research/`).
