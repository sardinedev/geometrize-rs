//! The shapes the optimiser can place.
//!
//! This module is the one place a new shape type gets wired in. To add one:
//! 1. add a variant to [`ShapeKind`] and to [`Shape`];
//! 2. write a file next to `triangle.rs` with `random`, `mutate`,
//!    `rasterize` and `write_svg`;
//! 3. add a match arm to each method below.
//!
//! The one rule a new shape's `rasterize` must follow: **never cover the same
//! pixel twice** (no overlapping scanlines). The partial error update in
//! [`crate::pixels::difference_partial`] removes each covered pixel's old
//! error once, so a pixel visited twice would corrupt the total. Give the new
//! shape a test like `rows_are_unique_and_sorted_within_each_half` in `raster.rs`.
//!
//! The search, the model, the colour maths and the SVG writer only ever see
//! [`Shape`], so nothing else changes. An enum (rather than a trait object)
//! keeps the hot loop free of dynamic dispatch, and lets shapes be `Copy` so a
//! failed mutation is undone by simply restoring the old value.

mod triangle;

pub use triangle::Triangle;

use crate::raster::{Scanline, crop_scanlines};
use crate::rng::Rng;

/// The kinds of shape the optimiser can be asked to use.
///
/// The numeric value is a stable code (the WASM build sends it to the browser
/// so the canvas renderer knows how to draw each shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShapeKind {
    /// A triangle with all three angles above 15°.
    Triangle = 0,
}

impl ShapeKind {
    /// Every supported kind.
    pub const ALL: &'static [ShapeKind] = &[ShapeKind::Triangle];

    /// The lowercase name used on the command line, e.g. `"triangle"`.
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Triangle => "triangle",
        }
    }

    /// Looks a kind up by its [`name`](ShapeKind::name). Returns `None` for an
    /// unknown name.
    pub fn from_name(name: &str) -> Option<ShapeKind> {
        ShapeKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.name() == name)
    }
}

/// One shape, positioned in working-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A triangle.
    Triangle(Triangle),
}

impl Shape {
    /// The kind of this shape.
    pub fn kind(&self) -> ShapeKind {
        match self {
            Shape::Triangle(_) => ShapeKind::Triangle,
        }
    }

    /// Creates a random shape of the given kind on a `width × height` image.
    pub(crate) fn random(kind: ShapeKind, width: i32, height: i32, rng: &mut Rng) -> Shape {
        match kind {
            ShapeKind::Triangle => Shape::Triangle(Triangle::random(width, height, rng)),
        }
    }

    /// Creates a random shape of a kind picked at random from `kinds`.
    ///
    /// The pick always uses the random generator, even when there is only one
    /// kind, so adding more kinds later doesn't change the output of runs that
    /// only use one.
    pub(crate) fn random_of(kinds: &[ShapeKind], width: i32, height: i32, rng: &mut Rng) -> Shape {
        let kind = kinds[rng.below(kinds.len() as u32) as usize];
        Shape::random(kind, width, height, rng)
    }

    /// Makes one small random change to the shape, keeping it valid.
    pub(crate) fn mutate(&mut self, width: i32, height: i32, rng: &mut Rng) {
        match self {
            Shape::Triangle(triangle) => triangle.mutate(width, height, rng),
        }
    }

    /// Replaces `lines` with the scanlines this shape covers, cropped to a
    /// `width × height` image. No pixel may appear in more than one line (see
    /// the module docs).
    pub(crate) fn rasterize(&self, width: i32, height: i32, lines: &mut Vec<Scanline>) {
        lines.clear();
        match self {
            Shape::Triangle(triangle) => triangle.rasterize(lines),
        }
        crop_scanlines(lines, width, height);
    }

    /// Appends the shape's SVG element to `out`.
    ///
    /// `scale` converts working-image pixels to output units. `attrs` holds the
    /// element's presentation attributes (`fill="…" fill-opacity="…"`), which
    /// the shape writes first.
    pub(crate) fn write_svg(&self, out: &mut String, scale: f64, attrs: &str) {
        match self {
            Shape::Triangle(triangle) => triangle.write_svg(out, scale, attrs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for &kind in ShapeKind::ALL {
            assert_eq!(ShapeKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ShapeKind::from_name("hexagon"), None);
    }

    #[test]
    fn random_of_uses_the_listed_kind() {
        let mut rng = Rng::new(3);
        let shape = Shape::random_of(&[ShapeKind::Triangle], 50, 40, &mut rng);
        assert_eq!(shape.kind(), ShapeKind::Triangle);
    }
}
