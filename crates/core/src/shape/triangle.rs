//! Triangles.
//!
//! Based on primitive's `triangle.go`: a new triangle starts small around a
//! random point, and mutation moves one vertex by a normally distributed
//! amount, retrying until all three angles are wider than 15°. The SVG
//! output is a compact `<path>`.

use crate::raster::{Point, Span, rasterize_triangle};
use crate::rng::Rng;
use crate::svg;

/// How far (in pixels) a vertex may sit outside the image. Letting vertices
/// overhang the edges lets triangles cover the borders with clean edges.
const MARGIN: i32 = 16;

/// Standard deviation (in pixels) of a vertex move during mutation.
const MUTATION_STEP: f64 = 16.0;

/// `cos 15°`. Every angle must be wider than 15°, so its cosine must be
/// smaller than this. (Written out because `f64::cos` isn't available in a
/// `const`; a test checks the value.)
const COS_MIN_ANGLE: f64 = 0.965_925_826_289_068_3;

/// A triangle in working-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    /// The three corners, in no particular order.
    pub vertices: [Point; 3],
}

impl Triangle {
    /// Creates a small random triangle somewhere on a `width × height` image:
    /// a random point plus two corners within 15 pixels of it, then one
    /// mutation (which also makes it valid).
    pub(crate) fn random(width: i32, height: i32, rng: &mut Rng) -> Triangle {
        let first = Point::new(
            rng.below(width as u32) as i32,
            rng.below(height as u32) as i32,
        );
        let mut nearby = || {
            let mut offset = || rng.below(31) as i32 - 15;
            Point::new(first.x + offset(), first.y + offset())
        };
        let mut triangle = Triangle {
            vertices: [first, nearby(), nearby()],
        };
        triangle.mutate(width, height, rng);
        triangle
    }

    /// Moves one random corner by a normally distributed amount (clamped to
    /// the image plus [`MARGIN`]), repeating until the triangle is valid.
    pub(crate) fn mutate(&mut self, width: i32, height: i32, rng: &mut Rng) {
        loop {
            let corner = &mut self.vertices[rng.below(3) as usize];
            corner.x = (corner.x + random_step(rng)).clamp(-MARGIN, width - 1 + MARGIN);
            corner.y = (corner.y + random_step(rng)).clamp(-MARGIN, height - 1 + MARGIN);
            if self.is_valid() {
                return;
            }
        }
    }

    /// Whether every angle of the triangle is wider than 15°. Thin slivers,
    /// straight lines and triangles with two corners in the same place are
    /// all invalid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let [a, b, c] = self.vertices;
        is_wide_angle(a, b, c) && is_wide_angle(b, c, a) && is_wide_angle(c, a, b)
    }

    /// Replaces `spans` with the pixels the triangle covers, clipped to a
    /// `width × height` image.
    pub(crate) fn rasterize(&self, width: usize, height: usize, spans: &mut Vec<Span>) {
        rasterize_triangle(self.vertices, width, height, spans);
    }

    /// Appends `<path {attrs} d="Mx1 y1 x2 y2 x3 y3z"/>` with each coordinate
    /// mapped to output units.
    pub(crate) fn write_svg(&self, out: &mut String, scale: f64, attrs: &str) {
        let coordinates = self.vertices.map(|corner| {
            [
                svg::scale_coordinate(corner.x, scale),
                svg::scale_coordinate(corner.y, scale),
            ]
        });
        out.push_str("<path ");
        out.push_str(attrs);
        out.push_str(" d=\"M");
        svg::push_numbers(out, coordinates.as_flattened());
        out.push_str("z\"/>");
    }
}

/// A random move along one axis: normally distributed with a standard
/// deviation of [`MUTATION_STEP`] pixels, rounded to whole pixels.
fn random_step(rng: &mut Rng) -> i32 {
    (rng.normal() * MUTATION_STEP).round() as i32
}

