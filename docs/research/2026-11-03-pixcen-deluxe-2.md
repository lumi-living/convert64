# Pixcen Deluxe 2.0: review against Convert64 (2026-11-03)

**What:** Pixcen Deluxe 2.0.0, released 2026-11-02, from <https://pd.elysium.pl/v2/>.
- **macOS and Windows** builds of 74 MB and 63 MB; Linux is marked "soon".
- **Who:**
  - Deluxe fork by Krzysztof Dąbrowski (Elysium);
  - original Pixcen by John Hammarberg (Censor Design);
  - UX by Joachim Ljunggren;
  - tested by sceners including The Sarge, Jetboy and Wacek.
- **Licence:** "© 2026 Pixcen Team. All rights reserved." No source is published, and no price is stated.
- **Method:** I read the website, the authors page, the 1.x site and a press article, and looked at the published screenshots. I didn't run the app. The changelog and release notes didn't load (they come from an update feed).

## 1. What it is, and how it differs from Convert64

Pixcen is the C64 pixel editor that ADR-006 names as one of the tools our users already live in. Version 2 turns it into a layered editor and composer of C64 hardware output.

| | Pixcen Deluxe 2.0 | Convert64 |
|---|---|---|
| **Core job** | Draw, edit, animate and compose C64 graphics | Turn high-resolution artwork into a legal C64 picture |
| **What a layer is** | A C64-native surface: a bitmap (hires or multicolor), characters, sprites, rasters or guides. Layers are hardware parts that combine into the final screen: a bitmap plus border sprites plus raster colour changes. | A 16-bit source image with its own transform, adjustments, mask and **dither recipe**. All layers are solved into one legal bitmap. |
| **Conversion** | A one-shot import dialog ("Pixcen Color Reduction") with one global recipe. After import you edit pixels. | The product itself: per-layer and per-region recipes, pins and locks, a non-destructive re-solve after every change, and eye-judged quality against png2c64. |
| **Editing** | A full pixel toolkit: pencil, curves, splines, gradients, clone, symmetry, text, selections | None, by design (ADR-003: the brush paints mask weight, never colour) |
| **Modes** | Hires, multicolor, mixed, FLI, characters, sprites, an **All Borders** layout (bitmap plus border sprites plus rasters), and animation | Hires and multicolor bitmap now; charsets, sprites and FLI are planned (E10, E11, E13) |
| **Automation** | Embedded V8 JavaScript: tools, brushes, filters, exporters, plus a console | Command line and MCP over the same command registry; byte-identical output to the app |
| **Platforms** | Native desktop (macOS, Windows) | Native desktop and the same app in the browser |
| **Look** | A dark native UI. The widgets look like Qt (Fusion), but that isn't confirmed. | egui |

**Verdict.** The overlap is real but partial. Both are "layered C64 tools", but the layers mean different things:
- **Pixcen's layers** are C64 hardware layers: sprites over a bitmap, rasters.
- **Ours** are artwork layers that each choose how they turn into C64 pixels.

Pixcen 2 is now the strongest editor and composer. Its conversion is still a single dialog with one recipe for the whole picture, and that is the part Convert64 does far more deeply. The two are **complementary**:

> Convert64 makes the best legal starting picture, and Pixcen finishes it by hand.

The risk is that the import dialog gets good enough for most users. That is the reason to keep conversion quality as our first priority, and to make the hand-off to Pixcen smooth.

## 2. The import dialog, in detail (from the screenshot)

