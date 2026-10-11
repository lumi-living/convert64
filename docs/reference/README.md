# Reference

The hardware truth and byte layouts Convert64 is built and tested against. These are plain facts about the Commodore 64, written down so nobody needs any particular tool, book or AI assistant to contribute.

| File | Contents |
|---|---|
| [vic-ii-constraints.md](vic-ii-constraints.md) | Palette (Pepto/Colodore values, luminance groups), every display mode's geometry and colour rules, memory alignment, VIC banks, char-ROM shadow, validator rules |
| [file-formats.md](file-formats.md) | Byte layouts for Koala, Art Studio, Doodle, FLI, raw bitmap sets, sprite blocks, SpritePad `.spd`, CharPad `.ctm` header and export bins, PRG, and the assembler include syntax we emit for |

When the solver, validator or a format writer disagrees with these files, these files win until a VICE screenshot or real hardware proves them wrong — then fix the file in the same PR.
- [dither-methods.md](dither-methods.md) — every dithering method in scope (ordered, noise, diffusion, palette-aware), parameters, and how they map to the attribute map.
