# SSIMULACRA2 reference scores

`reference.toml` holds png2amiga's SSIMULACRA2 score for every pair that
`tests/ssimulacra2.rs` builds from the fixtures. The test recomputes each pair with
`convert64_core::quality::ssimulacra2` and fails on a difference above 1e-4. Our port
matches the scores bit for bit.

## The reference build

- png2amiga commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2), the ADR-013
  pin. Its `src/ssimulacra2.cpp`, `ssimulacra2.hpp`, `types.hpp`, `aligned_vector.hpp`
  and `color_space.hpp` are linked with `harness.cpp` in this directory.
- **Scalar backend.** On arm64, `ssimulacra2.cpp` picks NEON. The harness build turns that
  off with one line, so the scalar loops run:

  ```sh
  sed 's/^#elif defined(__ARM_NEON) || defined(__aarch64__)$/#elif 0/' \
      png2amiga/src/ssimulacra2.cpp > ssimulacra2_scalar.cpp
  ```

- **Strict IEEE.** `g++-16 -std=c++26 -O2 -ffp-contract=off -I png2amiga/src harness.cpp
  ssimulacra2_scalar.cpp`. No other shims were needed. GCC 15.3.0, GCC 16.2.0 and Apple
  clang 21 give the same bits.

png2amiga's release builds use `-ffp-contract=fast` and NEON or AVX2. Those builds fuse
the blur's multiply-adds, so their scores move in the third to fifth decimal; on these
pairs the NEON build is off by up to 0.03 (the 37 × 29 crop) and usually by less than
5e-4. The scalar build is the reproducible one, and it is the one we match.

## Regenerating

```sh
CONVERT64_S2_DUMP=/tmp/s2 cargo test -p convert64-core --test ssimulacra2
./harness /tmp/s2/*.bin    # name, score, f32 bits per pair
```

`harness --constants` prints the two values png2amiga computes with the C library
(`cbrtf` of the XYB bias and the `expf` blur kernel). `ssimulacra2.rs` stores them as
bits, so the score does not depend on the platform's libm.
