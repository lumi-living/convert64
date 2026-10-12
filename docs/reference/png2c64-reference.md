# png2c64 as the reference solver (#181)

`convert64_core::solve::png2c64_ref` ports png2c64's bitmap pipeline so that our output
is pixel-identical to png2c64's. This page records which png2c64 build is "the
reference", why, and how the test data in the port was produced.

## The reference build

png2c64 commit `1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f` (1.0.0.76), built with
Homebrew GCC 15.3 on macOS arm64 with strict IEEE arithmetic:

```sh
git clone https://github.com/tinic/png2c64 && cd png2c64
git checkout 1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f
cmake -B build-strict -DCMAKE_CXX_COMPILER=g++-15 -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_CXX_FLAGS="-ffp-contract=off"
cmake --build build-strict --target png2c64
```

**Why strict.** GCC's default on arm64 fuses `a * b + c` into one fused multiply-add
wherever it likes; clang fuses within an expression. Error diffusion, and png2c64's
global Floyd–Steinberg pass that picks cell colours for the diffusion methods and `none`,
amplify a one-ulp difference into a different picture. Pixels agreeing with the strict
build, same commit, same input:

| Render | GCC 15 default | Apple clang 21 | #113's committed render |
|---|---|---|---|
| card `fs` | 98.853 % | 100 % | 100 % |
| card `fs-wide` | 94.487 % | 94.487 % | 100 % |
| card `line-fs` | 90.316 % | 90.316 % | 100 % |
| card `ostromoukhov` | 84.906 % | 83.709 % | 85.272 % |
| portrait `fs` | 100 % | 100 % | 61.850 % |
| portrait `line-fs` | 100 % | 100 % | 86.000 % |
| portrait `ostromoukhov` | 76.513 % | 76.616 % | 61.744 % |
| portrait `none` | 100 % | 100 % | 99.659 % |

Rust never fuses, so the strict build is the one a portable port can match bit for bit
on every platform. Two library functions remain: `std::cbrt` and `std::pow` on `float`.
The port computes both in `f64` and rounds once, which is correctly rounded; macOS's
`cbrtf` is correctly rounded (checked on 150 million inputs), and a strict build with
`std::pow` replaced by the correctly rounded version renders all 60 previews identically,
so the reference does not depend on the last bit of `powf`.

The fixture baselines (`fixtures/*/baseline.*`) were made with the GCC default build;
the strict build reproduces them on ≥ 99.997 % of pixels (the six use `checker` and
`none` hi-res, which are robust), so they stay as committed.

The dither previews (`dither-previews/png2c64-card/`, `png2c64-portrait/`) were first
rendered in #113 with an unrecorded clang 18 build. No build of the pinned commit
reproduces nine of them: the seven diffusion methods and `none` on the portrait, and
`ostromoukhov` on the card. GCC default, GCC strict and Apple clang agree with #113's
renders on only 61–99.7 % of those pixels (table above). They are regenerated with the
strict build (`cargo xtask png2c64-previews`); the other 51 came out byte-identical.

## Test data

The bit patterns in `crates/convert64-core/src/solve/png2c64_ref/testdata.rs` come from
[`png2c64-reference/colour.cpp`](png2c64-reference/colour.cpp), compiled against
png2c64's own sources with the reference flags:

```sh
g++-15 -O3 -std=c++26 -ffp-contract=off -I png2c64/src colour.cpp \
    png2c64/src/preprocess.cpp png2c64/src/scale.cpp -o colour && ./colour
```

## The parity test

`crates/convert64-core/tests/png2c64_parity.rs` runs the reference solver on every one of
png2c64's 30 method ids on the test card (strength 1.0) and the portrait (png2c64's
defaults), and on every fixture with its baseline's recorded settings, and requires at
least 99 % of pixels identical to png2c64's render in each case. CI runs it as its own
step (`png2c64 parity`) and prints every percentage. With the reference build all 66
cases are 100 % except the `title` fixture at 99.997 % (2 pixels), whose baseline the
GCC default build made.

## Divergences

Our extras that png2c64 does not have are named options of the reference solver
(`solve::Divergences`; project `solve.divergences`; CLI `--diverge`), each off by
default. With all off the solver is png2c64's and the parity test holds. Each has a row
on the divergence sheet (`dither-previews/convert64-sheets/sheet-divergences.png`) next
to the reference, and becomes a default only after a human has judged it there.

| Option | What it does on png2c64's pipeline | From |
|---|---|---|
| `chroma_guard` | An ordered pixel's search and pick avoid colours that add chroma it lacks: a neutral pixel (OKLab chroma ≤ 0.02) keeps to colours within 0.03 of its chroma, a tinted one to colours within about 53° of its hue (greys always pass). Rejected colours cost +1 in the distance table. | #157 |
| `max_luma_diff = n` | A pixel's colours other than those within `n` luma levels of its nearest colour cost +1 in the search, and diffusion picks only among the cell's colours within `n` levels of the one the pixel itself is nearest to. 0 is PAL-blend safe. | #123 |
| `pair_bias = x` | A cell set's search score gets `x` per same-luma pair in it. | E5 |
| `cell_clamp` | Diffused error is not passed into another cell whose colours lack the nearest colour to where it would push that pixel. png2c64 lets error flow everywhere. | E5 |
| `mix_levels = n` | An ordered pixel's threshold is quantised to `n` levels before it biases the search, so a pair mixes at one of `n + 1` ratios. | #142 |
| `bridging` | A cell set's search score gets, per ordered pixel, 0.01 × the squared width of the OKLab-L bracket the set gives it (½ plus the distance to the nearest end when unbracketed). | #157 |
