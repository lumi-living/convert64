/*
 * The reference build's maths shim (docs/reference/png2amiga-reference.md).
 *
 * png2amiga calls `std::pow`, `std::sin`, `std::cos`, `std::atan2` and `std::exp` on
 * `float`, which reach the platform's libm (`powf`, `sinf`, `cosf`, `atan2f`, `expf`).
 * Those are not the same function on every platform: macOS's `atan2f` differs from the
 * correctly rounded value on about 6 % of inputs, its `powf` on some. Linked into the
 * reference binary, these definitions take the place of libm's: each computes in double
 * and rounds once, which is what Convert64's port does (`solve::png2amiga_ref`), so the
 * reference and the port evaluate the same function on every platform.
 */
#include <math.h>

float powf(float x, float y) { return (float)pow((double)x, (double)y); }
float sinf(float x) { return (float)sin((double)x); }
float cosf(float x) { return (float)cos((double)x); }
float atan2f(float y, float x) { return (float)atan2((double)y, (double)x); }
float expf(float x) { return (float)exp((double)x); }
