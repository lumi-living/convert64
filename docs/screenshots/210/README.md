# E8 exit: the *No Glory* workflow (#210)

The title screen workflow end to end in the app, natively and in the browser, in both
themes. Two layered sources (the `title` fixture with the `portrait` fixture on top), a
mask on the portrait (a radial gradient, so the face sits in the snow scene), a pin on
the face's cells, then export, then the export shown by VICE.

| Step | Native | Browser |
|---|---|---|
| 1. Open the title, add the portrait as a layer | `native/01-layers-*.png` | `browser/01-layers-*.png` |
| 2. Mask mode: a radial gradient on the portrait | `native/02-mask-*.png` | `browser/02-mask-*.png` |
| 3. Result view: pins on the face | `native/03-pin-*.png` | `browser/03-pin-*.png` |
| 4. Export | `native/04-export-*.png` | `browser/04-export-*.png` |
| 5. The exported file in VICE 3.10 | `native/05-vice.png` | `browser/05-vice.png` |

- **Native** ran through `native-workflow.sh`: the app opens the pictures from the command
  line and runs the mask and pin as registry commands (`--command`), the same commands the
  panels send; `--export` wrote `source.kla` (10 003 B) and the display PRG
  (10 260 B) with the Export dialog's code.
- **Browser** is the `trunk build --release` bundle, driven in Chromium through Playwright:
  File › Open… and Layer › New Layer from File… through the browser's file picker, `M`
  then the Gradient tool dragged on the face, `P` and a drag over the face for the pins,
  `Cmd E` and Export PRG. The PRG's bytes were read from the download link the page made.
- **VICE**: `x64sc` 3.10 with `cargo xtask vice-check`'s settings (PAL, Pepto, no filter)
  autostarted each PRG; the 320 × 200 window of its screenshot, mapped to the nearest Pepto
  colour, matches the exported Koala in 64 000 of 64 000 pixels, natively and in the
  browser.
