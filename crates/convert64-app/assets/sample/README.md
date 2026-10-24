# Sample picture

`title.png` is what the empty state's **Try a sample picture** opens (`docs/UX.md` §5,
#292). It is the `title` fixture's painting (`fixtures/title/source.png`, 1024 × 637,
954 kB) scaled to 640 × 398 (2× the C64 screen's width, still sub-pixel for the solver)
so the browser build carries 387 kB instead of 954 kB. Licence and origin are the
fixture's: see `fixtures/title/README.md`.

Made with ImageMagick 7.1.2:

```sh
magick fixtures/title/source.png -resize 640x398! -strip \
  -define png:compression-level=9 crates/convert64-app/assets/sample/title.png
```

| File | Size (bytes) | SHA-256 |
|---|---|---|
| `title.png` | 387 332 | `464707ed4e061ff6be02688616ce271891d07395e7c8f09f9cdb824e82f28c46` |
