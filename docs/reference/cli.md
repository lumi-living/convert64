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
| `command` | `convert`, `validate`, `batch`, `export`, `apply`, `commands`, `recipe apply`, `recipe commands`, or `usage` for a command line that could not be parsed |
| `ok` | `exit_code == 0` |
| `exit_code` | As above |
| `error` | The error message, or `null` |
| `timing_ms` | `total`, plus each phase the command ran: `load`, `apply`, `solve`, `write`, `validate`, `convert` (batch) |
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
| `.png` | `png` | A preview: every pixel exactly a palette colour, 320 × 200 (multicolor pixels two wide) times `--zoom` (1–8, default 1) |
| `.json` | `raw` | A raw set ([file-formats.md](file-formats.md#raw-bitmap-set-convert64---format-raw)): `<name>.bitmap.bin`, `<name>.screen.bin`, and for multicolor `<name>.colour.bin` and `<name>.bg.bin`, plus the `<name>.json` manifest, next to each other. `-o build/title.json` and `--format raw -o build/title` both write the set `title` in `build/`. |

For a PNG, a `.kla` or `.art` output also picks the mode when `--mode` is not given. An output that disagrees with the mode is an error (exit 2).

| Option | Overrides | Values |
|---|---|---|
| `--solver` | `solve.solver` | `reference` (default: png2c64's pipeline, #181), `native` |
| `--diverge` | `solve.divergences` | The reference solver's named divergences from png2c64, all off by default: `chroma-guard`, `max-luma-diff=N` (0–8), `pair-bias=X` (−1–1), `cell-clamp`, `mix-levels=N` (1–16), `bridging`; repeatable or comma-separated ([png2c64-reference.md](png2c64-reference.md#divergences)) |
| `--mode` | `target` | `koala` (also `mc`, `multicolor`), `hires` (also `hi-res`, `art`) |
| `--palette` | `palette` | `pepto` / `pepto-pal`, `pepto-ntsc`, `colodore`, or a `.vpl` path |
| `--dither` | each layer's `recipe.method.kind` | Any id in [dither-methods.md](dither-methods.md) |
| `--strength` | `recipe.strength` | 0–1 |
| `--metric` | `recipe.metric` | `oklab`, `cielab`, `pepto-yuv`, `rgb`; png2c64's `mse`, `blur`, `ssim` set `cell_metric` |
| `--cell-metric` | `recipe.cell_metric` | `mse`, `blur`, `ssim` |
| `--no-cell-clamp` | `recipe.cell_clamp` | native solver |
| `--max-luma-diff` | `recipe.max_luma_diff` | 0–8; native solver |
| `--ordered-semantics` | `recipe.ordered_semantics` | `png2c64` (default), `legacy`; native solver |
| `--no-chroma-guard` | `recipe.chroma_guard` | native solver |
| `--error-clamp` | `recipe.error_clamp` | 0–1 |
| `--adaptive` | `recipe.adaptive` | 0–1 |
| `--mix-levels` | `recipe.mix_levels` | 0–16; native solver |
| `--smooth` | `recipe.smooth` | native solver |
| `--gamma` | each layer's `adjustments.gamma` | 0.1–8 |
| `--match-range` | `adjustments.match_range` | |
| `--rotate` | each layer's `transform.rotate`, through `layer_rotate` after the project's own commands (#198) | Degrees, clockwise on screen, about the layer's anchor; the project's `snap` applies |
| `--anchor` | each layer's `transform.anchor`, through `layer_rotate` (the picture stays in place) | `X,Y`, 0–1 across the layer's box: `0,0` top-left, `0.5,0.5` the centre (default). Alone, it moves the pivot and keeps the angle |
| `--bg` | `solve.background` | `auto` or 0–15 |
| `--hysteresis` | `solve.hysteresis` | (0, 1]; native solver |
| `--supersample` | `supersample` | 1–8 |

Other options:

| Option | Meaning |
|---|---|
| `--max-clashes N` | Exit 1 when more than N cells clash |
| `--report` | Print the clash summary; with `--json`, the ten worst cells in `worst` |
| `--save-project PATH` | Also save the project, overrides applied, as `.convert64` (sources linked relative to it) |
| `--embed` / `--link` | With `--save-project`: embed every source PNG, or link every one |

JSON fields: `input`, `mode`, `palette` (its name), `outputs` (`path`, `format`), `background`, `clashes`, `max_clashes`, `legal`, `violations`, `solve_error`, `solver` (`reference` or `native`), `solver_fallback` (why the native solver ran, when the reference was asked for), `summary` (`clashes`, `total_error`, `dropped_cost`, `worst`, `luma_crossings`) and `worst`.

A `Makefile` rule:

```make
build/title.kla: art/title.convert64 art/title.png
	convert64 convert $< -o $@
```

## `validate`

```sh
convert64 validate <file> [--format kla|art|prg|project|raw]
```

Checks a file against [vic-ii-constraints.md](vic-ii-constraints.md) §6 and prints `legal` (exit 0) or every violation (exit 1). Every format convert64 reads:

| Format | How it is checked |
|---|---|
| Koala (`.kla`, `.koa`), Art Studio (`.art`, `.aas`) | Read, then the picture and its load address validated |
| Display PRG (`.prg`) | Its header and loader must be the ones `convert64` writes, byte for byte (exit 2 otherwise); the embedded Koala or Art Studio file is then validated |
| Raw set (its `.json` manifest) | The manifest read, every block file beside it checked for size (rule 6) and the picture validated |
| Project (`.convert64`, or a version-1 recipe `.json`) | Loaded, solved, and the solved picture validated; `clashes` is reported |

The format comes from `--format`, else the extension (a `.json` whose `format` is `convert64-raw-bitmap` is a raw set, any other a recipe), else the size of a picture file. JSON fields: `file`, `format`, `mode`, `legal`, `violations` (`rule`, `location`, `message`), and `clashes` for a project.

## `batch`

```sh
convert64 batch <dir> [--manifest batch.toml] [--out build/] [-j N] [convert options]
```

Converts every PNG and `.convert64` project in `dir` (not its subdirectories), plus any file the manifest names, `N` at a time (default: one per CPU). A PNG with a project of the same name next to it is skipped, because the project usually links it. Outputs are `<out>/<name>.<format>`.

The manifest is TOML; `dir/batch.toml` is used when `--manifest` is not given and it exists:

```toml
out = "build"                 # relative to the manifest; default: dir

[defaults]                    # every file
dither = "bayer4"
outputs = ["kla", "prg"]      # default: the mode's picture format (kla or art)

[files."title.png"]           # one file, relative to dir
mode = "hires"
strength = 0.8
max-clashes = 20

[files."draft.png"]
skip = true
```

Every table takes `convert`'s options in kebab-case (`solver`, `diverge` (a list), `mode`, `palette`, `dither`, `strength`, `metric`, `cell-metric`, `no-cell-clamp`, `max-luma-diff`, `ordered-semantics`, `no-chroma-guard`, `error-clamp`, `adaptive`, `mix-levels`, `smooth`, `gamma`, `match-range`, `bg`, `hysteresis`, `supersample`), plus `outputs` (`kla`, `art`, `prg`, `png`, `raw`), `max-clashes`, `zoom` (PNG previews) and `skip`. Unknown keys are errors. A file's table wins over `[defaults]`; options on the command line win over both.

Files convert independently: one failure does not stop the others. The exit code is the worst file's (3, then 2, then 1). JSON fields: `dir`, `manifest`, `out`, `converted`, `failed`, `skipped` (`input`, `reason`) and `files`, one `convert` report per file with its `input`.

## `export`

```sh
convert64 export <project.convert64 | dir> --asm kickass|acme|64tass|ca65|c|all[,…] --out <dir> [--data include-raw|include-picture|bytes] [--name N] [--max-clashes N] [convert options]
```

Solves the project, or every `.convert64` in the directory, and writes for each target the source `convert64-formats` emits ([file-formats.md](file-formats.md), "Assembler source we emit"): `<name>.asm` (KickAssembler), `<name>.a` (ACME), `<name>.tas` (64tass), `<name>.s` (ca65) or `<name>.h` (C header), with the binaries it includes. `--data include-raw` (the default) includes the raw set's `.bin` files, `include-picture` the `.kla`/`.art` file, `bytes` writes the data into the source. `<name>` is the project's file name unless `--name` gives one (a single project only). Options override the projects' settings, as in `convert`.

One command exports a whole directory to every assembler: `convert64 export art --asm all --out build`. The exit code is the worst project's. JSON fields: `input`, `assemblers`, `projects` (one report per project: `input`, `mode`, `sources` (`assembler`, `path`), the solve fields of `convert`, `files_written`).

## `apply`

```sh
convert64 apply <project.convert64> [--commands steps.json] [--command '<json>']… [-o out.convert64 | --in-place] [--embed | --link]
```

Runs commands from the core's registry on a project, exactly as the app runs them (ADR-005), and saves it. Every registry command is available here: masks (`mask_*`, `layer_via_mask`), cell steering (`cell_*`), layers (`layer_add`, `layer_remove`, `layer_reorder`, `layer_set_visible`, `layer_set_opacity`, `layer_set_blend`, `layer_set_transform`, `layer_rotate`, `layer_nudge`, `layer_rename`, `layer_duplicate`), `adjustments_set`, `recipe_set`, and the document's settings (`mode_set`, `palette_set`, `background_pin`, `solve_set`, `snap_set`). `layer_set_transform` and `layer_rotate` round the transform they write to the project's `snap` (ADR-011), or to their own `snap` argument when given; `layer_nudge` moves by exactly what it is given. The `*_set` commands change the fields they name and keep the rest (`null` clears an optional one, such as `solve_set`'s `hysteresis`). This is the CLI's half of GUI/CLI parity: `crates/convert64-cli/tests/parity.rs` fails if a registry command cannot run through it or through the MCP server, or gives a different project than the core in-process.

Commands are `{"command": "<name>", …arguments}` objects. `--commands FILE` holds an array of them (paths inside, such as `mask_load`'s, are relative to the file); each `--command` adds one (paths relative to the working directory); the file's run first. Without `-o` or `--in-place` nothing is written and the commands are only checked. A failing command names itself and its index, and nothing is written (exit 2).

JSON fields: `file`, `commands_applied` (their names), `layers`.

## `commands`

```sh
convert64 commands [--json]
```

Lists the registry: each command's name, summary and arguments (`?` marks optional ones). With `--json`, `commands` holds each one's `name`, `summary`, `schema` (JSON Schema of its arguments, the same the MCP server gives its tools) and `example` (a working command for a project of at least two layers whose layer 0 is at least 32 × 32 pixels, with a greyscale `mask.png` of layer 0's size and a `photo.png` beside it).

## `recipe apply`

```sh
convert64 recipe apply <recipe.json | project.convert64> [--save out.convert64 [--embed | --link]] [--mask LAYER=PATH]… [--composite PATH] [--attributes PATH] [--report]
```

Loads a project or a version-1 recipe, runs its commands, and writes what is asked: the project (`--save`), a layer's evaluated mask, the composite or the attribute map as PNGs, and with `--report` the solve's clash summary (exit 1 if the solve were illegal). JSON fields: `file`, `layers`, `commands_applied`, and with `--report` the solve fields of `convert`.

## `recipe commands`

The same as `commands`.
