//! Canvases and the pixel maths: best colour, alpha blending and scoring.
//!
//! The approach is primitive's: for each shape, work out the one colour
//! that brings the covered pixels closest to the target, blend it in, and
//! keep a running total of the squared error so a shape can be scored by
//! looking only at the pixels it covers. The maths is plain 8-bit integer
//! arithmetic with exact rounding, rather than primitive's 16-bit
//! premultiplied form (which comes from Go's `image/color` package).
//!
//! Images are opaque, so canvases store RGB only.

use std::ops::Range;

use crate::Rgba;
use crate::raster::Span;

/// One pixel: `[red, green, blue]`.
pub(crate) type Pixel = [u8; 3];

/// An opaque image, stored row by row.
///
/// Deliberately not `Debug`: printing every pixel is never useful.
#[derive(Clone)]
pub(crate) struct Canvas {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height` pixels.
    pub pixels: Vec<Pixel>,
}

impl Canvas {
    /// Creates a canvas with every pixel set to `color` (its alpha is
    /// ignored).
    pub(crate) fn filled(width: usize, height: usize, color: Rgba) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![[color.r, color.g, color.b]; width * height],
        }
    }

    /// Creates a canvas from RGBA bytes (4 per pixel, row by row), dropping
    /// the alpha byte: the image is treated as opaque.
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
            pixels: pixels.iter().map(|&[r, g, b, _alpha]| [r, g, b]).collect(),
        }
    }

    /// The pixels as flat RGB bytes (3 per pixel).
    pub(crate) fn as_rgb(&self) -> &[u8] {
        self.pixels.as_flattened()
    }

    /// Width and height as `i32`, the type shape coordinates use.
    pub(crate) fn size(&self) -> (i32, i32) {
        (self.width as i32, self.height as i32)
    }

    /// The pixels a span covers.
    pub(crate) fn span(&self, span: &Span) -> &[Pixel] {
        &self.pixels[self.span_range(span)]
    }

    /// The pixels a span covers, for changing them.
    pub(crate) fn span_mut(&mut self, span: &Span) -> &mut [Pixel] {
        let range = self.span_range(span);
        &mut self.pixels[range]
    }

    /// Where a span's pixels sit in `pixels`.
    ///
    /// Checks, in debug builds, that the span lies inside one row of the
    /// canvas. Slicing alone wouldn't catch a span that runs past the end of
    /// its row: it would silently continue into the next row and corrupt the
    /// error total. Rasterisers must never produce such a span (see `raster`).
    fn span_range(&self, span: &Span) -> Range<usize> {
        debug_assert!(
            span.row < self.height && span.start < span.end && span.end <= self.width,
            "{span:?} is outside the {}×{} canvas",
            self.width,
            self.height
        );
        let row_start = span.row * self.width;
        row_start + span.start..row_start + span.end
    }

    /// Number of colour values in the canvas (3 per pixel), the divisor for
    /// the root-mean-square difference.
    fn channel_count(&self) -> f64 {
        (self.pixels.len() * 3) as f64
    }
}

/// Finds the colour that, blended over `current` with the given alpha, gets
/// the covered pixels as close as possible to `target`.
///
/// Blending colour `c` with opacity `α` onto pixel `p` gives
/// `p + (c − p)·α`. Over all covered pixels, the squared distance to the
/// target is smallest when `c = mean(p) + (mean(target) − mean(p)) / α`.
/// So only the per-channel sums are needed, and the division happens once,
/// rounded to the nearest whole value.
///
/// The result is clamped to `0..=255`: a target darker or brighter than what
/// blending can reach asks for an impossible colour. If the spans cover no
/// pixels the result is black.
pub(crate) fn compute_color(target: &Canvas, current: &Canvas, spans: &[Span], alpha: u8) -> Rgba {
    let mut target_sums = [0i64; 3];
    let mut current_sums = [0i64; 3];
    let mut count = 0i64;
    for span in spans {
        for (&t, &c) in target.span(span).iter().zip(current.span(span)) {
            add_pixel(&mut target_sums, t);
            add_pixel(&mut current_sums, c);
        }
        count += span.len() as i64;
    }
    if count == 0 {
        return Rgba::new(0, 0, 0, alpha);
    }

    let channel = |i: usize| best_channel(target_sums[i], current_sums[i], count, alpha);
    Rgba::new(channel(0), channel(1), channel(2), alpha)
}

/// Adds each channel of `pixel` to `sums`.
fn add_pixel(sums: &mut [i64; 3], pixel: Pixel) {
    for (sum, value) in sums.iter_mut().zip(pixel) {
        *sum += i64::from(value);
    }
}

/// One channel of the best colour (see [`compute_color`]), from the sums of
/// that channel over `count` pixels.
///
/// With `α = alpha / 255`, the best value is
/// `(Σcurrent·alpha + (Σtarget − Σcurrent)·255) / (count·alpha)`.
/// The sums are at most `255 × 2³²` (the largest image), so they fit in `i64`
/// with room to spare.
fn best_channel(target_sum: i64, current_sum: i64, count: i64, alpha: u8) -> u8 {
    let alpha = i64::from(alpha);
    let numerator = current_sum * alpha + (target_sum - current_sum) * 255;
    let denominator = count * alpha;
    // Round to the nearest whole number. `div_euclid` rounds down even when
    // the numerator is negative, so adding half the denominator first rounds
    // to nearest in both directions.
    let rounded = (2 * numerator + denominator).div_euclid(2 * denominator);
    rounded.clamp(0, 255) as u8
}

/// The "source over" blend of one colour onto pixels:
/// `result = src·α + dst·(1 − α)`, in 8-bit integers, rounded to nearest.
/// Built once per shape, then applied to each pixel it covers.
struct Blend {
    /// Each channel of the source colour, times its alpha (`src·alpha`).
    weighted_src: [u32; 3],
    /// How much of the destination survives: `255 − alpha`.
    keep: u32,
}

impl Blend {
    fn new(color: Rgba) -> Blend {
        let alpha = u32::from(color.a);
        let weigh = |channel: u8| u32::from(channel) * alpha;
        Blend {
            weighted_src: [weigh(color.r), weigh(color.g), weigh(color.b)],
            keep: 255 - alpha,
        }
    }

    /// The pixel `dst` with the colour blended over it.
    fn apply(&self, dst: Pixel) -> Pixel {
        // `src·α + dst·(255 − α)` is at most 255·255; the `+ 127` rounds the
        // division to the nearest whole value.
        let mix = |src: u32, dst: u8| ((src + u32::from(dst) * self.keep + 127) / 255) as u8;
        [
            mix(self.weighted_src[0], dst[0]),
            mix(self.weighted_src[1], dst[1]),
            mix(self.weighted_src[2], dst[2]),
        ]
    }
}

/// Alpha-blends `color` over the pixels the spans cover.
pub(crate) fn draw_spans(canvas: &mut Canvas, color: Rgba, spans: &[Span]) {
    let blend = Blend::new(color);
    for span in spans {
        for pixel in canvas.span_mut(span) {
            *pixel = blend.apply(*pixel);
        }
    }
}

/// Squared difference of two pixels, summed over the three channels.
fn pixel_error(a: Pixel, b: Pixel) -> u64 {
    let mut sum = 0;
    for (x, y) in a.into_iter().zip(b) {
        let delta = u64::from(x.abs_diff(y));
        sum += delta * delta;
    }
    sum
}

/// Turns a total error (from [`difference_full`] or
/// [`difference_after_drawing`]) into a score: the root-mean-square
/// difference, scaled to `0.0..=1.0`.
///
/// The exact integer total is what the model keeps; the score is only
/// computed from it for display, so no rounding error builds up.
pub(crate) fn score_from_total(total: u64, canvas: &Canvas) -> f64 {
    (total as f64 / canvas.channel_count()).sqrt() / 255.0
}

/// Total error between two same-sized canvases: the sum of squared
/// differences over every channel of every pixel. 0 means identical.
pub(crate) fn difference_full(a: &Canvas, b: &Canvas) -> u64 {
    let mut total: u64 = 0;
    for (&pa, &pb) in a.pixels.iter().zip(&b.pixels) {
        total += pixel_error(pa, pb);
    }
    total
}

/// The total error `current` would have if `color` were drawn over the
/// spans, given its error now (`total`). `current` itself is left unchanged.
///
/// Only the pixels a shape covers can change the error, so instead of
/// comparing the whole image again this removes those pixels' old error from
/// the total and adds the error they would have once blended. That makes
/// trying a shape cost its own size, not the image size.
pub(crate) fn difference_after_drawing(
    target: &Canvas,
    current: &Canvas,
    total: u64,
    color: Rgba,
    spans: &[Span],
) -> u64 {
    let blend = Blend::new(color);
    let mut total = total;
    for span in spans {
        for (&t, &before) in target.span(span).iter().zip(current.span(span)) {
            let after = blend.apply(before);
            // Can't underflow: `total` includes exactly this pixel's old
            // error, because no pixel appears in two spans (see `raster`).
            total -= pixel_error(t, before);
            total += pixel_error(t, after);
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Point, rasterize_triangle};
    use crate::rng::Rng;
    use crate::shape::Triangle;

    /// A canvas with a colourful but deterministic pattern.
    fn pattern(width: usize, height: usize) -> Canvas {
        let mut canvas = Canvas::filled(width, height, Rgba::new(0, 0, 0, 255));
        for y in 0..height {
            for x in 0..width {
                canvas.pixels[y * width + x] = [
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x ^ y) * 8) as u8,
                ];
            }
        }
        canvas
    }

    /// Spans of a random (not necessarily valid) triangle, which may stick
    /// out of the image like real candidates do.
    fn random_spans(rng: &mut Rng, size: usize, spans: &mut Vec<Span>) {
        let mut coord = || rng.below(size as u32 + 32) as i32 - 16;
        let coords = [coord(), coord(), coord(), coord(), coord(), coord()];
        Triangle::from_coords(coords).rasterize(size, size, spans);
    }

    #[test]
    fn predicted_difference_matches_drawing_and_comparing() {
        let target = pattern(32, 32);
        let mut current = Canvas::filled(32, 32, Rgba::new(120, 80, 200, 255));
        let mut total = difference_full(&target, &current);
        let mut rng = Rng::new(5);
        let mut spans = Vec::new();

        for step in 0..100 {
            random_spans(&mut rng, 32, &mut spans);
            let color = compute_color(&target, &current, &spans, 128);
            total = difference_after_drawing(&target, &current, total, color, &spans);
            draw_spans(&mut current, color, &spans);
            assert_eq!(total, difference_full(&target, &current), "step {step}");
        }
    }

    #[test]
    fn best_color_is_as_good_as_brute_force() {
        // For each channel, try all 256 values and find the lowest error
        // after blending. At full opacity blending is exact, so the closed
        // form must find the true best. At partial opacity each blended pixel
        // is rounded to a whole value, which makes the error flat in places
        // and can move the best by a tiny amount: measured at under 0.0001%
        // of the error, so allow 0.001%.
        let target = pattern(32, 32);
        let current = Canvas::filled(32, 32, Rgba::new(40, 160, 90, 255));
        let total = difference_full(&target, &current);
        let mut rng = Rng::new(9);
        let mut spans = Vec::new();

        for alpha in [255, 100] {
            for _ in 0..20 {
                random_spans(&mut rng, 32, &mut spans);
                let color = compute_color(&target, &current, &spans, alpha);
                let chosen = [color.r, color.g, color.b];
                for channel in 0..3 {
                    let error_with = |value: u8| {
                        let mut channels = chosen;
                        channels[channel] = value;
                        let [r, g, b] = channels;
                        let color = Rgba::new(r, g, b, alpha);
                        difference_after_drawing(&target, &current, total, color, &spans)
                    };
                    let best = (0..=255u8).map(error_with).min().unwrap();
                    let ours = error_with(chosen[channel]);
                    let allowed = if alpha == 255 { 0 } else { best / 100_000 };
                    assert!(
                        ours <= best + allowed,
                        "alpha {alpha}, channel {channel}: error {ours}, best {best}"
                    );
                }
            }
        }
    }

    #[test]
    fn opaque_color_on_flat_target_is_the_target_color() {
        let flat = Rgba::new(200, 30, 90, 255);
        let target = Canvas::filled(16, 16, flat);
        let current = Canvas::filled(16, 16, Rgba::new(10, 250, 128, 255));
        let mut spans = Vec::new();
        rasterize_triangle(
            [Point::new(0, 0), Point::new(15, 2), Point::new(4, 15)],
            16,
            16,
            &mut spans,
        );
        assert_eq!(compute_color(&target, &current, &spans, 255), flat);
    }

    #[test]
    fn opaque_draw_writes_the_exact_color() {
        let mut canvas = Canvas::filled(8, 8, Rgba::new(255, 0, 3, 255));
        let spans = [Span {
            row: 2,
            start: 1,
            end: 7,
        }];
        draw_spans(&mut canvas, Rgba::new(17, 99, 254, 255), &spans);
        assert!(
            canvas
                .span(&spans[0])
                .iter()
                .all(|&pixel| pixel == [17, 99, 254])
        );
        // Pixels outside the span are untouched.
        assert_eq!(canvas.pixels[2 * 8], [255, 0, 3]);
        assert_eq!(canvas.pixels[2 * 8 + 7], [255, 0, 3]);
    }

    #[test]
    fn half_alpha_blends_halfway_rounded() {
        let mut canvas = Canvas::filled(4, 1, Rgba::new(0, 0, 0, 255));
        let spans = [Span {
            row: 0,
            start: 0,
            end: 4,
        }];
        draw_spans(&mut canvas, Rgba::new(255, 255, 255, 128), &spans);
        // 255 × 128 / 255 = 128 exactly.
        assert_eq!(canvas.pixels[0], [128, 128, 128]);
    }

    #[test]
    fn empty_spans_give_black_with_requested_alpha() {
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
        assert!((score_from_total(total, &black) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn alpha_is_dropped_from_rgba_input() {
        let bytes: Vec<u8> = (0..24).collect();
        let canvas = Canvas::from_rgba(3, 2, &bytes);
        assert_eq!(canvas.pixels[1], [4, 5, 6]);
        assert_eq!(
            canvas.as_rgb(),
            [
                0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 16, 17, 18, 20, 21, 22
            ]
        );
    }
}
