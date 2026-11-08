//! Canvases and the pixel maths: best colour, alpha blending and scoring.
//!
//! Integer-for-integer port of primitive's `core.go` (`computeColor`,
//! `copyLines`, `drawLines`, `differenceFull`, `differencePartial`), so the
//! results behave like primitive's.
//!
//! Two conventions from Go's `image/color` package show up throughout:
//! - **16-bit colour.** Go widens an 8-bit channel `c` to 16 bits as
//!   `c * 0x101` (so 0xff becomes 0xffff), works in 16 bits, then shifts back
//!   down with `>> 8`. Doing the maths at 16 bits keeps rounding errors small.
//! - **Premultiplied colour.** When blending, the source colour is first
//!   multiplied by its own alpha.

use crate::Rgba;
use crate::raster::Scanline;

/// One pixel: `[red, green, blue, alpha]`.
pub(crate) type Pixel = [u8; 4];

/// An RGBA image, stored row by row.
///
/// Deliberately not `Debug`: printing every pixel is never useful.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Canvas {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height` pixels.
    pub pixels: Vec<Pixel>,
}

impl Canvas {
    /// Creates a canvas with every pixel set to `color`.
    pub(crate) fn filled(width: usize, height: usize, color: Rgba) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![[color.r, color.g, color.b, color.a]; width * height],
        }
    }

    /// Creates a canvas from RGBA bytes (4 per pixel, row by row).
    ///
    /// # Panics
    /// Panics if `rgba` isn't exactly `width * height * 4` bytes.
    pub(crate) fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Canvas {
        let (pixels, leftover) = rgba.as_chunks::<4>();
        assert!(
            leftover.is_empty() && pixels.len() == width * height,
            "RGBA buffer doesn't match a {width}×{height} image"
        );
        Canvas {
            width,
            height,
            pixels: pixels.to_vec(),
        }
    }

    /// The pixels as flat RGBA bytes.
    pub(crate) fn as_rgba(&self) -> &[u8] {
        self.pixels.as_flattened()
    }

    /// Index of pixel `(x, y)` in [`Canvas::pixels`]. The coordinates must be
    /// inside the canvas, which cropped scanlines always are.
    pub(crate) fn index(&self, x: i32, y: i32) -> usize {
        y as usize * self.width + x as usize
    }

    /// Number of colour values in the canvas (4 per pixel), the divisor for
    /// the root-mean-square difference.
    fn channel_count(&self) -> f64 {
        (self.pixels.len() * 4) as f64
    }
}

/// Finds the colour that, drawn over `current` with the given alpha, gets the
/// scanline pixels as close as possible to `target`.
///
/// Port of `computeColor`. Blending a colour `c` with opacity `α` onto a pixel
/// gives `current·(1-α) + c·α`. Setting that equal to `target` and solving for
/// `c` gives `c = (target - current)/α + current`. This averages that ideal
/// colour over every pixel the shape covers, in 16-bit fixed point:
/// `1/α = 255/alpha`, and the extra `0x101` widens to 16 bits.
///
/// If the lines cover no pixels the result is black (primitive does the same).
pub(crate) fn compute_color(
    target: &Canvas,
    current: &Canvas,
    lines: &[Scanline],
    alpha: u8,
) -> Rgba {
    let inverse_alpha = 0x101 * 255 / i64::from(alpha);
    // The ideal value of one channel, in 16 bits.
    let ideal = |t: u8, c: u8| (i64::from(t) - i64::from(c)) * inverse_alpha + i64::from(c) * 0x101;

    let mut r_sum: i64 = 0;
    let mut g_sum: i64 = 0;
    let mut b_sum: i64 = 0;
    let mut count: i64 = 0;

    for line in lines {
        for x in line.x1..=line.x2 {
            let i = target.index(x, line.y);
            let [tr, tg, tb, _] = target.pixels[i];
            let [cr, cg, cb, _] = current.pixels[i];
            r_sum += ideal(tr, cr);
            g_sum += ideal(tg, cg);
            b_sum += ideal(tb, cb);
            count += 1;
        }
    }

    if count == 0 {
        return Rgba::new(0, 0, 0, alpha);
    }

    // Mean in 16 bits, then back to 8 bits. The ideal colour can fall outside
    // 0..=255 (a target darker than what alpha blending can reach), so clamp.
    let to_channel = |sum: i64| ((sum / count) >> 8).clamp(0, 255) as u8;
    Rgba::new(
        to_channel(r_sum),
        to_channel(g_sum),
        to_channel(b_sum),
        alpha,
    )
}