![Pixcen Color Reduction](https://pd.elysium.pl/v2/images/feature-import-settings-v2-793.webp)

| Group | Settings |
|---|---|
| **Target mode** | Import into *Bitmap Multicolor*; *Start with: Last used*; *Reset to Factory*; *Show Original* |
| **Preprocessing** | Gamma 1.50, Brightness, Contrast 1.10, Sharpen, Hue, Saturation |
| **Algorithm** | Colour space *LAB (CIELAB)*; Distance *RGB Weighted (ITU-R)*; RGB weight 0.85 |
| **Image and canvas** | Width 160 × Height 200, Lock aspect, *Use Image Size*, *Crop Image*, Resize *Stretch* |
| **Ordered dithering** | A gallery of 22 pattern thumbnails (None, 2×2, 4×4, horizontal, vertical, diagonal, 8×8, checker, cluster, halftone, scanline, grid, cross and others); **Amount per channel R, G and B**; **Skew** |
| **Error diffusion** | Algorithm *Jarvis–Judice–Ninke*; Strength 0.70. It is used **together with** the ordered pattern. |
| **Import settings** | *Row backgrounds: Respect existing raster layers (fixed rows)*; **Force Background** with a 16-colour picker |

What this tells us:
- **It works like png2c64's baseline:** an ordered pattern plus clamped error diffusion, with gamma 1.5. Gamma 1.5 is exactly our baseline flag. We are not behind on method.
- **A thumbnail gallery for ordered patterns.** This confirms our method picker (#260/#270) is the expected pattern. Ours goes further: real solves in the current palette, all families, search, and hover preview.
- **Ordered amount per R, G and B channel, and "Skew".** This is unusual and probably niche. It's worth one test row on a comparison sheet, not a feature.
- **Raster-aware backgrounds.** With a Rasters layer, the background colour can change per row ($D021 per raster line), and the import respects those fixed rows. This is a real extra-colour technique Convert64 doesn't model. Mosaicus has a related mode (a background per 4 × 1 cell, which needs a SuperCPU).
- **No per-area recipes, no masks, no pins, no re-solve after import.** That remains Convert64's ground.

## 3. Worth taking into Convert64

Ordered by value for Convert64 and *No Glory*.

1. **Round-trip with Pixcen. High value, the most practical item here.**
   - Make sure our Koala, Art Studio and raw exports open cleanly in Pixcen 2: a test in the VICE-check style, done once by hand.
   - Then consider an **"Open in Pixcen" export**, either in its `.gpx` project format or as a plain Koala, with our cell grid as a Pixcen **Guides** layer.
   - The original Pixcen (GPL-3.0, <https://github.com/Hammarberg/pixcen>) documents the 1.x `.gpx` format in its source. Read it as a format reference only, because GPL code can't be copied into MIT Convert64.
   - The 2.0 format may differ: Deluxe 2.0 credits miniz and nlohmann JSON, which suggests a zipped JSON format. Ask the Deluxe author before building on it.
   - The reverse direction already exists as E16: "import C64 files as locked layers".
2. **Vector quantisation for character sets. High value for E10 and the *No Glory* maps.**

   ![Vector quantisation](https://pd.elysium.pl/v2/images/feature-quantization-v2-793.webp)

   Pixcen builds a dictionary of 8 × 8 blocks with these parameters:
   - dictionary size (256);
   - epsilon;
   - tie-breaking *Minimum distortion*;
   - centroid *Majority voting (for discrete data)*;
   - initialisation *Farthest-first spread*;
   - max iterations;
   - **process all frames**, so animation frames share blocks;
   - **Set priorities**, which weights the areas that must stay exact;
   - *Convert losslessly*;
   - an optional second "Optimise dictionary" pass.

   Together with png2amiga's tile budget and reserve, this is a complete parameter list for E10's character solver:
   - "majority voting" is the right centroid for 1-bit and 2-bit pixel data;
   - "priorities" maps naturally onto our pins and regions;
   - "all frames" maps onto our sprite and charset sets.
3. **A raster background layer (a `$D021` colour per row). Medium value, later.**
   - It is an M3 "scene feature", like FLI.
   - It could be a document-level setting: background per row, fixed or solved.
   - The solver already searches background colours per picture; per row is the same search with a different scope.
   - Note it in EPICS under E13/E15 as a separate mode.
4. **All Borders layout (sprites in the borders). Medium value, for E11.** It's a good reference for how to show hardware sprite regions: yellow sprite boxes on the canvas, plus a parent layer with Upper, Left, Right and Lower children. Combine it with Mosaicus's sprite pins and keep-out boxes when designing E11's overlay.
5. **Layer list details. Low effort; adopt them in the layers sidebar.**
   - **A subtitle under each layer name** saying what it is: "Bitmap MC", "Rasters Layer", "Guides Layer". For us that would be "Floyd–Steinberg · 100 %" or "Region · Atkinson".
   - **Child rows** for the parts of a composite: border-sprite parts for them, regions under their layer for us. That matches the hierarchy we drew on 2026-10-24.
   - **Opacity, blend and offset X/Y directly under the list** for the selected layer.
   - **A Guides layer** that is never exported: construction lines and notes. Mosaicus has the same idea.
6. **Packed PRG output. Low effort, if the dependency is approved.** Pixcen bundles TSCrunch and ByteBoozer to make compact, runnable PRGs, plus an embedded assembler (x65) to build its viewers. An optional "packed" PRG export would help *No Glory* disk budgets. Any packer is a new TAD §4 dependency, so it needs Jaakko's approval, and licences must be checked first.
7. **A New Project dialog, for when charsets and sprites arrive.**
   - Pixcen's asks for: graphics family, content type, layout, size, frames, background, border, *Enable rasters*, *Enable guide layer*.
   - It shows a live summary such as "Chars: 40 × 25 = 1000".
   - Today Convert64 starts from a picture, which is right for bitmaps. Starting from a target (a charset of N characters, a sprite set) needs this dialog.

**Not to take:**
- **Pixel-drawing tools.** ADR-003 rules them out; finishing touches belong in Pixcen, which is item 1.
- **Animation timeline and colour cycling.** These are out of scope for a converter. Sprite animation frames, if ever, belong to E11 sets, not a timeline.
- **JavaScript scripting.** Our command line and MCP already give automation, with byte-identical output; a second scripting surface isn't worth it.
- **A Qt or native UI.** See ADR-006 and the 2026-10-29 Qt Bridge review.

## 4. Suggested next steps

1. **Jaakko: install Pixcen 2 on the Mac and run two checks.** It's a 74 MB download from the official site.
   - **(a)** Open our Koala exports of the six test pictures. Do they load unchanged?
   - **(b)** Import the six source pictures with Pixcen's Color Reduction at its defaults and at png2c64-like settings (checker, gamma 1.5). Save the results as PNG. They become one more comparison column, judged by eye as before.
2. **Issues filed:**
   - **#320:** Pixcen round-trip test, plus an "Open in Pixcen" export (E16).
   - **#321:** E10 design note on character quantisation parameters from Pixcen and png2amiga.
   - **#322:** Layer-row subtitles, regions as child rows, opacity under the list, and a Guides layer.
   - **#323:** Raster background mode, a background colour per row band (M3).
3. **Contact (optional, Jaakko's call).** The Deluxe author is reachable through Elysium. Since the tools are complementary, it may be worth asking about a documented `.gpx` 2.0 format, or agreeing a simple exchange format.

## Sources

- [Pixcen Deluxe 2.0](https://pd.elysium.pl/v2/)
- [Authors and libraries](https://pd.elysium.pl/v2/authors/)
- [Download](https://pd.elysium.pl/v2/download/)
- [Pixcen Deluxe 1.x site](https://pd.elysium.pl/index-v1.html)
- [The Oasis BBS on Pixcen Deluxe 1.0.7](https://theoasisbbs.com/pixcen-deluxe-c64-pixel-editor-the-tool-your-vic-ii-has-been-waiting-for/)
- [Original Pixcen, GPL-3.0](https://github.com/Hammarberg/pixcen/)
- Pixcen macOS port, GPL-3.0: `JackPearse/pixcenMacOS` on GitHub
- Screenshots: [app](https://pd.elysium.pl/v2/images/hero-app-1263.webp), [import](https://pd.elysium.pl/v2/images/feature-import-settings-v2-793.webp), [layers](https://pd.elysium.pl/v2/images/feature-layers-v2-793.webp), [quantisation](https://pd.elysium.pl/v2/images/feature-quantization-v2-793.webp), [borders](https://pd.elysium.pl/v2/images/feature-borders-v2-793.webp), [new project](https://pd.elysium.pl/v2/images/hero-new-project-385.webp)
