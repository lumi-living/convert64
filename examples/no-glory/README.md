# No Glory: a title screen built by `make`

A small game project's art pipeline in miniature: the title screen is a `.convert64` project next to its source PNG, and `make` turns it into C64 files with the `convert64` command line. No GUI is involved, so the build runs in CI (`.github/workflows/ci.yml`) and on any machine with `convert64` on its `PATH`.

```sh
make                                   # build/title.kla, .prg, .png, .asm (+ .bin)
make check                             # ... and validate the .kla and .prg
x64sc build/title.prg                  # look at it in VICE
make CONVERT64=../../target/debug/convert64 check   # inside this repository
```

## Files

| File | What it is |
|---|---|
| `art/title.png` | The painting, 320 × 200: a winter dusk, spruce on the horizon, a road through snow, the words NO GLORY. |
| `art/title.convert64` | The project ([schema](../../docs/reference/project-file.md)): three layers sharing the painting's pixels, each with its own dither. The sky is Bayer 4 × 4 at 80 %, the snow below the horizon Floyd–Steinberg at 60 % (a gradient mask), and the lettering band undithered so the letters stay crisp. Multicolor, Pepto PAL. |
| `Makefile` | From the project: `build/title.kla` (Koala), `build/title.prg` (self-displaying), `build/title.png` (a 2× preview) and `build/title.asm` (KickAssembler source including the raw set's `.bin` files, written beside it). `check` validates the Koala file and the PRG; `CONVERT64` names the binary. |
| `tools/make_title.py` | Draws `art/title.png` with the Python standard library; the output is byte-identical on every run. |

To change the look, edit the project's recipes (any method in [dither-methods.md](../../docs/reference/dither-methods.md)) or try options without editing it:

```sh
convert64 convert art/title.convert64 --dither blue-noise --palette colodore -o build/try.prg
convert64 export art/title.convert64 --asm all --out build   # ACME, 64tass, ca65 and C too
```

## Licence

`art/title.png` and `tools/make_title.py` are dedicated to the public domain under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/): use them for anything, without attribution. The rest of this directory is under the repository's MIT licence.
