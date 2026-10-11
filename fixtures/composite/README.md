# Composite scenes

Layered scenes for the E4 compositor and attribute map (#4, ADR-002, ADR-008). Each
folder holds:

| File | What it is |
|---|---|
| `scene.toml` | target mode, supersample factor, layers (source fixture, transform, opacity, blend, recipe, adjustments, mask sources and modifiers) and regions |
| `expected_1x.png` | the composite box-filtered to the target grid (160×200 or 320×200), straight sRGB, alpha = coverage |
| `expected_attr.png` | the attribute map's debug render: hue = dither method, lightness = strength, black = uncovered |
| `README.md` | what the scene tests |

The scenes reuse the `portrait` and `title` sources; their origin and licence are in
[../portrait/README.md](../portrait/README.md) and [../title/README.md](../title/README.md).
No new image material is added here.

The golden harness (`crates/convert64-core/tests/golden.rs`, module `composite`)
renders each scene and compares it with the PNGs, allowing ±2 per channel for last-bit
differences in platform maths. Regenerate with `cargo xtask regen-goldens --accept
--reason "..."`, which compares exactly and logs the reason in [../CHANGELOG.md](../CHANGELOG.md).
