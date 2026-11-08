//! The model: target image, picture so far, and the shapes added to it.
//!
//! Port of primitive's `model.go` (`NewModel`, `Step`, `Add`), without the
//! `repeat` loop and the PNG/GIF output, and with a stop condition on SVG size.

use crate::pixels::{
    Canvas, compute_color, copy_lines, difference_full, difference_partial, draw_lines,
};
use crate::raster::Scanline;
use crate::search::{SearchContext, best_hill_climb};
use crate::shape::Shape;
use crate::svg::{self, Frame};
use crate::{Error, Options, Rgba};

use std::fmt;

/// Largest supported image side, in pixels. Working images are a few hundred
/// pixels, and this keeps all the `i32` coordinate maths far from overflowing.
const MAX_SIDE: u32 = 1 << 16;

/// What one [`Model::step`] added.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepResult {
    /// The new shape, in working-image pixels.
    pub shape: Shape,
    /// Its colour; the alpha is the opacity as an 8-bit value.
    pub color: Rgba,
    /// The score after adding it (0 = identical to the target, 1 = opposite).
    pub score: f64,
    /// Size of the SVG after adding it, in bytes.
    pub bytes: usize,
}

/// Approximates a target image one shape at a time.
#[derive(Clone)]
pub struct Model {
    /// The image being approximated (alpha forced to 255).
    target: Canvas,
    /// The picture so far.
    current: Canvas,
    /// Scratch canvas holding `current` as it was before the last shape, on
    /// that shape's scanlines only, for the partial score update.
    before: Canvas,
    /// Scratch scanlines for adding a shape.
    lines: Vec<Scanline>,
    /// Difference between `target` and `current`.
    score: f64,
    /// Background colour.
    background: Rgba,
    /// `options.opacity` rounded to a whole percentage (1 to 100). Both the
    /// pixel maths and the SVG use this rounded value, so they always agree.
    opacity_percent: u8,
    /// `opacity_percent` as an 8-bit alpha.
    alpha: u8,
    /// Settings.
    options: Options,
    /// Size of the SVG at `options.out_width`.
    frame: Frame,
    /// Shapes added so far, with their colours.
    shapes: Vec<(Shape, Rgba)>,
    /// Number of steps searched so far (seeds the next search).
    step_index: u64,
    /// Current size of the SVG at `options.out_width`, in bytes.
    bytes: usize,
    /// Set once `step` returns `None`, so it keeps returning `None`.
    done: bool,
}

impl Model {
    /// Creates a model for an RGBA image of `width × height` pixels.
    ///
    /// `rgba` holds 4 bytes per pixel, row by row, at the working size (the
    /// model doesn't resize). The alpha channel is ignored: the image is
    /// treated as opaque.
    ///
    /// # Errors
    /// Returns an [`Error`] if the image is empty or too large, the buffer
    /// has the wrong length, or an option is out of range (see [`Error`]'s
    /// variants).
    pub fn new(rgba: &[u8], width: u32, height: u32, options: Options) -> Result<Model, Error> {
        if width == 0 || height == 0 {
            return Err(Error::EmptyImage);
        }
        let too_large = Error::ImageTooLarge { width, height };
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(too_large);
        }
        let (w, h) = (width as usize, height as usize);
        // `checked_mul` because on 32-bit targets (WebAssembly) a large image's
        // byte count doesn't fit in `usize`.
        let expected = w
            .checked_mul(h)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(too_large)?;
        if rgba.len() != expected {
            return Err(Error::BufferLength {
                expected,
                actual: rgba.len(),
            });
        }
        let opacity_percent = validate_options(&options)?;
        // Integer rounding of `percent × 255 / 100` (50% → 128).
        let alpha = ((u32::from(opacity_percent) * 255 + 50) / 100) as u8;

        let mut target = Canvas::from_rgba(w, h, rgba);
        for pixel in &mut target.pixels {
            pixel[3] = 255;
        }

        let background = options.bg.unwrap_or_else(|| average_color(&target));
        let background = Rgba {
            a: 255,
            ..background
        };
        let current = Canvas::filled(w, h, background);
        let score = difference_full(&target, &current);

        let frame = Frame::new(width, height, options.out_width);
        let bytes = svg::empty_len(frame, background);
        if let Some(limit) = options.limit.max_bytes()
            && limit < bytes
        {
            return Err(Error::ByteLimitTooSmall {
                limit,
                minimum: bytes,
            });
        }

