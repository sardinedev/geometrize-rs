//! WebAssembly bindings for [`geometrize_core`], used by the browser preview
//! in `web/`.
//!
//! The page creates a [`Session`] from the photo's pixels and a
//! [`SessionOptions`], then calls [`Session::step`] in a loop inside a Web
//! Worker, drawing each returned shape on a `<canvas>` as it arrives:
//!
//! ```js
//! import init, { Session, SessionOptions } from "geometrize-wasm";
//!
//! await init();
//! const options = new SessionOptions();
//! options.maxBytes = 30_000;
//! options.seed = 42n; // a BigInt, so any seed the CLI prints works here too
//! const session = new Session(imageData.data, imageData.width, imageData.height, options);
//! let shape;
//! while ((shape = session.step()) !== undefined) {
//!   draw(shape); // [kind, ...coordinates, r, g, b, a]; see `Session::step`
//! }
//! const svg = session.svg();
//! ```
//!
//! Everything runs single-threaded: the core is built without its `parallel`
//! feature, because threads in WebAssembly need `SharedArrayBuffer`, and
//! with it cross-origin isolation headers on the site. The output is the same
//! anyway: a seed gives the same SVG here as in the `geometrize` CLI, for the
//! same working-size pixels.
//!
//! This crate only translates types. Everything else, including validating
//! the options, is the core's job, and its error messages are passed on
//! unchanged.

use geometrize_core::{Limit, Model, Options, Shape, StepResult};
use wasm_bindgen::Clamped;
use wasm_bindgen::prelude::*;

/// Settings for a [`Session`]. `new SessionOptions()` starts from the core's
/// defaults; set the fields you need before creating the session.
///
/// The whole-number fields are `u32`. wasm-bindgen converts JS numbers to
/// them with `>>> 0`, so a negative or too-large value silently wraps
/// (`-1` becomes 4294967295) instead of throwing: the page must keep its
/// inputs in range.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct SessionOptions {
    /// Opacity of every shape, from 0.01 to 1. Default 0.5.
    pub opacity: f32,
    /// Stop after this many shapes. Default `undefined`.
    ///
    /// The two limits work as `-n` and `--max-size` do in the CLI: set either
    /// or both (whichever is reached first). With neither set, the core's
    /// default limit applies (500 shapes).
    #[wasm_bindgen(js_name = maxShapes)]
    pub max_shapes: Option<u32>,
    /// Keep the SVG at or under this many bytes. Default `undefined`; see
    /// `maxShapes`.
    #[wasm_bindgen(js_name = maxBytes)]
    pub max_bytes: Option<u32>,
    /// Seed for the random search, as a `BigInt`. Default `0n`.
    pub seed: u64,
    /// Width of the SVG's viewBox. Default 1200.
    #[wasm_bindgen(js_name = outWidth)]
    pub out_width: u32,
    /// Random shapes tried per search. Default 1000.
    pub candidates: u32,
    /// Failed mutations in a row before a hill climb gives up. Default 100.
    pub age: u32,
    /// Independent searches per step. Default 16.
    pub climbs: u32,
}

#[wasm_bindgen]
impl SessionOptions {
    /// The core's default settings, with neither limit set.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> SessionOptions {
        let defaults = Options::default();
        SessionOptions {
            opacity: defaults.opacity,
            max_shapes: None,
            max_bytes: None,
            seed: defaults.seed,
            out_width: defaults.out_width,
            candidates: defaults.candidates,
            age: defaults.age,
            climbs: defaults.climbs,
        }
    }
}

impl Default for SessionOptions {
    fn default() -> SessionOptions {
        SessionOptions::new()
    }
}

impl SessionOptions {
    /// The core [`Options`] these settings describe. With neither limit set,
    /// the core's default limit applies, as in the CLI.
    fn to_options(self) -> Options {
        let defaults = Options::default();
        let limit = Limit::from_parts(self.max_shapes, self.max_bytes.map(|bytes| bytes as usize));
        Options {
            opacity: self.opacity,
            limit: limit.unwrap_or(defaults.limit),
            out_width: self.out_width,
            seed: self.seed,
            candidates: self.candidates,
            age: self.age,
            climbs: self.climbs,
            ..defaults
        }
    }
}

/// One cover being built: a [`Model`] plus the JavaScript-facing methods.
#[wasm_bindgen]
pub struct Session {
    /// The model doing the work.
    model: Model,
}

