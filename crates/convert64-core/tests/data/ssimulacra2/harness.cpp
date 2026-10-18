// Scores image pairs with png2amiga's SSIMULACRA2 (src/ssimulacra2.cpp), for the
// reference scores in reference.toml. See README.md in this directory.
//
//   harness --constants      prints the two constants png2amiga computes with libm
//   harness PAIR.bin...      prints "name score bits" per pair
//
// A pair file is what `tests/ssimulacra2.rs` writes with CONVERT64_S2_DUMP set: width
// and height (u32 LE), then the reference and the distorted image, each width × height
// linear RGB triples (f32 LE).
#include "ssimulacra2.hpp"
#include "types.hpp"

#include <bit>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using png2amiga::Color3f;

int main(int argc, char** argv) {
    if (argc > 1 && std::strcmp(argv[1], "--constants") == 0) {
        // As src/ssimulacra2.cpp computes them (kBiasCbrt, blur_kernel()).
        constexpr float kBias = 0.0037930732552754493f;
        volatile float bias = kBias;
        float cbrt = std::cbrt(static_cast<float>(bias));
        std::printf("kBiasCbrt %08x\n", std::bit_cast<std::uint32_t>(cbrt));
        float k[11];
        float sum = 0.0f;
        volatile float sigma = 1.5f;
        for (int i = 0; i < 11; ++i) {
            float x = static_cast<float>(i - 5);
            k[i] = std::exp(-0.5f * x * x / (sigma * sigma));
            sum += k[i];
        }
        for (int i = 0; i < 11; ++i)
            std::printf("kernel[%d] %08x\n", i, std::bit_cast<std::uint32_t>(k[i] / sum));
        return 0;
    }
    for (int a = 1; a < argc; ++a) {
        std::FILE* f = std::fopen(argv[a], "rb");
        if (!f) {
            std::fprintf(stderr, "cannot open %s\n", argv[a]);
            return 1;
        }
        std::uint32_t wh[2];
        if (std::fread(wh, sizeof wh, 1, f) != 1) return 1;
        std::size_t n = static_cast<std::size_t>(wh[0]) * wh[1];
        std::vector<Color3f> ref(n), dist(n);
        if (std::fread(ref.data(), sizeof(Color3f), n, f) != n) return 1;
        if (std::fread(dist.data(), sizeof(Color3f), n, f) != n) return 1;
        std::fclose(f);
        float s = png2amiga::ssimulacra2::compute(ref, dist, wh[0], wh[1]);
        std::string name = argv[a];
        name = name.substr(name.find_last_of('/') + 1);
        name = name.substr(0, name.rfind(".bin"));
        std::printf("%s %.9g %08x\n", name.c_str(), static_cast<double>(s),
                    std::bit_cast<std::uint32_t>(s));
    }
    return 0;
}