/// Whether the angle at `corner`, between the edges to `a` and to `b`, is
/// wider than 15°.
///
/// The angle `θ` between two edges satisfies `cos θ = dot / (|ea|·|eb|)`, and
/// `θ > 15°` means `cos θ < cos 15°`. Multiplying both sides by the lengths
/// avoids dividing by zero: when an edge has no length, both sides are 0 and
/// `0 < 0` is false, so the triangle is invalid. Only `×`, `+` and `sqrt` are
/// used, which are exact under IEEE-754 on every platform (`acos` isn't).
fn is_wide_angle(corner: Point, a: Point, b: Point) -> bool {
    let (ax, ay) = (f64::from(a.x - corner.x), f64::from(a.y - corner.y));
    let (bx, by) = (f64::from(b.x - corner.x), f64::from(b.y - corner.y));
    let dot = ax * bx + ay * by;
    let lengths = (ax * ax + ay * ay).sqrt() * (bx * bx + by * by).sqrt();
    dot < COS_MIN_ANGLE * lengths
}

#[cfg(test)]
impl Triangle {
    /// Test shorthand: a triangle from `[x1, y1, x2, y2, x3, y3]`.
    pub(crate) fn from_coords([x1, y1, x2, y2, x3, y3]: [i32; 6]) -> Triangle {
        Triangle {
            vertices: [Point::new(x1, y1), Point::new(x2, y2), Point::new(x3, y3)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cos_min_angle_is_cos_15_degrees() {
        assert!((COS_MIN_ANGLE - 15f64.to_radians().cos()).abs() < 1e-15);
    }

    #[test]
    fn random_triangles_are_valid_and_in_bounds() {
        let mut rng = Rng::new(11);
        for _ in 0..1000 {
            let t = Triangle::random(64, 48, &mut rng);
            assert!(t.is_valid(), "{t:?}");
            for corner in t.vertices {
                assert!((-MARGIN..64 + MARGIN).contains(&corner.x), "{t:?}");
                assert!((-MARGIN..48 + MARGIN).contains(&corner.y), "{t:?}");
            }
        }
    }

    #[test]
    fn mutation_keeps_the_triangle_valid() {
        let mut rng = Rng::new(12);
        let mut t = Triangle::random(32, 32, &mut rng);
        for _ in 0..10_000 {
            t.mutate(32, 32, &mut rng);
            assert!(t.is_valid(), "{t:?}");
        }
    }

    #[test]
    fn validity_rejects_slivers_and_degenerates() {
        assert!(Triangle::from_coords([0, 0, 10, 0, 5, 9]).is_valid());
        // A 16° angle passes, a 14° one doesn't (tan 16° ≈ 0.287, tan 14° ≈ 0.249).
        assert!(Triangle::from_coords([0, 0, 1000, 0, 1000, 287]).is_valid());
        assert!(!Triangle::from_coords([0, 0, 1000, 0, 1000, 249]).is_valid());
        assert!(
            !Triangle::from_coords([0, 0, 100, 0, 50, 2]).is_valid(),
            "sliver"
        );
        assert!(
            !Triangle::from_coords([3, 3, 3, 3, 3, 3]).is_valid(),
            "point"
        );
        assert!(
            !Triangle::from_coords([0, 0, 5, 5, 10, 10]).is_valid(),
            "line"
        );
        assert!(
            !Triangle::from_coords([0, 0, 0, 0, 10, 4]).is_valid(),
            "two corners together"
        );
    }

    #[test]
    fn svg_coordinates_are_scaled_and_compact() {
        let t = Triangle::from_coords([0, 1, 9, -3, -16, 4]);
        let mut out = String::new();
        t.write_svg(&mut out, 2.0, "fill=\"#abc\"");
        // (v + 0.5) * 2 → 1 3 19 -5 -31 9, with no space before negatives.
        assert_eq!(out, "<path fill=\"#abc\" d=\"M1 3 19-5-31 9z\"/>");
    }
}