#[wasm_bindgen]
impl Session {
    /// Creates a session for a `width × height` image.
    ///
    /// `rgba` is 4 bytes per pixel, row by row: pass `ImageData.data` (a
    /// `Uint8ClampedArray`) as it is. The image must already be shrunk to
    /// the working size (a few hundred pixels). The alpha channel is
    /// ignored, so draw transparent images onto a background before reading
    /// their pixels.
    ///
    /// # Errors
    /// Throws an `Error` with the core's message if the image or a setting
    /// is invalid.
    #[wasm_bindgen(constructor)]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "wasm-bindgen can only receive a `Uint8ClampedArray` as an owned `Vec`"
    )]
    pub fn new(
        rgba: Clamped<Vec<u8>>,
        width: u32,
        height: u32,
        options: &SessionOptions,
    ) -> Result<Session, JsError> {
        // `?` turns the core's error into a `JsError` with the same message.
        let model = Model::new(&rgba, width, height, options.to_options())?;
        Ok(Session { model })
    }

    /// Adds the next shape and returns it as an `Int32Array`, or returns
    /// `undefined` once a limit is reached (and on every call after that).
    ///
    /// The array is `[kind, ...coordinates, r, g, b, a]`, in working-image
    /// pixels, where each pixel's centre is at its whole-number coordinates.
    /// `kind` is the core's `ShapeKind` number, so future kinds get their own
    /// codes. For a triangle (`kind` 0) that is
    /// `[0, x1, y1, x2, y2, x3, y3, r, g, b, a]`; `a` is the opacity as
    /// 0–255.
    pub fn step(&mut self) -> Option<Vec<i32>> {
        self.model.step().map(|step| encode_step(&step))
    }

    /// Current score: from 0 (identical to the photo) to 1.
    #[must_use]
    pub fn score(&self) -> f64 {
        self.model.score()
    }

    /// Current size of [`Session::svg`], in bytes.
    #[must_use]
    pub fn bytes(&self) -> u32 {
        self.model.bytes() as u32
    }

    /// Number of shapes added so far.
    #[wasm_bindgen(js_name = shapeCount)]
    #[must_use]
    pub fn shape_count(&self) -> u32 {
        self.model.shapes().len() as u32
    }

    /// The background colour as a `Uint8Array` of `[r, g, b]`.
    #[must_use]
    pub fn background(&self) -> Vec<u8> {
        let color = self.model.background();
        vec![color.r, color.g, color.b]
    }

    /// The SVG so far, at [`SessionOptions::out_width`].
    #[must_use]
    pub fn svg(&self) -> String {
        self.model.svg()
    }
}

/// Packs a step's shape and colour into the flat array [`Session::step`]
/// returns. One small array per shape copies quickly from WebAssembly memory
/// and can be posted from the worker to the page as it is.
fn encode_step(step: &StepResult) -> Vec<i32> {
    let kind = step.shape.kind() as i32;
    let mut numbers = match step.shape {
        Shape::Triangle(triangle) => {
            let [a, b, c] = triangle.vertices;
            vec![kind, a.x, a.y, b.x, b.y, c.x, c.y]
        }
    };
    let color = step.color;
    numbers.extend([color.r, color.g, color.b, color.a].map(i32::from));
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;
    use geometrize_core::{Point, Rgba, Triangle};

    #[test]
    fn default_options_are_the_cores() {
        assert_eq!(SessionOptions::new().to_options(), Options::default());
    }

    #[test]
    fn limits_combine_like_the_cli() {
        let both = SessionOptions {
            max_shapes: Some(10),
            max_bytes: Some(5_000),
            ..SessionOptions::new()
        };
        let expected = Limit::Both {
            shapes: 10,
            bytes: 5_000,
        };
        assert_eq!(both.to_options().limit, expected);

        // Only a byte limit, as the page sets it: no hidden shape cap.
        let bytes_only = SessionOptions {
            max_bytes: Some(30_000),
            ..SessionOptions::new()
        };
        assert_eq!(bytes_only.to_options().limit, Limit::Bytes(30_000));
    }

    #[test]
    fn triangle_is_encoded_as_kind_coordinates_and_colour() {
        let step = StepResult {
            shape: Shape::Triangle(Triangle {
                vertices: [Point::new(1, 2), Point::new(-3, 4), Point::new(5, 6)],
            }),
            color: Rgba::new(10, 20, 30, 128),
            score: 0.5,
            bytes: 100,
            shapes: 1,
        };
        assert_eq!(encode_step(&step), [0, 1, 2, -3, 4, 5, 6, 10, 20, 30, 128]);
    }

    #[test]
    fn session_runs_to_its_limit() {
        // A gradient, so every step finds a shape that improves the picture.
        let rgba: Vec<u8> = (0..16u8)
            .flat_map(|y| (0..16u8).flat_map(move |x| [x * 16, y * 16, 128, 255]))
            .collect();
        let options = SessionOptions {
            max_shapes: Some(3),
            candidates: 10,
            age: 2,
            climbs: 1,
            ..SessionOptions::new()
        };
        let mut session = Session::new(Clamped(rgba), 16, 16, &options).unwrap();
        let mut count = 0;
        while let Some(shape) = session.step() {
            assert_eq!(shape.len(), 11);
            count += 1;
        }
        assert_eq!(count, 3);
        assert_eq!(session.shape_count(), 3);
        assert_eq!(session.svg().len(), session.bytes() as usize);
        assert_eq!(session.step(), None);
    }
}
