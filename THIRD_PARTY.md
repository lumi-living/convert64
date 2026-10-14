# Third-party code

## png2c64

`crates/convert64-core/src/solve/png2c64_ref/` is a port to Rust of parts of
[png2c64](https://github.com/tinic/png2c64) by Tinic Uro, at commit
`1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f` (version 1.0.0.76): `color_space.hpp`,
`palette.hpp`, `preprocess.cpp`, `scale.cpp`, `quantize.cpp` and `dither.cpp`. It is the
reference solver of #181; each file names the source file it ports.

png2c64's README declares the MIT licence. The repository carries no `LICENSE` file at
that commit, so the standard MIT text is reproduced here with the author's name:

```
MIT License

Copyright (c) 2026 Tinic Uro

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## png2amiga (C64 path)

`crates/convert64-core/src/solve/png2amiga_ref/` is a port to Rust of png2amiga's C64
bitmap path by Tinic Uro, at commit `4a8777653536ac07074a650ee86860347c8755fb`
(v1.111.2): `scale.cpp`, `preprocess.cpp`, `color_space.hpp`, `dither.cpp`,
`dither.hpp`, `c64.cpp`, `cell_graph_refine.hpp`, and the tables `void_cluster_64.inc`
and `cluster_noise_64.inc` (as `void_cluster_64.bin`, `cluster_noise_64.bin`). It is the
reference solver since ADR-013 (#261); each file names the source file it ports. The same
MIT licence applies as for png2c64 above and SSIMULACRA2 below, with the same author.

## SSIMULACRA2 (png2amiga, Cloudinary, libjxl)

`crates/convert64-core/src/quality/ssimulacra2.rs` is a port to Rust of
`src/ssimulacra2.cpp` from [png2amiga](https://github.com/tinic/png2amiga) by Tinic Uro,
at commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2). png2amiga's README
declares the MIT licence and the repository carries no `LICENSE` file, as with png2c64
above; the same MIT text applies, with the same author.

png2amiga's file ports [SSIMULACRA2](https://github.com/cloudinary/ssimulacra2) by Jon
Sneyers (Cloudinary), whose metric and 108 weights it keeps. Cloudinary's code is under
the BSD 3-Clause licence:

```
Copyright (c) Cloudinary.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

SSIMULACRA2 began as `tools/ssimulacra2.cc` in [libjxl](https://github.com/libjxl/libjxl),
whose XYB transform it uses. libjxl's licence is the same BSD 3-Clause text with the
copyright line `Copyright (c) the JPEG XL Project Authors.`

## Fonts

`crates/convert64-app/assets/fonts/` bundles Inter (Regular, Medium; rsms/inter v4.1) and
JetBrains Mono (Regular; JetBrains/JetBrainsMono v2.304), unmodified, under the SIL Open
Font License 1.1. The licence texts are `OFL-Inter.txt` and `OFL-JetBrainsMono.txt` in
that directory; its `README.md` records the release and SHA-256 of every file. The
Material Symbols icon font is embedded by the `egui_material_icons` crate (Apache-2.0).
