# geometrize-rs

Turn a photo into a compact SVG of semi-transparent triangles.

This is a trimmed Rust port of [fogleman/primitive](https://github.com/fogleman/primitive), the algorithm behind [geometrize](https://github.com/Tw1ddle/geometrize). It's built for one job: making [marabyte.com](https://marabyte.com) cover images. It keeps primitive's core algorithm, adds a stop condition on output size, and is meant to run both as a CLI and in the browser through WebAssembly.

## How it works

Each step adds the one shape that most improves the picture:

1. Try many random triangles (1000 by default).
2. Take the best one and refine it by hill climbing: nudge a vertex, and keep the change only if the picture gets closer to the photo.
3. Do this 16 times independently and keep the overall winner.
4. Blend it onto the canvas in the colour that best matches the photo underneath it.

The process repeats until it reaches a number of shapes, a maximum SVG size, or whichever of the two comes first.

## Status

| Part | Crate | State |
| --- | --- | --- |
| Core algorithm | `crates/core` (`geometrize-core`) | done |
| CLI | `crates/cli` (`geometrize`) | planned |
| WebAssembly bindings | `crates/wasm` | planned |
| Preview site | `web/` (Astro) | planned |

## Using the core crate

`geometrize-core` has no dependencies. It takes RGBA bytes that are already resized to the working size (around 256 px on the longest side), so the caller handles decoding.

```rust
use geometrize_core::{Limit, Model, Options};

let options = Options {
    limit: Limit::Both { shapes: 500, bytes: 30_000 },
    opacity: 0.5,
    seed: 42,
    ..Options::default()
};
let mut model = Model::new(&rgba, width, height, options)?;
while let Some(step) = model.step() {
    eprintln!("score {:.4}, {} bytes", step.score, step.bytes);
}
std::fs::write("cover.svg", model.svg())?;
```

Enable the `parallel` feature to run the 16 searches of each step on [rayon](https://github.com/rayon-rs/rayon)'s thread pool.

The same seed always gives a byte-identical SVG, with or without `parallel` and whatever the thread count. Native and WebAssembly builds may still differ very occasionally, because `ln`, `cos` and `acos` come from different maths libraries on the two targets.

The output is a single line, in the same style as the SVGO-optimised covers:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 802"><path fill="#667692" d="M0 0h1200v802H0z"/><path fill="#ffe818" fill-opacity=".5" d="M405 227 551 874 846 260z"/>…</svg>
```

## What was trimmed from primitive

Kept from primitive:
- the triangle shape and the scanline rasteriser;
- the integer pixel maths (best colour, alpha blending, full and partial scores);
- the random search plus hill climbing;
- the average-colour background;
- SVG output.

Left out:
- **The other seven shape types.** The code is structured so they can come back: add a variant to `Shape`, a file with `random`/`mutate`/`rasterize`/`write_svg`, and one match arm per method in `crates/core/src/shape/mod.rs`.
- **Combo mode**, simulated annealing, the heatmap, `-rep`, and alpha mutation (`-a 0`).
- **PNG, JPG and GIF output**, frame sequences, and multiple `-n` configurations.
- **The `gg` and `freetype` dependencies.**

New in this version:
- a **maximum SVG size** stop condition;
- a configurable opacity, written as `fill-opacity`;
- per-search random seeds, so threading can't change the output;
- a compact `<path>` output format.

## Development

```sh
cargo test --workspace
cargo test -p geometrize-core --features parallel
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# Writes /tmp/core.svg (300 shapes on a synthetic image) for a visual check
cargo test -p geometrize-core --release --features parallel -- --ignored
```

## Licence

MIT. Based on [primitive](https://github.com/fogleman/primitive), © 2016 Michael Fogleman, also MIT. See [LICENSE](LICENSE).
