# Bundled fonts

The app's text fonts (ADR-007, `design/tokens.json` → `egui / Text`). Both
families are licensed under the SIL Open Font License 1.1; the licence texts
ship next to the fonts. `cargo xtask tokens` maps the family named in the
tokens to `<Family without spaces>-Regular.ttf` here (and `-Medium.ttf` for
the proportional family, if present) and fails if the file is missing.

| File | Family / weight | Release | Size (bytes) | SHA-256 |
|---|---|---|---|---|
| `Inter-Regular.ttf` | Inter Regular (`FontFamily::Proportional`) | [rsms/inter v4.1](https://github.com/rsms/inter/releases/tag/v4.1), `Inter-4.1.zip` → `extras/ttf/` | 411 640 | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| `Inter-Medium.ttf` | Inter Medium (named family `medium`, for headings and emphasis) | same archive | 417 300 | `97ad806f526e41546d46365bb3a393145f75b7b1568913db74549ad8b8dba872` |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono Regular (`FontFamily::Monospace`) | [JetBrains/JetBrainsMono v2.304](https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304), `JetBrainsMono-2.304.zip` → `fonts/ttf/` | 273 900 | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| `OFL-Inter.txt` | Inter's `LICENSE.txt` from the same archive | | | |
| `OFL-JetBrainsMono.txt` | JetBrains Mono's `OFL.txt` from the same archive | | | |

Archive SHA-256: `Inter-4.1.zip` `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`,
`JetBrainsMono-2.304.zip` `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf`
(both downloaded 2026-10-13; the latest releases on that date).

The three files add about 1.1 MB to the binary and the wasm bundle (before
transport compression). They are the unmodified static TTFs; subsetting them
is possible later if the web download needs to shrink. The icon font comes
from the `egui_material_icons` crate (Material Symbols, Apache-2.0), not from
here.

To update: download the release archive from the upstream GitHub release only,
copy the same files and licence, update this table and run `cargo xtask tokens`.
