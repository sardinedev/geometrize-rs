//! The shapes the optimiser can place.
//!
//! This module is the one place a new shape type gets wired in. To add one:
//! 1. add a variant to [`ShapeKind`] and to [`Shape`], and add it to
//!    [`ShapeKind::ALL`] by hand: that list is the one place the compiler
//!    can't check (a missing entry means the name can't be parsed);
//! 2. write a file next to `triangle.rs` with `random`, `mutate`,
//!    `rasterize` and `write_svg`;
//! 3. add a match arm to each method below.
//!
//! The one rule a new shape's `rasterize` must follow: **never cover the same
//! pixel twice**. The partial error update in
//! [`crate::pixels::difference_after_drawing`] removes each covered pixel's
//! old error once, so a pixel visited twice would corrupt the total. Filling
//! one span per row, as `raster::rasterize_triangle` does, guarantees it.
//!
//! The search, the model, the colour maths and the SVG writer only ever see
//! [`Shape`], so nothing else changes. An enum (rather than a trait object)
//! keeps the hot loop free of dynamic dispatch, and lets shapes be `Copy` so a
//! failed mutation is undone by simply restoring the old value.

mod triangle;

pub use triangle::Triangle;

use std::fmt;
use std::str::FromStr;

use crate::raster::Span;
use crate::rng::Rng;

/// The kinds of shape the optimiser can be asked to use.
///
/// The numeric value is a stable code (the WASM build sends it to the browser
/// so the canvas renderer knows how to draw each shape). The name, as used on
/// the command line, comes from [`Display`](fmt::Display) and is read back by
/// [`FromStr`]: `"triangle".parse::<ShapeKind>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShapeKind {
    /// A triangle with all three angles wider than 15°.
    Triangle = 0,
}

impl ShapeKind {
    /// Every supported kind.
    pub const ALL: &'static [ShapeKind] = &[ShapeKind::Triangle];

    /// The lowercase name, e.g. `"triangle"`.
    fn name(self) -> &'static str {
        match self {
            ShapeKind::Triangle => "triangle",
        }
    }
}

impl fmt::Display for ShapeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ShapeKind {
    type Err = UnknownShapeKind;

    /// Reads a kind from its lowercase name, e.g. `"triangle"`.
    fn from_str(name: &str) -> Result<ShapeKind, UnknownShapeKind> {
        ShapeKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| UnknownShapeKind {
                name: name.to_string(),
            })
    }
}

/// A name that isn't a [`ShapeKind`]. Its message lists the valid names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownShapeKind {
    /// The name that was given.
    name: String,
}

impl fmt::Display for UnknownShapeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = ShapeKind::ALL.iter().map(|kind| kind.name()).collect();
        write!(
            f,
            "unknown shape '{}' (available: {})",
            self.name,
            names.join(", ")
        )
    }
}

impl std::error::Error for UnknownShapeKind {}

/// One shape, positioned in working-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A triangle.
    Triangle(Triangle),
}

impl Shape {
    /// The kind of this shape.
    #[must_use]
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

    /// Replaces `spans` with the pixels this shape covers, clipped to a
    /// `width × height` image. No pixel may appear twice (see the module
    /// docs).
    pub(crate) fn rasterize(&self, width: usize, height: usize, spans: &mut Vec<Span>) {
        match self {
            Shape::Triangle(triangle) => triangle.rasterize(width, height, spans),
        }
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
    fn kind_codes_are_stable() {
        // The browser preview draws shapes by these codes: changing one
        // breaks it, so a new kind must take the next free number.
        assert_eq!(ShapeKind::Triangle as u8, 0);
    }

    #[test]
    fn names_round_trip() {
        for &kind in ShapeKind::ALL {
            assert_eq!(kind.to_string().parse::<ShapeKind>(), Ok(kind));
        }
    }

    #[test]
    fn unknown_names_list_the_valid_ones() {
        let error = "hexagon".parse::<ShapeKind>().unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown shape 'hexagon' (available: triangle)"
        );
    }

    #[test]
    fn random_of_uses_the_listed_kind() {
        let mut rng = Rng::new(3);
        let shape = Shape::random_of(&[ShapeKind::Triangle], 50, 40, &mut rng);
        assert_eq!(shape.kind(), ShapeKind::Triangle);
    }
}