        Ok(Model {
            before: current.clone(),
            target,
            current,
            lines: Vec::new(),
            score,
            background,
            opacity_percent,
            alpha,
            options,
            frame,
            shapes: Vec::new(),
            step_index: 0,
            bytes,
            done: false,
        })
    }

    /// Searches for the best next shape and adds it.
    ///
    /// Returns `None`, without changing anything, once a limit in
    /// [`Options::limit`] is reached. With a byte limit, the shape that would
    /// have pushed the SVG past the limit is discarded, so the SVG never
    /// exceeds it. It also returns `None` if even the best shape found
    /// wouldn't improve the picture, so the score always goes down.
    /// After the first `None`, every later call returns `None`.
    pub fn step(&mut self) -> Option<StepResult> {
        if self.done {
            return None;
        }
        if let Some(max_shapes) = self.options.limit.max_shapes()
            && self.shapes.len() >= max_shapes as usize
        {
            self.done = true;
            return None;
        }

        let ctx = SearchContext {
            target: &self.target,
            current: &self.current,
            score: self.score,
            alpha: self.alpha,
            options: &self.options,
            step: self.step_index,
        };
        let best = best_hill_climb(&ctx);
        self.step_index += 1;

        // Adding a shape that doesn't lower the score would only waste bytes.
        // This happens when the picture can't get any closer, for example a
        // flat image that already matches its background.
        if best.energy >= self.score {
            self.done = true;
            return None;
        }
        let shape = best.shape;

        let (width, height) = (self.target.width as i32, self.target.height as i32);
        shape.rasterize(width, height, &mut self.lines);
        let color = compute_color(&self.target, &self.current, &self.lines, self.alpha);

        let added_bytes = svg::shape_len(self.frame, &shape, color, self.opacity_percent);
        if let Some(max_bytes) = self.options.limit.max_bytes()
            && self.bytes + added_bytes > max_bytes
        {
            self.done = true;
            return None;
        }

        copy_lines(&mut self.before, &self.current, &self.lines);
        draw_lines(&mut self.current, color, &self.lines);
        self.score = difference_partial(
            &self.target,
            &self.before,
            &self.current,
            self.score,
            &self.lines,
        );
        self.shapes.push((shape, color));
        self.bytes += added_bytes;

        Some(StepResult {
            shape,
            color,
            score: self.score,
            bytes: self.bytes,
        })
    }

    /// Current score: the root-mean-square difference between the target and
    /// the picture so far, from 0 (identical) to 1.
    pub fn score(&self) -> f64 {
        self.score
    }

    /// The shapes added so far, oldest first, with their colours.
    pub fn shapes(&self) -> &[(Shape, Rgba)] {
        &self.shapes
    }

    /// The background colour (always opaque).
    pub fn background(&self) -> Rgba {
        self.background
    }

    /// Size in bytes of [`Model::svg`] right now.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Working image width in pixels.
    pub fn width(&self) -> u32 {
        self.target.width as u32
    }

    /// Working image height in pixels.
    pub fn height(&self) -> u32 {
        self.target.height as u32
    }

    /// The picture so far as RGBA bytes at the working size.
    pub fn pixels(&self) -> &[u8] {
        self.current.as_rgba()
    }

    /// The SVG at [`Options::out_width`].
    pub fn svg(&self) -> String {
        self.svg_document(self.frame)
    }

    /// The SVG at a different output width (at least 1). The byte limit only
    /// applies to [`Model::svg`]; other widths can be a little larger or
    /// smaller.
    pub fn svg_at(&self, out_width: u32) -> String {
        let frame = Frame::new(self.width(), self.height(), out_width);
        self.svg_document(frame)
    }

    fn svg_document(&self, frame: Frame) -> String {
        svg::document(frame, self.background, &self.shapes, self.opacity_percent)
    }
}

/// Checks the options and returns the opacity as a whole percentage.
fn validate_options(options: &Options) -> Result<u8, Error> {
    // `contains` is false for NaN, so NaN is rejected too.
    if !(0.01..=1.0).contains(&options.opacity) {
        return Err(Error::Opacity(options.opacity));
    }
    if options.shapes.is_empty() {
        return Err(Error::NoShapeKinds);
    }
    if options.candidates == 0 {
        return Err(Error::Zero("candidates"));
    }
    if options.climbs == 0 {
        return Err(Error::Zero("climbs"));
    }
    if options.out_width == 0 {
        return Err(Error::Zero("output width"));
    }
    Ok((options.opacity * 100.0).round() as u8)
}

/// Shows the settings and progress, not the pixels (three full canvases
/// would flood a log or the browser console).
impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("width", &self.target.width)
            .field("height", &self.target.height)
            .field("score", &self.score)
            .field("shapes", &self.shapes.len())
            .field("bytes", &self.bytes)
            .field("background", &self.background)
            .field("done", &self.done)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// The mean red, green and blue of the image, opaque.
///
/// Port of primitive's `AverageImageColor`.
fn average_color(image: &Canvas) -> Rgba {
    let (mut r_sum, mut g_sum, mut b_sum) = (0u64, 0u64, 0u64);
    for &[r, g, b, _] in &image.pixels {
        r_sum += u64::from(r);
        g_sum += u64::from(g);
        b_sum += u64::from(b);
    }
    let count = image.pixels.len() as u64;
    let mean = |sum: u64| (sum / count) as u8;
    Rgba::new(mean(r_sum), mean(g_sum), mean(b_sum), 255)
}
