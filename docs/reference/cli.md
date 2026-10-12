# The `convert64` command line

`convert64` does headlessly what the app does (CLAUDE.md non-negotiable 4): every command reads PNGs, `.convert64` projects ([project-file.md](project-file.md)) or picture files, and writes legal C64 files. `convert64 --help` and `convert64 <command> --help` list every option.

## Exit codes

Every command exits with one of these:

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | The result is not legal VIC-II data, or it has more clashing cells than `--max-clashes`. Files are still written, so the result can be inspected. `validate` exits 1 for an illegal file. |
| 2 | An input cannot be read, or an option or a command line is invalid (also clap's usage errors). Nothing is written. |
| 3 | An output file cannot be written. |

## `--json`

`--json` (anywhere on the command line) prints one JSON object on stdout instead of the human-readable lines. Errors still go to stderr as well. Every report starts with:

| Field | Meaning |
|---|---|
| `command` | `convert`, `validate`, `recipe apply`, `recipe commands`, or `usage` for a command line that could not be parsed |
| `ok` | `exit_code == 0` |
| `exit_code` | As above |
| `error` | The error message, or `null` |
| `timing_ms` | `total`, plus each phase the command ran: `load`, `solve`, `write`, `validate` |
| `files_written` | Every file written, in order, as given on the command line |

followed by the command's own fields.

## `convert`

```sh
convert64 convert <input.png | project.convert64 | recipe.json> -o <out> [-o <out> …] [options]
```

A PNG is scaled to fill the 320 × 200 screen with the default recipe (Floyd–Steinberg at 100 %, Pepto PAL). A project uses its own settings. The options below override either: recipe options apply to every layer (not to regions, whose recipes are deliberate exceptions), the rest to the project.

The picture is solved once and written in each `-o`'s format, from its extension or `--format`:

| Extension | `--format` | Output |
|---|---|---|
| `.kla`, `.koa` | `kla` | Koala Painter; multicolor only |
| `.art`, `.aas` | `art` | Art Studio; hi-res only |
| `.prg` | `prg` | A self-displaying PRG (`LOAD"…",8` then `RUN`) embedding the Koala or Art Studio file ([file-formats.md](file-formats.md#koala--art-studio-display-prg-convert64_formatsprgdisplay_prg)) |

For a PNG, a `.kla` or `.art` output also picks the mode when `--mode` is not given. An output that disagrees with the mode is an error (exit 2).

| Option | Overrides | Values |
|---|---|---|
| `--mode` | `target` | `koala` (also `mc`, `multicolor`), `hires` (also `hi-res`, `art`) |
| `--palette` | `palette` | `pepto` / `pepto-pal`, `pepto-ntsc`, `colodore`, or a `.vpl` path |
| `--dither` | each layer's `recipe.method.kind` | Any id in [dither-methods.md](dither-methods.md) |
| `--strength` | `recipe.strength` | 0–1 |
| `--metric` | `recipe.metric` | `oklab`, `cielab`, `pepto-yuv`, `rgb`; png2c64's `mse`, `blur`, `ssim` set `cell_metric` |
| `--cell-metric` | `recipe.cell_metric` | `mse`, `blur`, `ssim` |
| `--no-cell-clamp` | `recipe.cell_clamp` | |
| `--max-luma-diff` | `recipe.max_luma_diff` | 0–8 |
| `--error-clamp` | `recipe.error_clamp` | 0–1 |
| `--adaptive` | `recipe.adaptive` | 0–1 |
| `--mix-levels` | `recipe.mix_levels` | 0–16 |
| `--smooth` | `recipe.smooth` | |
| `--gamma` | each layer's `adjustments.gamma` | 0.1–8 |
| `--match-range` | `adjustments.match_range` | |
| `--bg` | `solve.background` | `auto` or 0–15 |
| `--hysteresis` | `solve.hysteresis` | (0, 1] |
| `--supersample` | `supersample` | 1–8 |

Other options:

| Option | Meaning |
|---|---|
| `--max-clashes N` | Exit 1 when more than N cells clash |
| `--report` | Print the clash summary; with `--json`, the ten worst cells in `worst` |
| `--save-project PATH` | Also save the project, overrides applied, as `.convert64` (sources linked relative to it) |
| `--embed` / `--link` | With `--save-project`: embed every source PNG, or link every one |

JSON fields: `input`, `mode`, `palette` (its name), `outputs` (`path`, `format`), `background`, `clashes`, `max_clashes`, `legal`, `violations`, `solve_error`, `summary` (`clashes`, `total_error`, `dropped_cost`, `worst`, `luma_crossings`) and `worst`.

A `Makefile` rule:

```make
build/title.kla: art/title.convert64 art/title.png
	convert64 convert $< -o $@
```

## `validate`

```sh
convert64 validate <file> [--format kla|art]
```

Checks a picture against [vic-ii-constraints.md](vic-ii-constraints.md) §6 and prints `legal` (exit 0) or every violation (exit 1). JSON fields: `file`, `format`, `mode`, `legal`, `violations` (`rule`, `location`, `message`).

## `recipe apply`

```sh
convert64 recipe apply <recipe.json | project.convert64> [--save out.convert64 [--embed | --link]] [--mask LAYER=PATH]… [--composite PATH] [--attributes PATH] [--report]
```

Loads a project or a version-1 recipe, runs its commands, and writes what is asked: the project (`--save`), a layer's evaluated mask, the composite or the attribute map as PNGs, and with `--report` the solve's clash summary (exit 1 if the solve were illegal). JSON fields: `file`, `layers`, `commands_applied`, and with `--report` the solve fields of `convert`.

## `recipe commands`

Lists the registry's commands, the ones a recipe or project's `commands` can contain. JSON field: `commands` (`name`, `summary`).
