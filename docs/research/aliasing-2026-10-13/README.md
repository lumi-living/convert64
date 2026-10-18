# Aliasing on large sources (evidence for #235)

`aliasing-sheet.png`: png2c64 1.0 (`--mode multicolor --palette pepto --dither checker --dither-strength 0.7 --error-clamp 0.1 --gamma 1.5`) on 4096 × 2560 sources, run directly and on the same source area-averaged (box) to 320 × 200 first.

- **Top row:** a zone plate. The correct result beyond the centre is flat grey; the direct run fills the frame with aliasing rings.
- **Bottom row:** the `title` fixture upscaled to 4096 px with Gaussian grain (σ = 14/255). Against the same source without grain, 24.5 % of the direct run's output pixels change, versus 2.8 % for the prefiltered run.

Cause: png2c64's `scale::bicubic` always reads 4 taps per output pixel, whatever the shrink. Convert64's compositor has done the same since ADR-012 (#233).
