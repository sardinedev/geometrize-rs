//! Turning shapes into horizontal runs of pixels.
//!
//! Every shape is drawn and scored as a list of [`Span`]s, so the colour
//! maths in [`crate::pixels`] never needs to know which shape it is working
//! on.
//!
//! **Coverage rule.** Shape coordinates are pixel centres: the point
//! `(x, y)` is the centre of pixel `(x, y)`. A pixel is covered when its
//! centre is inside the shape, using half-open intervals (`top <= y < bottom`
//! and `left <= x < right`). That gives exactly one span per row, and two
//! shapes that share an edge never both cover the same pixel.
//!
//! Only `+ − × ÷` and `ceil` are used on floats. They are exact under
//! IEEE-754, so every platform (native or WebAssembly) covers the same pixels.

/// A point in working-image pixel coordinates (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    /// Column, growing to the right.
    pub x: i32,
    /// Row, growing downwards.
    pub y: i32,
}

impl Point {
    /// Creates a point.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

/// A horizontal run of pixels on one row: columns `start..end` (`end` not
/// included). Spans are always inside the image and never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    /// The row.
    pub row: usize,
    /// First column covered.
    pub start: usize,
    /// One past the last column covered.
    pub end: usize,
}

impl Span {
    /// Number of pixels covered.
    pub(crate) fn len(&self) -> usize {
        self.end - self.start
    }
}

/// Replaces `spans` with the pixels the triangle covers, clipped to a
/// `width × height` image.
pub(crate) fn rasterize_triangle(
    vertices: [Point; 3],
    width: usize,
    height: usize,
    spans: &mut Vec<Span>,
) {
    spans.clear();
    let [a, b, c] = vertices;
    let top = a.y.min(b.y).min(c.y).max(0);
    let bottom = a.y.max(b.y).max(c.y).min(height as i32);
    let right_edge = width as f64;

    for row in top..bottom {
        let y = f64::from(row);
        // A row strictly inside the triangle crosses exactly two of its
        // edges (horizontal edges never count, see `crossing`).
        let mut crossings = [(a, b), (b, c), (c, a)]
            .into_iter()
            .filter_map(|(from, to)| crossing(from, to, y));
        let (Some(x1), Some(x2)) = (crossings.next(), crossings.next()) else {
            continue;
        };
        // Pixels whose centre `x` satisfies `left <= x < right`, inside the
        // image.
        let start = x1.min(x2).ceil().clamp(0.0, right_edge) as usize;
        let end = x1.max(x2).ceil().clamp(0.0, right_edge) as usize;
        if start < end {
            spans.push(Span {
                row: row as usize,
                start,
                end,
            });
        }
    }
}