/// Copies the scanline pixels from `src` into `dst`. Both canvases must be the
/// same size.
///
/// Port of `copyLines`. Used to reset just the part of a scratch canvas a
/// shape is about to touch, instead of copying the whole image.
pub(crate) fn copy_lines(dst: &mut Canvas, src: &Canvas, lines: &[Scanline]) {
    for line in lines {
        let row = src.index(line.x1, line.y)..=src.index(line.x2, line.y);
        dst.pixels[row.clone()].copy_from_slice(&src.pixels[row]);
    }
}

/// Alpha-blends `color` over the scanline pixels of `canvas`.
///
/// Port of `drawLines` with full coverage, the "source over" blend
/// `result = dst·(1-α) + src·α` done in Go's 16-bit premultiplied form.
pub(crate) fn draw_lines(canvas: &mut Canvas, color: Rgba, lines: &[Scanline]) {
    const MAX: u32 = 0xffff;
    let alpha = u32::from(color.a);

    // The source colour as 16-bit premultiplied values, the same numbers Go's
    // `color.NRGBA.RGBA()` returns.
    let premultiply = |channel: u8| u32::from(channel) * 0x101 * alpha / 0xff;
    let src_r = premultiply(color.r);
    let src_g = premultiply(color.g);
    let src_b = premultiply(color.b);
    let src_a = alpha * 0x101;

    // How much of the destination survives, scaled so that
    // `dst (8-bit) * keep / MAX` comes out in 16 bits.
    let keep = (MAX - src_a) * 0x101;

    // This can't overflow u32: `dst * keep <= MAX * (MAX - src_a)` and
    // `src * MAX <= src_a * MAX`, so the sum is at most `MAX * MAX`, which is
    // below `u32::MAX`.
    let blend = |dst: u8, src: u32| (((u32::from(dst) * keep + src * MAX) / MAX) >> 8) as u8;

    for line in lines {
        for x in line.x1..=line.x2 {
            let i = canvas.index(x, line.y);
            let [r, g, b, a] = canvas.pixels[i];
            canvas.pixels[i] = [
                blend(r, src_r),
                blend(g, src_g),
                blend(b, src_b),
                blend(a, src_a),
            ];
        }
    }
}

/// Squared difference of two pixels, summed over all four channels.
fn pixel_error(a: Pixel, b: Pixel) -> u64 {
    let mut sum = 0;
    for (x, y) in a.into_iter().zip(b) {
        let delta = i32::from(x) - i32::from(y);
        sum += (delta * delta) as u64;
    }
    sum
}

/// Turns a total error (from [`difference_full`] or [`difference_partial`])
/// into a score: the root-mean-square difference, scaled to `0.0..=1.0`.
///
/// primitive keeps only this float and converts it back to a total on every
/// update. We keep the exact integer total instead and compute the score from
/// it when it's shown, so no rounding error can build up from step to step.
pub(crate) fn score_from_total(total: u64, canvas: &Canvas) -> f64 {
    (total as f64 / canvas.channel_count()).sqrt() / 255.0
}

/// Total error between two same-sized canvases: the sum of squared
/// differences over every channel of every pixel. 0 means identical.
///
/// Port of `differenceFull`, returning the integer total rather than the
/// score (see [`score_from_total`]).
pub(crate) fn difference_full(a: &Canvas, b: &Canvas) -> u64 {
    let mut total: u64 = 0;
    for (&pa, &pb) in a.pixels.iter().zip(&b.pixels) {
        total += pixel_error(pa, pb);
    }
    total
}

