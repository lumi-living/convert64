#include "color_space.hpp"
#include "palette.hpp"
#include "preprocess.hpp"
#include "scale.hpp"
#include <cstdio>
#include <cstdint>
#include <cstring>
using namespace png2c64;
static uint32_t st = 12345u;
static uint32_t rnd() { st = st * 1103515245u + 12345u; return (st >> 16) & 0xFFFFu; }
static uint32_t B(float f) { uint32_t u; std::memcpy(&u, &f, 4); return u; }
static void p3(float a, float b, float c) { std::printf("[0x%08x, 0x%08x, 0x%08x]", B(a), B(b), B(c)); }
int main() {
  auto pal = palette::by_name("pepto");
  std::printf("PALETTE_LINEAR\n"); for (auto& c : pal.colors) { p3(c.r, c.g, c.b); std::printf(",\n"); }
  std::printf("PALETTE_OKLAB\n"); for (auto& c : pal.colors) { auto l = color_space::linear_to_oklab(c); p3(l.L, l.a, l.b); std::printf(",\n"); }
  std::printf("RANDOM\n");
  for (int i = 0; i < 64; ++i) { uint8_t r = rnd() & 255, g = rnd() & 255, b = rnd() & 255;
    auto l = color_space::linear_to_oklab(color_space::srgb_u8_to_linear(r, g, b));
    std::printf("([%u, %u, %u], ", r, g, b); p3(l.L, l.a, l.b); std::printf("),\n"); }
  std::printf("LUT\n"); for (int i = 0; i < 256; ++i) std::printf("0x%08x,%s", B(color_space::srgb_u8_to_linear((uint8_t)i,0,0).r), i%8==7?"\n":" ");
  Image im(4, 2); std::printf("PRE_IN\n");
  for (int i = 0; i < 8; ++i) { uint8_t r = rnd()&255, g=rnd()&255, b=rnd()&255; im.pixels()[i] = color_space::srgb_u8_to_linear(r,g,b); std::printf("[%u, %u, %u],\n", r,g,b);}
  preprocess::Settings s; preprocess::apply(im, s);
  std::printf("PRE_GAMMA\n"); for (auto& p : im.pixels()) { p3(p.r, p.g, p.b); std::printf(",\n"); }
  preprocess::match_palette_range(im, pal);
  std::printf("PRE_MATCH\n"); for (auto& p : im.pixels()) { p3(p.r, p.g, p.b); std::printf(",\n"); }
  // bicubic: 7x5 random image -> 3x4 (Mitchell) and -> 9x6 (Catmull-Rom)
  Image src(7, 5); for (auto& p : src.pixels()) p = color_space::srgb_u8_to_linear(rnd()&255, rnd()&255, rnd()&255);
  std::printf("SCALE_SRC\n"); for (auto& p : src.pixels()) { p3(p.r, p.g, p.b); std::printf(",\n"); }
  auto a = scale::bicubic(src, 3, 4); std::printf("SCALE_MITCHELL_3x4\n"); for (auto& p : a->pixels()) { p3(p.r, p.g, p.b); std::printf(",\n"); }
  auto b = scale::bicubic(src, 9, 6, scale::Kernel::catmull_rom); std::printf("SCALE_CATMULL_9x6\n"); for (auto& p : b->pixels()) { p3(p.r, p.g, p.b); std::printf(",\n"); }
}
