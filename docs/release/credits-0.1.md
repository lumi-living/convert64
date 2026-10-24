# Credits for 0.1 — what Convert64 took from each tool

The source for the credit lines in [csdb-0.1.md](csdb-0.1.md) and
[forum-post-0.1.md](forum-post-0.1.md). Each line says what Convert64 took or learned,
with where the repository records it, so the release text never claims more than that.

| Tool | Author | What Convert64 took or learned | Where it is recorded |
|---|---|---|---|
| [png2c64](https://github.com/tinic/png2c64) | Tinic Uro (MIT) | **Code.** Convert64's reference solver is a Rust port of png2c64's colour pipeline, scaler, quantiser and dithers, at png2c64 1.0.0.76; a CI test keeps it identical to png2c64's own renders. Its 4-tap Mitchell resampling is used for every layer, and its outputs are the baselines the fixtures are checked against. | `THIRD_PARTY.md`, ADR-010, ADR-012, ADR-014, `docs/reference/png2c64-reference.md` |
| [png2amiga](https://github.com/tinic/png2amiga) | Tinic Uro (MIT) | **Code.** Its C64 path is ported as a *comparison* solver (`--solver png2amiga`), kept identical to its renders by a CI test; png2c64 stays the default because it looked better by eye on 5 of 6 test pictures. From it also: the opt-in Lanczos resampler (its widened Lanczos-3), and the SSIMULACRA2 port, shown in `--json` as information only. The opt-in line-art recipe runs on this solver. | `THIRD_PARTY.md`, ADR-013, ADR-014, `docs/research/2026-10-13-png2amiga.md` |
| [retropixels](https://github.com/micheldebree/retropixels) | micheldebree (MIT, archived 2026) | **Research, no code.** Its converter (frequency-based cell colours, ordered dither, colour spaces) was studied as a baseline that Convert64 chose not to follow; the Koala and Art Studio byte layouts were cross-checked against its documented writers. | `docs/research/2026-10-11-converter-algorithms.md`, `docs/reference/file-formats.md` |
| [ArcPaint64](https://www.doussis.com/arcpaint64/) | arch0N | **Vocabulary and pattern shapes, no code.** The *Adaptive* / *Force* colour-fitting terms, kept so scene users recognise them; the dither pattern shapes Dot, Diamond, Star, vertical lines, lines with a checker and diagonals, regenerated from our own formulas. It is also the pixel editor the docs suggest for finishing touches. | `docs/UX.md` §9, `docs/PDD.md`, `docs/reference/dither-methods.md` |
| [C64CC](https://www.elder0010.com/projects/c64cc/) | Elder0010 | **Ideas, no code.** GUI and CLI with the same features, for build pipelines (in 0.1: the app, `convert64` and the MCP server run the same commands). Its character reuse under a budget is noted for the charset mode, which is not in 0.1. | `docs/PDD.md`, `docs/TAD.md` |
| [ALBERT](https://www.albertpixels.com/) | — | **Ideas, no code.** Per-channel curves as an adjustment, and a luminance-weighted colour distance as an alternative to plain RGB; Convert64 has its own curves and a Pepto Y'UV luma-weighted metric option. | `docs/PDD.md`, `docs/research/2026-10-11-converter-algorithms.md` |

Also used, and worth a line where there is room: the Pepto and Colodore palettes (Philip
Timmermann, pepto.de), [VICE](https://vice-emu.sourceforge.io/) for checking every export,
and [PhotoCraft](https://github.com/storytold/photocraft) (storytold), whose
engine-first design and egui layer panels Convert64 follows.

Open points for Jaakko before posting:

- ALBERT's author is not named in the repository's research; add the name only if
  confirmed.
- Whether to state in the credits that the code was written with an AI coding assistant
  (the commit history says so).