/// Updates `total` (the error between `target` and `before`) to the error
/// between `target` and `after`, where the two canvases differ only on
/// `lines`.
///
/// Port of `differencePartial`. Only the pixels a shape touches can change the
/// error, so instead of comparing the whole image again this removes those
/// pixels' old error from the total and adds their new error. That makes
/// trying a shape cost its own size, not the image size.
pub(crate) fn difference_partial(
    target: &Canvas,
    before: &Canvas,
    after: &Canvas,
    total: u64,
    lines: &[Scanline],
) -> u64 {
    let mut total = total;
    for line in lines {
        for x in line.x1..=line.x2 {
            let i = target.index(x, line.y);
            // Can't underflow: `total` includes exactly this pixel's old error,
            // as long as no pixel appears in two lines (a rule every shape's
            // rasteriser follows; see the `shape` module docs).
            total -= pixel_error(target.pixels[i], before.pixels[i]);
            total += pixel_error(target.pixels[i], after.pixels[i]);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{crop_scanlines, rasterize_triangle};
    use crate::rng::Rng;

    /// A canvas with a colourful but deterministic pattern.
    fn pattern(width: usize, height: usize) -> Canvas {
        let mut canvas = Canvas::filled(width, height, Rgba::new(0, 0, 0, 255));
        for y in 0..height {
            for x in 0..width {
                canvas.pixels[y * width + x] = [
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x ^ y) * 8) as u8,
                    255,
                ];
            }
        }
        canvas
    }

    fn random_lines(rng: &mut Rng, w: i32, h: i32, lines: &mut Vec<Scanline>) {
        lines.clear();
        let mut coord = |n: i32| rng.below((n + 32) as u32) as i32 - 16;
        let (x1, y1, x2, y2, x3, y3) = (coord(w), coord(h), coord(w), coord(h), coord(w), coord(h));
        rasterize_triangle((x1, y1), (x2, y2), (x3, y3), lines);
        crop_scanlines(lines, w, h);
    }

    #[test]
    fn partial_difference_matches_full_difference() {
        let target = pattern(32, 32);
        let mut current = Canvas::filled(32, 32, Rgba::new(120, 80, 200, 255));
        let mut before = current.clone();
        let mut total = difference_full(&target, &current);
        let mut rng = Rng::new(5);
        let mut lines = Vec::new();

        for step in 0..100 {
            random_lines(&mut rng, 32, 32, &mut lines);
            let color = compute_color(&target, &current, &lines, 128);
            copy_lines(&mut before, &current, &lines);
            draw_lines(&mut current, color, &lines);
            total = difference_partial(&target, &before, &current, total, &lines);
            let full = difference_full(&target, &current);
            assert_eq!(total, full, "step {step}");
        }
    }

    #[test]
    fn opaque_color_on_flat_target_is_the_target_color() {
        let flat = Rgba::new(200, 30, 90, 255);
        let target = Canvas::filled(16, 16, flat);
        let current = Canvas::filled(16, 16, Rgba::new(10, 250, 128, 255));
        let mut lines = Vec::new();
        rasterize_triangle((0, 0), (15, 2), (4, 15), &mut lines);
        assert_eq!(compute_color(&target, &current, &lines, 255), flat);
    }

    #[test]
    fn opaque_draw_writes_the_exact_color() {
        let color = Rgba::new(17, 99, 254, 255);
        let mut canvas = Canvas::filled(8, 8, Rgba::new(255, 0, 3, 255));
        let lines = [Scanline { y: 2, x1: 1, x2: 6 }];
        draw_lines(&mut canvas, color, &lines);
        for x in 1..=6 {
            assert_eq!(canvas.pixels[canvas.index(x, 2)], [17, 99, 254, 255]);
        }
        // Pixels outside the line are untouched.
        assert_eq!(canvas.pixels[canvas.index(0, 2)], [255, 0, 3, 255]);
    }

    #[test]
    fn half_alpha_blends_halfway() {
        let mut canvas = Canvas::filled(4, 1, Rgba::new(0, 0, 0, 255));
        let lines = [Scanline { y: 0, x1: 0, x2: 3 }];
        draw_lines(&mut canvas, Rgba::new(255, 255, 255, 128), &lines);
        let [red, _, _, alpha] = canvas.pixels[0];
        assert!((127..=129).contains(&red), "got {red}");
        assert_eq!(alpha, 255, "opaque stays opaque");
    }

    #[test]
    fn empty_lines_give_black_with_requested_alpha() {
        let canvas = Canvas::filled(4, 4, Rgba::new(9, 9, 9, 255));
        assert_eq!(
            compute_color(&canvas, &canvas, &[], 128),
            Rgba::new(0, 0, 0, 128)
        );
    }

    #[test]
    fn identical_canvases_have_zero_difference() {
        let a = pattern(16, 16);
        assert_eq!(difference_full(&a, &a), 0);
        let black = Canvas::filled(16, 16, Rgba::new(0, 0, 0, 0));
        let white = Canvas::filled(16, 16, Rgba::new(255, 255, 255, 255));
        let total = difference_full(&black, &white);
        assert_eq!(score_from_total(total, &black), 1.0);
    }

    #[test]
    fn rgba_bytes_round_trip() {
        let bytes: Vec<u8> = (0..24).collect();
        let canvas = Canvas::from_rgba(3, 2, &bytes);
        assert_eq!(canvas.pixels[1], [4, 5, 6, 7]);
        assert_eq!(canvas.as_rgba(), &bytes[..]);
    }
}