/// Where the edge from `from` to `to` crosses row `y`, if it does.
///
/// An edge covers the rows from its upper end (included) to its lower end
/// (not included). So a horizontal edge covers no rows, and where two edges
/// meet at a vertex, only one of them counts for that row.
fn crossing(from: Point, to: Point, y: f64) -> Option<f64> {
    let (upper, lower) = if from.y <= to.y {
        (from, to)
    } else {
        (to, from)
    };
    let covers_row = f64::from(upper.y) <= y && y < f64::from(lower.y);
    if !covers_row {
        return None;
    }
    let fraction = (y - f64::from(upper.y)) / f64::from(lower.y - upper.y);
    Some(f64::from(upper.x) + fraction * f64::from(lower.x - upper.x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rasterize(coords: [i32; 6], width: usize, height: usize) -> Vec<Span> {
        let [x1, y1, x2, y2, x3, y3] = coords;
        let vertices = [Point::new(x1, y1), Point::new(x2, y2), Point::new(x3, y3)];
        let mut spans = Vec::new();
        rasterize_triangle(vertices, width, height, &mut spans);
        spans
    }

    fn pixel_count(spans: &[Span]) -> usize {
        spans.iter().map(Span::len).sum()
    }

    /// Checks every span is non-empty and inside a `width × height` image.
    fn assert_inside(spans: &[Span], width: usize, height: usize) {
        for span in spans {
            assert!(span.row < height, "{span:?}");
            assert!(span.start < span.end && span.end <= width, "{span:?}");
        }
    }

    #[test]
    fn pixel_count_is_close_to_area() {
        let triangles = [
            [10, 10, 90, 20, 40, 80],
            [5, 50, 95, 50, 50, 5],
            [0, 0, 99, 0, 0, 99],
            [20, 90, 80, 85, 50, 10],
        ];
        for t in triangles {
            let [x1, y1, x2, y2, x3, y3] = t.map(f64::from);
            let area = ((x2 - x1) * (y3 - y1) - (x3 - x1) * (y2 - y1)).abs() / 2.0;
            let perimeter =
                (x2 - x1).hypot(y2 - y1) + (x3 - x2).hypot(y3 - y2) + (x1 - x3).hypot(y1 - y3);
            let count = pixel_count(&rasterize(t, 100, 100)) as f64;
            // Pixel centres sample the area, so the count can only be off
            // along the edges: allow half a pixel per unit of perimeter.
            assert!(
                (count - area).abs() <= perimeter / 2.0,
                "triangle {t:?}: {count} pixels for area {area}"
            );
        }
    }

    #[test]
    fn one_span_per_row_between_the_top_and_bottom_vertices() {
        let spans = rasterize([10, 10, 90, 40, 30, 90], 100, 100);
        let rows: Vec<usize> = spans.iter().map(|span| span.row).collect();
        // Row 10 holds only the top vertex, a single point, which covers no
        // pixel. Row 90 is the bottom edge, which the half-open rule leaves
        // to whatever lies below.
        let expected: Vec<usize> = (11..90).collect();
        assert_eq!(rows, expected);
    }

    #[test]
    fn triangles_sharing_an_edge_cover_each_pixel_once() {
        // Two halves of a 20×20 square, split along the diagonal.
        let mut coverage = [[0u8; 20]; 20];
        for t in [[0, 0, 20, 0, 0, 20], [20, 0, 20, 20, 0, 20]] {
            for span in rasterize(t, 20, 20) {
                for cell in &mut coverage[span.row][span.start..span.end] {
                    *cell += 1;
                }
            }
        }
        assert!(coverage.iter().flatten().all(|&count| count == 1));
    }

    #[test]
    fn off_image_triangle_covers_nothing() {
        assert_eq!(rasterize([-16, -16, -2, -10, -8, -1], 32, 32), []);
        assert_eq!(rasterize([40, 5, 47, 10, 45, 20], 32, 32), []);
        assert_eq!(rasterize([5, 40, 10, 47, 20, 45], 32, 32), []);
    }

    #[test]
    fn partly_outside_triangle_is_clipped_to_the_image() {
        let spans = rasterize([-10, -10, 45, 5, 10, 45], 32, 32);
        assert_ne!(spans, []);
        assert_inside(&spans, 32, 32);
    }

    #[test]
    fn degenerate_triangles_cover_nothing() {
        for t in [
            [5, 5, 20, 20, 30, 30],
            [3, 7, 3, 7, 3, 7],
            [0, 4, 31, 4, 15, 4],
        ] {
            assert_eq!(rasterize(t, 32, 32), [], "{t:?}");
        }
    }

    #[test]
    fn spans_are_replaced_not_appended() {
        let mut spans = Vec::new();
        let big = [Point::new(0, 0), Point::new(30, 0), Point::new(0, 30)];
        let small = [Point::new(0, 0), Point::new(4, 0), Point::new(0, 4)];
        rasterize_triangle(big, 32, 32, &mut spans);
        rasterize_triangle(small, 32, 32, &mut spans);
        assert_eq!(spans.len(), 4);
    }
}
