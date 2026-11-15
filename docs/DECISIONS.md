# Decisions

Design decisions, with the reason and any measurements: changes that were suggested (often by code review) and deliberately not made, and larger changes that were made on purpose. **Don't redo these without new evidence**; if something here changes, update the entry instead of deleting it.

Most entries also have a comment at the spot in the code, so a reader there sees the reason too.

---

## Measured and rejected

| Idea | Result | Why not |
| --- | --- | --- |
| Apply the EXIF rotation *after* shrinking the photo (`crates/cli/src/main.rs`, `load_image`) | Saves ~85 ms and one full-size copy (~86 MB) on a 28.8 MP photo with EXIF orientation 6 | Changes the output for rotated photos: `thumbnail` samples pixels differently depending on orientation. Not worth an output change for 3–4% of one run. |
| Set up each triangle edge once (slope per row) instead of dividing on every row in `raster::rasterize_triangle` | Mixed: −3% to +12% across benchmarks, within noise | No clear gain, and the precomputed slope rounds differently on some rows, which changed the Big Ben output. |
| Spread the 16 searches of a step more evenly over 10 cores | ~72% CPU use (16.2 s user against 2.25 s wall × 10) | Needs a different `climbs` or work splitting inside a search, which changes the output or the determinism design. A tuning question only. |

## Done, and why

- **The core no longer follows primitive's exact numerics** (branch `refactor/idiomatic-core`). The Go-style parts were replaced with plain Rust:
  - **colour maths:** 8-bit integer maths with exact rounding, instead of the 16-bit premultiplied form from Go's `image/color`;
  - **canvases:** RGB only, instead of RGBA with an alpha channel that was always 255;
  - **rasteriser:** a single pixel-centre rasteriser that clips as it goes, instead of float-stepping flat-top/flat-bottom halves plus a crop pass;
  - **angle test:** a dot product against `cos 15°` instead of `acos`;
  - **mutation steps:** an Irwin–Hall normal approximation instead of Box–Muller.

  **Why:** simpler code, and every float operation left is exact IEEE-754, so native and WebAssembly give the same bytes (a golden test pins this). The search is 15–35% faster in the benchmarks, and the full Big Ben cover takes 1.7 s instead of 2.4 s, at the same quality.

  **Cost:** every seed now gives a different SVG from before. Scores are also not comparable with older ones: they now average over 3 channels instead of 4 (the alpha channel only ever added zeros), which makes them √(4/3) ≈ 1.155 times higher for the same picture.
- **clippy's `pedantic` group is on**, except for the cast lints and `format_push_string` (see `Cargo.toml`). The cast lints flag every `as` in the pixel maths, which moves between `u8`, `i32`, `usize` and `f64` all the time. Each of those values is clamped or already known to be in range, and wrapping them in `try_from(..).expect(..)` would hurt readability. The rest of the group found real improvements, such as `#[must_use]` on getters, and now applies to new code too.

## Decided to leave as is

**Core (`crates/core`)**
- **No ziggurat (table-based) normal distribution in `Rng::normal`.** Not measured: `normal()` is a small part of a step next to rasterising and scoring, and the sum of 12 uniforms is short and exact on every platform. A lot of clever table code for a small gain.
- **The winner is recomputed in `Model::step`.** The search already knows the winning shape's colour and error, and its pixels get blended twice (once to predict, once to draw). It's one shape next to ~17,000 evaluations per step, and it keeps adding a shape self-contained. A `debug_assert_eq!` checks that both agree.
- **The SVG writer builds temporary strings** (`format!`, `to_string`) and measures each shape by writing it. A whole 500-shape cover is written in well under a millisecond.
- **A span buffer is allocated per search round** instead of being kept across steps: at most `climbs` small allocations per round.
- **`opacity_percent` and `alpha` are both stored.** `alpha` is derived once and needed on every evaluation.
- **8-bit alpha against the 2-decimal `fill-opacity`.** Up to ~0.2 percentage points of drift (50% is blended as 128/255 ≈ 50.2%). Writing the exact alpha would turn `.5` into `.502` on every shape, about 1 KB per cover.
- **A byte-limited run stops at the first shape that doesn't fit,** although a smaller one might. Less than one shape's length (roughly 40–60 bytes) is left by then.
- **`MAX_FAILED_SEARCHES` is a constant (4), not an option.** There's no useful reason to tune it.
- **Vertices are rounded to whole output units.** Half a unit is about a tenth of a working pixel at the default 1200 width.
- **`candidates`, `climbs` and `out_width` are not `NonZeroU32`.** Every caller would have to write `NonZeroU32::new(50).unwrap()`; `Error::Zero` does the same job.
- **No separate `Opacity` type.** It would only wrap the two fields above.
- **`Model::rgb()` returns RGB, not RGBA.** It replaced the RGBA `pixels()`, which nothing called. The planned WASM preview draws each shape on the browser canvas instead of copying pixels. If a pixel copy is ever needed, add a `write_rgba(&mut [u8])` then.
- **Width and height are `i32` for shape creation and mutation but `usize` for rasterising.** Shapes need signed coordinates (vertices can sit in the margin outside the image), while clipping and indexing need unsigned sizes. Unifying them would only move the casts around.
- **No `Model::limit()` accessor.** The CLI copies the limit before building the model, and the WASM form already knows it.
- **The working size (256) and the rule for shrinking stay in the CLI,** not in the core. The browser resamples differently anyway, so sharing them wouldn't make CLI and web output match. Revisit with the WASM crate.
- **No pinned `const` struct sizes, `#[cold]` paths, custom allocators, PGO or BOLT.** These are tools for much larger programs; nothing here has asked for them.

**CLI (`crates/cli`)**
- **`Error::Zero(&'static str)` is turned into flag names by matching the core's strings,** and `is_usage` and `explain_core_error` each match every `Error` variant. A typed `Setting` enum was offered and declined for now.
- **The seed is stored twice in `Args`** (`Args::seed` and a placeholder in `options.seed`, filled in by `run`). The alternatives aren't clearer.
- **`-n 0` is accepted** and writes a background-only SVG.
- **`parse_size`'s upper bound** lets one value just past `usize::MAX` through, which then saturates. Harmless.
- **The progress line is built on every step,** even when stderr isn't a terminal and only every 50th line is printed.
- **The `args.rs` module doc** says only formats are checked there, though a few CLI-only ranges are too.

**WebAssembly bindings (`crates/wasm`)**
- **Settings are a `SessionOptions` class, not a plain JS object.** Reading an object would need `serde` and `serde-wasm-bindgen` (or `js-sys` and hand-written property lookups). The class needs no extra dependency, and the generated `.d.ts` types every field.
- **`Session::step` returns an `Int32Array`,** not the `Float32Array` of the original plan. Every value (kind, coordinates, colour) is a whole number.
- **The seed is a `BigInt` (`u64`),** so any seed the CLI prints can be pasted into the page unchanged; a JS `number` would lose seeds above 2⁵³.
- **No background or shape-kind settings yet.** The background is always the photo's average and the only shape is the triangle. Add them when the page needs them.
- **No `wasm-bindgen-test` in CI.** The bindings only translate types: their unit tests run natively, CI lints them for `wasm32-unknown-unknown`, and the core's own tests (golden SVG included) run on WebAssembly. Running the built module in Node matched the golden SVG byte for byte; revisit once the preview page exists and can be tested end to end.

**Tests**
- **Unit tests stay next to the code** in `#[cfg(test)]` modules, rather than all living in `tests/`. Easier to read alongside what they test.
- **A shared "run until done" benchmark helper isn't worth it.** The one-line `while model.step().is_some() {}` reads better inline.
