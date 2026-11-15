# Decisions

Changes that were suggested (often by code review) and deliberately not made, with the reason and any measurements. **Don't redo these without new evidence**; if something here changes, update the entry instead of deleting it.

Most entries also have a comment at the spot in the code, so a reader there sees the reason too.

---

## Measured and rejected

| Idea | Result | Why not |
| --- | --- | --- |
| Apply the EXIF rotation *after* shrinking the photo (`crates/cli/src/main.rs`, `load_image`) | Saves ~85 ms and one full-size copy (~86 MB) on a 28.8 MP photo with EXIF orientation 6 | Changes the output for rotated photos: `thumbnail` samples pixels differently depending on orientation. Not worth an output change for 3–4% of one run. |
| Compare cosines against `cos(15°)` instead of using `acos` in `Triangle::is_valid` | `acos`, `hypot` and mutation are ~5–7% of samples | Small gain, and changing float edge cases would change the deterministic output. |
| clippy's whole `pedantic` group | 111 warnings, most of them cast lints (`cast_possible_truncation` and similar) | They flag the deliberate `as` casts in the integer maths ported from primitive. Noise, not findings. A small chosen set lives in `[workspace.lints]` instead. |
| Spread the 16 searches of a step more evenly over 10 cores | ~72% CPU use (16.2 s user against 2.25 s wall × 10) | Needs a different `climbs` or work splitting inside a search, which changes the output or the determinism design. A tuning question only. |

## Decided to leave as is

**Core (`crates/core`)**
- **The winner is recomputed in `Model::step`.** The search already knows the winning shape's colour and error, and its pixels get blended twice (once to predict, once to draw). It's one shape next to ~17,000 evaluations per step, and it keeps adding a shape self-contained. A `debug_assert_eq!` checks that both agree.
- **The SVG writer builds temporary strings** (`format!`, `to_string`) and measures each shape by writing it. A whole 500-shape cover is written in well under a millisecond.
- **A scanline buffer is allocated per search round** instead of being kept across steps: at most `climbs` small allocations per round.
- **`opacity_percent` and `alpha` are both stored.** `alpha` is derived once and needed on every evaluation.
- **8-bit alpha against the 2-decimal `fill-opacity`.** Up to ~0.2 percentage points of drift (50% is blended as 128/255 ≈ 50.2%). Writing the exact alpha would turn `.5` into `.502` on every shape, about 1 KB per cover.
- **A byte-limited run stops at the first shape that doesn't fit,** although a smaller one might. Less than one shape's length (roughly 40–60 bytes) is left by then.
- **`MAX_FAILED_SEARCHES` is a constant (4), not an option.** There's no useful reason to tune it.
- **Vertices are rounded to whole output units.** Half a unit is about a tenth of a working pixel at the default 1200 width.
- **`candidates`, `climbs` and `out_width` are not `NonZeroU32`.** Every caller would have to write `NonZeroU32::new(50).unwrap()`; `Error::Zero` does the same job.
- **No separate `Opacity` type.** It would only wrap the two fields above.
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

**Tests**
- **Unit tests stay next to the code** in `#[cfg(test)]` modules, rather than all living in `tests/`. Easier to read alongside what they test.
- **A shared "run until done" benchmark helper isn't worth it.** The one-line `while model.step().is_some() {}` reads better inline.
