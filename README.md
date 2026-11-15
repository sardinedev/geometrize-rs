# geometrize-rs

[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/sardinedev/geometrize-rs?utm_source=badge)

Turn a photo into a small, stylised SVG built from a few hundred semi-transparent triangles, which makes good cover images and placeholders. You set the **maximum file size**, and the same seed always gives the **exact same file**. It's a Rust port of [fogleman/primitive](https://github.com/fogleman/primitive), trimmed down to triangles and SVG, with a core that has no dependencies so it can run on the command line and in the browser.

- **Size budget.** Stop at a number of shapes, a maximum SVG size in bytes, or whichever comes first. The SVG never goes over the byte limit.
- **Reproducible.** The same image, options and seed give a byte-identical SVG, however many threads the search runs on, and on every platform: native and WebAssembly builds produce the same bytes.
- **Compact output.** One-line, SVGO-style markup: `#rgb` colours where possible, no space before negative numbers, and no wrapper groups or transforms.
- **No dependencies.** The core crate uses only the standard library. The optional `parallel` feature adds [rayon](https://github.com/rayon-rs/rayon).
- **Exact maths.** Colour fitting, blending and scoring are integer maths with exact rounding. Everything else uses only the float operations IEEE-754 defines exactly (`+ − × ÷`, `sqrt`, rounding), never maths-library functions like `ln` or `cos`, so results can't depend on the platform.

```rust
use geometrize_core::{Limit, Model, Options};

let options = Options { limit: Limit::Bytes(30_000), ..Options::default() };
let mut model = Model::new(&rgba, width, height, options)?; // RGBA pixels, ~256 px wide
for _ in model.steps() {}                                   // add triangles until the budget is used
std::fs::write("cover.svg", model.svg())?;
// cover.svg, all on one line:
// <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 802"><path fill="#6d93c0" d="M0 0h1200v802H0z"/>
//   <path fill="#000039" fill-opacity=".5" d="M1141 874 626 382 1216-73z"/>…</svg>
```

> **Status:** the core library and the `geometrize` command-line tool work and are tested. The WebAssembly build and a browser preview page are next. Nothing is published on crates.io yet.

---

- [Command line](#command-line)
- [Getting started with the library](#getting-started-with-the-library)
- [Options](#options)
- [How it works](#how-it-works)
- [Roadmap](#roadmap)
- [What was trimmed from primitive](#what-was-trimmed-from-primitive)
- [Development](#development)
- [Licence](#licence)

---

## Command line

**1. Install the `geometrize` command** (from a clone of this repository):

```sh
cargo install --path crates/cli
```

**2. Turn a photo into a cover:**

```sh
geometrize -i photo.jpg -o cover.svg --max-size 30k
# seed 42 (pass --seed 42 to reproduce)
# done: 430 shapes, score 0.0470, 29980 bytes in … s -> cover.svg
```

It reads JPEG and PNG, turns phone photos upright, blends transparent pixels onto the background, and shrinks the photo to 256 pixels on its long side before searching. Without `--seed` it picks one and prints it, so you can make the same file again. Progress goes to stderr, and `-o -` writes the SVG to stdout.

| Flag | Default | What it does |
| --- | --- | --- |
| `-i, --input <file>` | required | JPEG or PNG photo to read |
| `-o, --output <file>` | required | SVG file to write, or `-` for stdout |
| `-n, --shapes <n>` | 500 (see below) | Stop after n shapes |
| `--max-size <size>` | none | Keep the SVG at or under this size: `30000`, `30k`, `1.5m` (k = 1000 bytes) |
| `--opacity <0.01-1>` | 0.5 | Opacity of every shape |
| `--shape <kind>` | `triangle` | Shape to use; repeat for several (only `triangle` for now) |
| `--width <px>` | 1200 | Width of the SVG's viewBox |
| `--bg <rrggbb>` | photo's average | Background colour: `ff8800` or `f80` (quote a leading `#` in a shell: `'#f80'`). Transparent pixels are blended onto it, or onto white |
| `--seed <n>` | from the clock | Random seed; the same seed gives the same SVG |
| `--work-size <px>` | 256 | Long side of the photo while searching |
| `--candidates`, `--age`, `--climbs` | 1000, 100, 16 | Search settings (see [Options](#options)) |
| `-j, --threads <n>` | all cores | Threads to use |
| `-q, --quiet` | off | Print nothing except errors |

With neither `-n` nor `--max-size`, it stops at 500 shapes. With only `--max-size`, there's no shape limit. With both, it stops at whichever comes first. `geometrize --help` prints the same list.

---

## Getting started with the library

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
    for step in model.steps() {
        eprintln!("shape {} score {:.4} {} bytes", step.shapes, step.score, step.bytes);
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

> The same seed also gives the same SVG on every platform, because no step uses a maths-library function (`ln`, `cos`, `acos`…) whose result could differ between them. A golden test pins the exact output of a small run, and CI checks it both natively and on WebAssembly.

---

## Roadmap

| Part | Location | State |
| --- | --- | --- |
| Core library | `crates/core` (`geometrize-core`) | done |
| Command-line tool | `crates/cli` (`geometrize`) | done |
| WebAssembly bindings | `crates/wasm` | next |
| Browser preview page | `web/` (Astro) | planned |

---

## What was trimmed from primitive

**Kept:** primitive's approach. That means the triangle shape, the random search plus hill climbing, fitting each shape's colour to the photo underneath it, scoring only the pixels a shape covers, the average-colour background, and SVG output. The code itself is idiomatic Rust rather than a line-for-line port, so results differ from primitive's for the same settings.

**Left out:**
- **The other seven shape types.** The `Shape` enum is built so they can come back. Adding one takes a variant, one file with `random`, `mutate`, `rasterize` and `write_svg`, and a match arm per method in `crates/core/src/shape/mod.rs`.
- **Combo mode**, simulated annealing, the heatmap, `-rep`, and alpha mutation (`-a 0`).
- **PNG, JPG and GIF output**, frame sequences, and multiple `-n` configurations.
- **The `gg` and `freetype` dependencies.**

**New here:**
- a maximum-size stop condition;
- per-search seeds, so threading can't change the result;
- the compact `<path>` output;
- exact integer error totals, so no rounding error builds up from step to step;
- plain 8-bit colour maths and a pixel-centre rasteriser, in place of primitive's 16-bit premultiplied maths (from Go's `image` package) and float-stepped scanlines. It's faster on the same search (see [docs/DECISIONS.md](docs/DECISIONS.md)) and identical on every platform.

---

## Development

The Rust version is pinned in `rust-toolchain.toml`, so rustup installs the right one automatically. Lints shared by all crates are in `[workspace.lints]` in `Cargo.toml`. Before reworking something for speed or style, check [docs/DECISIONS.md](docs/DECISIONS.md): it lists what was already tried or decided against, and why.

```sh
cargo test --workspace                              # core with `parallel`, plus the CLI
cargo test -p geometrize-core                       # core without `parallel` (the WebAssembly path)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# Benchmarks (divan, also run by CodSpeed in CI)
cargo bench -p geometrize-core

# Writes /tmp/core.svg (300 triangles on a synthetic image) for a visual check
cargo test -p geometrize-core --release --features parallel -- --ignored
```

---

## Licence

MIT. Based on [primitive](https://github.com/fogleman/primitive), © 2016 Michael Fogleman, also MIT. See [LICENSE](LICENSE).
