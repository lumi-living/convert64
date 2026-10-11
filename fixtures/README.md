# Fixtures

Source images for golden tests. Each folder holds `source.png` (the input) and a `README.md` stating origin, licence and what the image tests. Golden outputs (`expected.*`) and png2c64 baselines (`baseline.*`) are added by #39–#41.

| Fixture | Mode exercised | Why |
|---|---|---|
| [portrait](portrait/) | multicolor bitmap | skin tones, same-luma dither pairs, cell clashes |
| [terrain](terrain/) | charset / tiles | 960 candidate chars vs the 256 budget, per-tile colours |
| [title](title/) | multicolor bitmap | smooth sky gradient + detailed foreground; per-layer recipes |
| [gradient](gradient/) | all | deterministic regression chart with exact Pepto swatches |
| [photo](photo/) | multicolor bitmap | dynamic range, natural texture, metric choice |
| [lineart](lineart/) | hi-res bitmap | two colours per cell, thin strokes |

Rules for new fixtures are in [CONTRIBUTING.md](../CONTRIBUTING.md): rights-clear only, PNG, ≤ 2 MB, 16:10, and a README. Keep `fixtures/_local/` (git-ignored) for anything you cannot redistribute.
