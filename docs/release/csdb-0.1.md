# CSDb release entry — Convert64 0.1 (draft)

**Draft, not submitted.** Post only after Jaakko's go, once the GitHub release and the
browser build exist (#9). Fields follow CSDb's "Add release" form; everything in
`[brackets]` is a placeholder. The credit wording comes from
[credits-0.1.md](credits-0.1.md), which says where each claim is recorded.

## Fields

| Field | Value |
|---|---|
| Release name | Convert64 0.1 |
| Type | Other Platform C64 Tool *(CSDb's type for tools that run on another computer and make C64 data. "C64 Tool" is for programs that run on the C64; check the list on the form)* |
| Release date | [YYYY-MM-DD, the day the GitHub release is published] |
| Released by | [Jaakko's CSDb scener or group entry; Lumi Living Oy is the copyright holder, not a scene group] |
| Released at | — |
| Platforms (note) | macOS (Apple silicon and Intel), Windows, Linux, and any browser with WebAssembly |
| Website | https://github.com/lumi-living/convert64 |
| Screenshot | `docs/media/vice-koala.png` (VICE, the `title` test picture converted with the default recipe). Disclose in the comment that the test picture is AI-generated (see `fixtures/title/README.md`). |

## Credits

| Role | Who |
|---|---|
| Code | [Jaakko Haaparanta, as his CSDb entry] |
| Idea / concept | [Jaakko Haaparanta] |
| Thanks to | Tinic Uro (png2c64, png2amiga), micheldebree (retropixels), arch0N (ArcPaint64), Elder0010 (C64CC), ALBERT, Pepto (palette), the VICE team |

*Open:* whether to say that the code was written with an AI coding assistant; the
commit history says so, and scene readers may ask.

## Download links

| Label | URL |
|---|---|
| macOS (Apple silicon), unsigned | [https://github.com/lumi-living/convert64/releases/download/v0.1.0/…-aarch64-apple-darwin.…] |
| macOS (Intel), unsigned | [https://github.com/lumi-living/convert64/releases/download/v0.1.0/…-x86_64-apple-darwin.…] |
| Windows `.msi`, unsigned | [URL, if the packaging lands for 0.1] |
| Linux AppImage | [URL, if the packaging lands for 0.1] |
| Browser build | [https://lumi-living.github.io/convert64/, once Pages is enabled and deployed] |
| Source (MIT) | https://github.com/lumi-living/convert64 |

## Summary / comment

> Convert64 is a converter for legal C64 bitmaps — multicolor (Koala) and hi-res (Art
> Studio) — from layered sources. It is not a pixel editor: it is the step before one.
>
> Each layer keeps its own dither recipe, so the sky can get ordered dither while the
> figures get error diffusion, and one solve still gives one legal picture. Layers are
> kept in float and placed with sub-pixel precision, with masks, feather and per-layer
> adjustments (levels, curves, HSL, sharpen). Cells can be steered: pin colours, lock
> cells, allow or deny colours, Force or Adaptive fitting. The result re-solves as you
> drag, at the 2:1 pixel aspect, and the status bar counts the colour-limited cells.
>
> The default solver is a bit-for-bit port of Tinic Uro's png2c64; his png2amiga C64
> path is there as a comparison (`--solver png2amiga`). About 90 dither methods, each with a
> live thumbnail in the picker.
>
> Exports: Koala `.kla`, Art Studio `.art`, a self-displaying `.prg`, raw binaries with a
> JSON manifest, PNG previews, and source for KickAssembler, ACME, 64tass, ca65 and C.
> Every Koala and Art Studio export is checked by loading it in VICE.
>
> The app runs natively and in the browser: [downloads for macOS; Windows and Linux
> packages if they land for 0.1, otherwise build from source]. The `convert64`
> CLI and an MCP server run the same commands as the app, so a `.convert64` project can
> be rebuilt from a Makefile.
>
> 0.1 is a first release: bitmaps only; charsets and sprites come later. The desktop
> builds are unsigned: on macOS, right-click the app and choose Open the first time.
>
> Credits and thanks:
> - png2c64 (Tinic Uro): the reference solver is a port of its pipeline.
> - png2amiga (Tinic Uro): its C64 path is the comparison solver; the Lanczos resampler
>   comes from it.
> - retropixels (micheldebree): studied, and the Koala / Art Studio layouts were checked
>   against its documented writers.
> - ArcPaint64 (arch0N): the Adaptive / Force terms and several dither pattern shapes.
> - C64CC (Elder0010): the idea of a GUI and a CLI with the same features.
> - ALBERT: the idea of per-channel curves and a luminance-weighted colour distance.
> - Pepto for the palette, and VICE for checking every export.
>
> MIT licence. Bug reports and test pictures welcome on GitHub.
