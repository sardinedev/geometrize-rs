# Test fixtures

Real images used by the tests and benchmarks.

## `big_ben.jpg`

- **What:** Big Ben and the Houses of Parliament at sunset, from across the Thames.
- **Photographer:** Luke Stackpoole ([@withluke](https://unsplash.com/@withluke)).
- **Source:** <https://unsplash.com/photos/big-ben-tower-mOEqOtmuPG8>
- **Licence:** [Unsplash License](https://unsplash.com/license). It is **not** covered by this repository's MIT licence.
- **Changes:** shrunk from 930×1162 to 409×512 pixels and re-saved as JPEG (quality 85), to keep the repository small. The tools shrink photos to 256 pixels before searching anyway, so this loses nothing.

Used by:
- `crates/cli/tests/cli.rs`, which checks that JPEG decoding, the size budget and quality hold on a real photo;
- `crates/core/benches/model.rs`, which benchmarks the core on a real photo next to the synthetic one.
