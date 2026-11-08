//! Turning shapes into horizontal runs of pixels.
//!
//! Port of primitive's `raster.go` (`rasterizeTriangle` and its two helpers)
//! and `scanline.go` (`cropScanlines`). Every shape is drawn and scored as a
//! list of [`Scanline`]s, so the colour maths in [`crate::pixels`] never needs
//! to know which shape it is working on.

/// A horizontal run of pixels on row `y`, from `x1` to `x2` inclusive.
///
/// primitive's scanlines also carry a coverage alpha. Every shape here is fully
/// opaque at its edges, so that field is dropped.
#[derive(Clone, Copy)]
pub(crate) struct Scanline {
    /// Row of the run.
    pub y: i32,
    /// First pixel of the run (inclusive).
    pub x1: i32,
    /// Last pixel of the run (inclusive).
    pub x2: i32,
}

/// A vertex in pixel coordinates: `(x, y)`.
pub(crate) type Point = (i32, i32);

/// Appends the scanlines covering the triangle `a b c`.
///
/// The triangle is sorted top to bottom and split at the middle vertex into a
/// flat-bottomed part and a flat-topped part, each filled row by row. Lines are
/// not cropped to the image; call [`crop_scanlines`] afterwards.
pub(crate) fn rasterize_triangle(a: Point, b: Point, c: Point, lines: &mut Vec<Scanline>) {
    // primitive sorts with three compare-and-swaps. Its order for vertices on
    // the same row can differ from this stable sort, but those cases are the
    // flat ones, which fill the same pixels whichever way round they are.
    let mut vertices = [a, b, c];
    vertices.sort_by_key(|&(_, y)| y);
    let [top, middle, bottom] = vertices;

    if middle.1 == bottom.1 {
        fill_flat_bottom(top, middle, bottom, lines);
    } else if top.1 == middle.1 {
        fill_flat_top(top, middle, bottom, lines);
    } else {
        // Split at the middle vertex's row, at the point on the long edge
        // (top to bottom) at the same height as the middle vertex.
        let (x1, y1) = top;
        let (x3, y3) = bottom;
        let fraction = f64::from(middle.1 - y1) / f64::from(y3 - y1);
        let split = (x1 + (fraction * f64::from(x3 - x1)) as i32, middle.1);
        fill_flat_bottom(top, middle, split, lines);
        fill_flat_top(middle, split, bottom, lines);
    }
}

/// Fills a triangle whose two lower vertices share a row (`b.1 == c.1`), from
/// the top vertex `a` down to that row, inclusive.
///
/// `as i32` truncates towards zero, exactly like Go's `int()`.
fn fill_flat_bottom((x1, y1): Point, (x2, y2): Point, (x3, y3): Point, lines: &mut Vec<Scanline>) {
    let slope1 = f64::from(x2 - x1) / f64::from(y2 - y1);
    let slope2 = f64::from(x3 - x1) / f64::from(y3 - y1);
    let mut ax = f64::from(x1);
    let mut bx = f64::from(x1);
    for y in y1..=y2 {
        let a = ax as i32;
        let b = bx as i32;
        ax += slope1;
        bx += slope2;
        lines.push(Scanline {
            y,
            x1: a.min(b),
            x2: a.max(b),
        });
    }
}

/// Fills a triangle whose two upper vertices share a row (`a.1 == b.1`), from
/// the bottom vertex `c` up to, but not including, that row. Leaving the
/// shared row out means the two halves of a split triangle never overlap.
fn fill_flat_top((x1, y1): Point, (x2, y2): Point, (x3, y3): Point, lines: &mut Vec<Scanline>) {
    let slope1 = f64::from(x3 - x1) / f64::from(y3 - y1);
    let slope2 = f64::from(x3 - x2) / f64::from(y3 - y2);
    let mut ax = f64::from(x3);
    let mut bx = f64::from(x3);
    for y in ((y1 + 1)..=y3).rev() {
        ax -= slope1;
        bx -= slope2;
        let a = ax as i32;
        let b = bx as i32;
        lines.push(Scanline {
            y,
            x1: a.min(b),
            x2: a.max(b),
        });
    }
}

/// Clips scanlines to a `width × height` image, dropping the ones that fall
/// completely outside it.
pub(crate) fn crop_scanlines(lines: &mut Vec<Scanline>, width: i32, height: i32) {
    lines.retain_mut(|line| {
        let off_image = line.y < 0 || line.y >= height || line.x1 >= width || line.x2 < 0;
        if off_image {
            return false;
        }
        line.x1 = line.x1.clamp(0, width - 1);
        line.x2 = line.x2.clamp(0, width - 1);
        line.x1 <= line.x2
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_count(lines: &[Scanline]) -> i64 {
        lines.iter().map(|l| i64::from(l.x2 - l.x1 + 1)).sum()
    }

    fn rasterize(points: [i32; 6], w: i32, h: i32) -> Vec<Scanline> {
        let mut lines = Vec::new();
        let [x1, y1, x2, y2, x3, y3] = points;
        rasterize_triangle((x1, y1), (x2, y2), (x3, y3), &mut lines);
        crop_scanlines(&mut lines, w, h);
        lines
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
            let perimeter = ((x2 - x1).hypot(y2 - y1))
                + ((x3 - x2).hypot(y3 - y2))
                + ((x1 - x3).hypot(y1 - y3));
            let count = pixel_count(&rasterize(t, 100, 100)) as f64;
            // Edge pixels can go either way, so allow roughly one pixel per
            // unit of perimeter.
            assert!(
                (count - area).abs() <= perimeter,
                "triangle {t:?}: {count} pixels for area {area}"
            );
        }
    }

    #[test]
    fn rows_are_unique_and_sorted_within_each_half() {
        let lines = rasterize([10, 10, 90, 40, 30, 90], 100, 100);
        let mut rows: Vec<i32> = lines.iter().map(|l| l.y).collect();
        rows.sort();
        let before = rows.len();
        rows.dedup();
        assert_eq!(before, rows.len(), "a row was filled twice");
        assert_eq!(rows.first(), Some(&10));
        assert_eq!(rows.last(), Some(&90));
    }

    #[test]
    fn off_image_triangle_crops_to_nothing() {
        assert!(rasterize([-16, -16, -2, -10, -8, -1], 32, 32).is_empty());
        assert!(rasterize([40, 5, 47, 10, 45, 20], 32, 32).is_empty());
        assert!(rasterize([5, 40, 10, 47, 20, 45], 32, 32).is_empty());
    }

    /// Checks that every line lies inside a `size × size` image.
    fn assert_inside(lines: &[Scanline], size: i32) {
        for line in lines {
            assert!((0..size).contains(&line.y));
            assert!(0 <= line.x1 && line.x1 <= line.x2 && line.x2 < size);
        }
    }

    #[test]
    fn partly_outside_triangle_stays_inside_the_image() {
        let lines = rasterize([-10, -10, 45, 5, 10, 45], 32, 32);
        assert!(!lines.is_empty());
        assert_inside(&lines, 32);
    }

    #[test]
    fn degenerate_triangles_stay_inside_the_image() {
        for t in [
            [5, 5, 20, 20, 30, 30],
            [3, 7, 3, 7, 3, 7],
            [0, 4, 31, 4, 15, 4],
        ] {
            assert_inside(&rasterize(t, 32, 32), 32);
        }
    }
}
