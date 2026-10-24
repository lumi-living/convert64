# Forum post — Convert64 0.1 (draft)

**Draft, not posted.** One generic post for the CSDb forum (Tools / C64 Graphics),
Lemon64 and Forum64 (in English; Forum64 readers may prefer a German summary on top).
Post only after Jaakko's go, once the GitHub release and the browser build exist (#9).
Everything in `[brackets]` is a placeholder. The credit wording comes from
[credits-0.1.md](credits-0.1.md). Plain text with bare URLs, since the three boards use
different markup; add images by hand where a board allows them
(`docs/media/per-layer-dithering-dark.png`, `docs/media/sub-pixel-nudge.gif`,
`docs/media/vice-koala.png`).

---

**Subject:** Convert64 0.1 — a layered image converter for legal C64 bitmaps

Hi all,

I've released the first version of Convert64, an open-source converter that turns
pictures into legal C64 multicolor (Koala) and hi-res (Art Studio) bitmaps. It is not a
pixel editor; it is meant for the step before Pixcen, Multipaint or ArcPaint64.

What is different from a one-shot converter:

- **Each layer has its own dither recipe.** Put the sky on one layer with ordered
  dither and the figures on another with Floyd–Steinberg; one solve still gives one
  legal picture, because the recipe travels with each pixel through compositing.
- **Sub-pixel placement.** Nudge a layer by a quarter of a pixel and watch the cells
  re-solve; sometimes that is all it takes to give an eye the colour it needs.
- **Steering instead of retouching.** Pin a cell's colours, lock cells, allow or deny
  colours, choose Force or Adaptive fitting per layer. Pins survive every re-solve.
- **Live result** at the real 2:1 aspect in the Pepto or Colodore palette, with a count
  of the colour-limited cells and an overlay that shows where they are.
- **About 90 dither methods** in a picker with live thumbnails.
- **png2c64 inside.** The default solver is a bit-for-bit Rust port of Tinic Uro's
  png2c64, and his png2amiga C64 path is there for comparison (`--solver png2amiga`).
- **Exports:** `.kla`, `.art`, a self-displaying `.prg`, raw binaries with a JSON
  manifest, PNG previews, and source for KickAssembler, ACME, 64tass, ca65 and C. Every
  Koala and Art Studio export is checked in VICE.
- **Same features on the command line.** `convert64 convert title.png -o title.kla`, or
  a saved `.convert64` project in a Makefile; an MCP server exposes the same commands.

Get it:

- Browser: [https://lumi-living.github.io/convert64/]
- Downloads: [https://github.com/lumi-living/convert64/releases/tag/v0.1.0] — [macOS
  (Apple silicon and Intel); Windows and Linux if the packages land for 0.1]
- Source (MIT): https://github.com/lumi-living/convert64

The desktop builds are not signed yet. On macOS, right-click the app and choose Open the
first time; Windows SmartScreen may ask too.

0.1 does bitmaps only. Charsets and sprites are next, then FLI. The browser build has no
screen-reader support (an egui limit).

Credits and thanks:

- png2c64 by Tinic Uro: the reference solver is a port of its pipeline, and its
  results are the bar the tests hold Convert64 to.
- png2amiga by Tinic Uro: its C64 path is the comparison solver; the Lanczos resampler
  comes from it.
- retropixels by micheldebree: studied while designing the solver; the Koala and Art
  Studio layouts were checked against its documented writers.
- ArcPaint64 by arch0N: the Adaptive / Force terms, and several dither pattern shapes.
- C64CC by Elder0010: the idea that the GUI and the CLI should do the same things.
- ALBERT: the idea of per-channel curves and a luminance-weighted colour distance.
- Pepto for the palette, and the VICE team for the emulator every export is checked in.

I'd like to hear where it fails: send pictures that convert badly, files that don't load,
or workflows it gets in the way of. Issues on GitHub, or reply here.

[Jaakko / handle]
