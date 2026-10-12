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
