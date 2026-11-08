# geometrize-rs

Turn a photo into a small, stylised SVG built from a few hundred semi-transparent triangles, which makes good cover images and placeholders. You set the **maximum file size**, and the same seed always gives the **exact same file**. It's a Rust port of [fogleman/primitive](https://github.com/fogleman/primitive), trimmed down to triangles and SVG, with a core that has no dependencies so it can run on the command line and in the browser.

- **Size budget.** Stop at a number of shapes, a maximum SVG size in bytes, or whichever comes first. The SVG never goes over the byte limit.
- **Reproducible.** The same image, options and seed give a byte-identical SVG, however many threads the search runs on.
- **Compact output.** One-line, SVGO-style markup: `#rgb` colours where possible, no space before negative numbers, and no wrapper groups or transforms.
- **No dependencies.** The core crate uses only the standard library. The optional `parallel` feature adds [rayon](https://github.com/rayon-rs/rayon).
- **primitive's maths.** The colour fitting, alpha blending and scoring are ported integer for integer from primitive's Go code.

```rust
use geometrize_core::{Limit, Model, Options};

let options = Options { limit: Limit::Bytes(30_000), ..Options::default() };
let mut model = Model::new(&rgba, width, height, options)?; // RGBA pixels, ~256 px wide
while model.step().is_some() {}                             // add triangles until the budget is used
std::fs::write("cover.svg", model.svg())?;
// cover.svg, all on one line:
// <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 802"><path fill="#6c92bf" d="M0 0h1200v802H0z"/>
//   <path fill="#00003b" fill-opacity=".5" d="M645 391 1198 874 1198-73z"/>…</svg>
```

> **Status:** the core library works and is tested. The `geometrize` command-line tool, the WebAssembly build and a browser preview page are next. Nothing is published on crates.io yet.

---

- [Getting started](#getting-started)
- [Options](#options)
- [How it works](#how-it-works)
- [Roadmap](#roadmap)
- [What was trimmed from primitive](#what-was-trimmed-from-primitive)
- [Development](#development)
- [Licence](#licence)

---

## Getting started

The core crate doesn't decode or resize images, so pair it with an image library. These steps use [`image`](https://crates.io/crates/image).

**1. Create a project and add the dependencies:**

```sh
cargo new cover-demo
cd cover-demo
cargo add geometrize-core --git https://github.com/sardinedev/geometrize-rs --features parallel
cargo add image
```

**2. Replace `src/main.rs` with:**

```rust
use geometrize_core::{Limit, Model, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Decode the photo and shrink it to the working size (~256 px on the long side).
    let photo = image::open("photo.jpg")?.thumbnail(256, 256).to_rgba8();
    let (width, height) = photo.dimensions();

    let options = Options {
        limit: Limit::Both { shapes: 500, bytes: 30_000 },
        seed: 42,
        ..Options::default()
    };
    let mut model = Model::new(photo.as_raw(), width, height, options)?;
    while let Some(step) = model.step() {
        eprintln!("shape {} score {:.4} {} bytes", model.shapes().len(), step.score, step.bytes);
    }

    std::fs::write("cover.svg", model.svg())?;
    Ok(())
}
```

**3. Put a photo next to `Cargo.toml` as `photo.jpg` and run it in release mode:**

```sh
cargo run --release
```

> **Use `--release`.** Each step tries thousands of shapes, and a debug build is many times slower.

The run stops after 500 triangles, or earlier if the next triangle would take `cover.svg` past 30,000 bytes.

---

## Options

Start from `Options::default()` and change only what you need.

| Field | Default | What it does |
| --- | --- | --- |
| `limit` | `Limit::Shapes(500)` | When to stop: `Shapes(n)`, `Bytes(n)`, or `Both { shapes, bytes }`. `Limit::from_parts(Option, Option)` builds one from optional values. |
| `opacity` | `0.5` | Opacity of every shape, from 0.01 to 1, rounded to two decimals. At 1, `fill-opacity` is left out. |
| `out_width` | `1200` | Width of the SVG's `viewBox`. The height follows the image's aspect ratio. |
| `seed` | `0` | Seed for the random search. The same seed gives the same SVG. |
| `bg` | `None` | Background colour. `None` uses the image's average colour. |
| `shapes` | `[Triangle]` | Shape kinds to pick from. Only triangles exist for now. |
| `candidates` | `1000` | Random shapes tried per search. |
| `age` | `100` | Failed mutations in a row before a hill climb gives up. |
| `climbs` | `16` | Independent searches per step. The best one wins. |

Lower `candidates`, `age` and `climbs` give faster, rougher results. `Model::new` returns an `Error` for invalid options, such as a zero setting, an opacity out of range, or a byte limit smaller than an empty SVG.

---

## How it works

Each step adds the one triangle that most improves the picture:

1. **Random search.** Try `candidates` random triangles and keep the closest match.
2. **Hill climb.** Nudge one vertex at a time, and keep each change only if the picture gets closer to the photo.
3. **Best of many.** Run `climbs` such searches independently, in parallel with the `parallel` feature, and keep the overall winner.
4. **Best colour.** Blend the winner onto the canvas in the colour that best matches the photo underneath it.

Every search has its own random generator, seeded from `(seed, step, search)`, and ties go to the lowest search number. That's why the thread count can't change the output.

> Determinism holds across runs and thread counts, not between native and WebAssembly builds. `ln`, `cos` and `acos` come from different maths libraries on those targets, so a seed can very occasionally give a slightly different SVG in the browser.

---

## Roadmap

| Part | Location | State |
| --- | --- | --- |
| Core library | `crates/core` (`geometrize-core`) | done |
| Command-line tool | `crates/cli` (`geometrize`) | next |
| WebAssembly bindings | `crates/wasm` | planned |
| Browser preview page | `web/` (Astro) | planned |

---

## What was trimmed from primitive

**Kept:** the triangle shape, the scanline rasteriser, the integer pixel maths, the random search plus hill climbing, the average-colour background, and SVG output.

**Left out:**
- **The other seven shape types.** The `Shape` enum is built so they can come back. Adding one takes a variant, one file with `random`, `mutate`, `rasterize` and `write_svg`, and a match arm per method in `crates/core/src/shape/mod.rs`.
- **Combo mode**, simulated annealing, the heatmap, `-rep`, and alpha mutation (`-a 0`).
- **PNG, JPG and GIF output**, frame sequences, and multiple `-n` configurations.
- **The `gg` and `freetype` dependencies.**

**New here:**
- a maximum-size stop condition;
- per-search seeds, so threading can't change the result;
- the compact `<path>` output;
- exact integer error totals, so no rounding error builds up from step to step.

---

## Development

```sh
cargo test --workspace
cargo test -p geometrize-core --features parallel
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# Writes /tmp/core.svg (300 triangles on a synthetic image) for a visual check
cargo test -p geometrize-core --release --features parallel -- --ignored
```

---

## Licence

MIT. Based on [primitive](https://github.com/fogleman/primitive) by Michael Fogleman, also MIT. See [LICENSE](LICENSE).
