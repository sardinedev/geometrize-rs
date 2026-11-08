//! Triangles.
//!
//! Port of primitive's `triangle.go`: random creation, vertex mutation, the
//! minimum-angle validity test and rasterisation. The SVG output is new: a
//! compact `<path>` instead of primitive's `<polygon>`.

use crate::raster::{Point, Scanline, rasterize_triangle};
use crate::rng::Rng;
use crate::svg;

/// How far (in pixels) a vertex may sit outside the image. Letting vertices
/// overhang the edges lets triangles cover the borders with clean edges.
const MARGIN: i32 = 16;

/// Standard deviation (in pixels) of a vertex move during mutation.
const MUTATION_STEP: f64 = 16.0;

/// Every angle must be larger than this, which rules out thin slivers.
const MIN_ANGLE_DEGREES: f64 = 15.0;

/// A triangle in working-image pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    /// First vertex, x.
    pub x1: i32,
    /// First vertex, y.
    pub y1: i32,
    /// Second vertex, x.
    pub x2: i32,
    /// Second vertex, y.
    pub y2: i32,
    /// Third vertex, x.
    pub x3: i32,
    /// Third vertex, y.
    pub y3: i32,
}

impl Triangle {
    /// Creates a small random triangle somewhere on a `width × height` image:
    /// a random point plus two vertices within 15 pixels of it, then one
    /// mutation (which also makes it valid).
    pub(crate) fn random(width: i32, height: i32, rng: &mut Rng) -> Triangle {
        let x1 = rng.below(width as u32) as i32;
        let y1 = rng.below(height as u32) as i32;
        let mut offset = || rng.below(31) as i32 - 15;
        let mut triangle = Triangle {
            x1,
            y1,
            x2: x1 + offset(),
            y2: y1 + offset(),
            x3: x1 + offset(),
            y3: y1 + offset(),
        };
        triangle.mutate(width, height, rng);
        triangle
    }

    /// Moves one random vertex by a normally distributed amount (clamped to
    /// the image plus [`MARGIN`]), repeating until the triangle is valid.
    pub(crate) fn mutate(&mut self, width: i32, height: i32, rng: &mut Rng) {
        loop {
            let (x, y) = match rng.below(3) {
                0 => (&mut self.x1, &mut self.y1),
                1 => (&mut self.x2, &mut self.y2),
                _ => (&mut self.x3, &mut self.y3),
            };
            // `as i32` truncates towards zero, like Go's `int()`.
            *x = (*x + (rng.normal() * MUTATION_STEP) as i32).clamp(-MARGIN, width - 1 + MARGIN);
            *y = (*y + (rng.normal() * MUTATION_STEP) as i32).clamp(-MARGIN, height - 1 + MARGIN);
            if self.is_valid() {
                return;
            }
        }
    }

    /// The three vertices as `(x, y)` points.
    fn vertices(&self) -> [Point; 3] {
        [(self.x1, self.y1), (self.x2, self.y2), (self.x3, self.y3)]
    }

    /// Whether every angle of the triangle is larger than 15°.
    ///
    /// Degenerate triangles (two vertices in the same place) produce NaN
    /// angles, and NaN fails every comparison, so they are invalid too.
    pub fn is_valid(&self) -> bool {
        let [p1, p2, p3] = self.vertices();
        let a1 = angle_degrees(p1, p2, p3);
        let a2 = angle_degrees(p2, p1, p3);
        let a3 = 180.0 - a1 - a2;
        a1 > MIN_ANGLE_DEGREES && a2 > MIN_ANGLE_DEGREES && a3 > MIN_ANGLE_DEGREES
    }

    /// Appends the (uncropped) scanlines covering the triangle.
    pub(crate) fn rasterize(&self, lines: &mut Vec<Scanline>) {
        let [p1, p2, p3] = self.vertices();
        rasterize_triangle(p1, p2, p3, lines);
    }

    /// Appends `<path {attrs} d="Mx1 y1 x2 y2 x3 y3z"/>` with each coordinate
    /// mapped to output units.
    pub(crate) fn write_svg(&self, out: &mut String, scale: f64, attrs: &str) {
        let points = [self.x1, self.y1, self.x2, self.y2, self.x3, self.y3]
            .map(|v| svg::scale_coordinate(v, scale));
        out.push_str("<path ");
        out.push_str(attrs);
        out.push_str(" d=\"M");
        svg::push_numbers(out, &points);
        out.push_str("z\"/>");
    }
}

/// The angle, in degrees, at `vertex` between the edges to `a` and to `b`.
fn angle_degrees(vertex: Point, a: Point, b: Point) -> f64 {
    let (ax, ay) = (f64::from(a.0 - vertex.0), f64::from(a.1 - vertex.1));
    let (bx, by) = (f64::from(b.0 - vertex.0), f64::from(b.1 - vertex.1));
    let a_length = ax.hypot(ay);
    let b_length = bx.hypot(by);
    let cosine = (ax * bx + ay * by) / (a_length * b_length);
    cosine.acos().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_triangles_are_valid_and_in_bounds() {
        let mut rng = Rng::new(11);
        for _ in 0..1000 {
            let t = Triangle::random(64, 48, &mut rng);
            assert!(t.is_valid(), "{t:?}");
            for x in [t.x1, t.x2, t.x3] {
                assert!((-MARGIN..64 + MARGIN).contains(&x));
            }
            for y in [t.y1, t.y2, t.y3] {
                assert!((-MARGIN..48 + MARGIN).contains(&y));
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
        let equilateral_ish = Triangle {
            x1: 0,
            y1: 0,
            x2: 10,
            y2: 0,
            x3: 5,
            y3: 9,
        };
        assert!(equilateral_ish.is_valid());
        let sliver = Triangle {
            x1: 0,
            y1: 0,
            x2: 100,
            y2: 0,
            x3: 50,
            y3: 2,
        };
        assert!(!sliver.is_valid());
        let point = Triangle {
            x1: 3,
            y1: 3,
            x2: 3,
            y2: 3,
            x3: 3,
            y3: 3,
        };
        assert!(!point.is_valid());
        let line = Triangle {
            x1: 0,
            y1: 0,
            x2: 5,
            y2: 5,
            x3: 10,
            y3: 10,
        };
        assert!(!line.is_valid());
    }

    #[test]
    fn svg_coordinates_are_scaled_and_compact() {
        let t = Triangle {
            x1: 0,
            y1: 1,
            x2: 9,
            y2: -3,
            x3: -16,
            y3: 4,
        };
        let mut out = String::new();
        t.write_svg(&mut out, 2.0, "fill=\"#abc\"");
        // (v + 0.5) * 2 → 1 3 19 -5 -31 9, with no space before negatives.
        assert_eq!(out, "<path fill=\"#abc\" d=\"M1 3 19-5-31 9z\"/>");
    }
}
