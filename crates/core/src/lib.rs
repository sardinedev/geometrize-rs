//! Approximate an image with semi-transparent triangles and export it as a
//! compact SVG.
//!
//! This is a trimmed Rust port of [fogleman/primitive](https://github.com/fogleman/primitive).
//! Each [`Model::step`] adds the one shape that most improves the picture:
//! many random candidates are tried, the best ones are refined by hill
//! climbing, and the winner is alpha-blended onto the canvas in the colour
//! that best matches the target underneath it.
//!
//! The crate does no image decoding or resizing. It takes RGBA bytes already
//! shrunk to the working size (around 256 pixels on the longest side works
//! well), so a CLI and a browser can each load images their own way.
//!
//! # Example
//!
//! ```
//! use geometrize_core::{Limit, Model, Options};
//!
//! // A 16×16 test image: a red-to-black gradient.
//! let (width, height) = (16, 16);
//! let rgba: Vec<u8> = (0..width * height)
//!     .flat_map(|i| [(i % width * 16) as u8, 40, 80, 255])
//!     .collect();
//!
//! let options = Options {
//!     limit: Limit::Shapes(5),
//!     candidates: 20,
//!     age: 5,
//!     climbs: 2,
//!     out_width: 320,
//!     ..Options::default()
//! };
//! let mut model = Model::new(&rgba, width, height, options)?;
//! while let Some(step) = model.step() {
//!     println!("score {:.4}, {} bytes", step.score, step.bytes);
//! }
//!
//! let svg = model.svg();
//! assert_eq!(model.shapes().len(), 5);
//! assert_eq!(svg.len(), model.bytes());
//! # Ok::<(), geometrize_core::Error>(())
//! ```
//!
//! # Determinism
//!
//! The same input and [`Options`] (including [`Options::seed`]) always give a
//! byte-identical SVG, with or without the `parallel` feature and whatever the
//! thread count. Native and WebAssembly builds may still differ very
//! occasionally, because `ln`, `cos` and `acos` come from different maths
//! libraries on the two targets.
//!
//! # Features
//!
//! - `parallel`: run each step's independent searches on rayon's thread pool.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod model;
mod pixels;
mod raster;
mod rng;
mod search;
mod shape;
mod svg;

pub use model::{Model, StepResult};
pub use shape::{Shape, ShapeKind, Triangle};

use std::fmt;

/// An 8-bit RGBA colour (not premultiplied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha: 0 is transparent, 255 is opaque.
    pub a: u8,
}

impl Rgba {
    /// Creates a colour from its four channels.
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }
}

/// When to stop adding shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Stop after this many shapes.
    Shapes(u32),
    /// Stop before the SVG would grow beyond this many bytes.
    Bytes(usize),
    /// Stop at whichever of the two limits is reached first.
    Both {
        /// Maximum number of shapes.
        shapes: u32,
        /// Maximum SVG size in bytes.
        bytes: usize,
    },
}

impl Limit {
    /// The shape limit, if there is one.
    pub fn max_shapes(self) -> Option<u32> {
        match self {
            Limit::Shapes(shapes) | Limit::Both { shapes, .. } => Some(shapes),
            Limit::Bytes(_) => None,
        }
    }

    /// The byte limit, if there is one.
    pub fn max_bytes(self) -> Option<usize> {
        match self {
            Limit::Bytes(bytes) | Limit::Both { bytes, .. } => Some(bytes),
            Limit::Shapes(_) => None,
        }
    }
}

/// Settings for a [`Model`]. Start from [`Options::default`] and change what
/// you need.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Opacity of every shape, from 0.01 to 1. Default 0.5.
    ///
    /// It's rounded to two decimals (0.333 becomes 0.33), and that rounded
    /// value is used both to fit the colours and in the SVG's `fill-opacity`,
    /// so the file looks the way it was scored. At 1 the attribute is left out.
    pub opacity: f32,
    /// Shape kinds to use. Each candidate picks one at random. Default
    /// `[Triangle]`.
    pub shapes: Vec<ShapeKind>,
    /// When to stop. Default 500 shapes.
    pub limit: Limit,
    /// Width of the SVG's viewBox. The height follows the image's aspect
    /// ratio. Default 1200.
    pub out_width: u32,
    /// Seed for the random search. The same seed gives the same SVG. Default 0.
    pub seed: u64,
    /// Background colour. `None` (the default) uses the image's average colour.
    pub bg: Option<Rgba>,
    /// Random shapes tried per search. Default 1000.
    pub candidates: u32,
    /// Failed mutations in a row before a hill climb gives up. Default 100.
    pub age: u32,
    /// Independent searches per step; the best one wins. Default 16.
    pub climbs: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            opacity: 0.5,
            shapes: vec![ShapeKind::Triangle],
            limit: Limit::Shapes(500),
            out_width: 1200,
            seed: 0,
            bg: None,
            candidates: 1000,
            age: 100,
            climbs: 16,
        }
    }
}

/// Why a [`Model`] couldn't be created.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The image width or height is 0.
    EmptyImage,
    /// The image is larger than 65536 pixels on a side, or too big to address
    /// on this platform.
    ImageTooLarge {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// The pixel buffer isn't `width × height × 4` bytes long.
    BufferLength {
        /// Bytes needed.
        expected: usize,
        /// Bytes given.
        actual: usize,
    },
    /// The opacity is outside `0.01..=1` (or NaN).
    Opacity(f32),
    /// [`Options::shapes`] is empty.
    NoShapeKinds,
    /// A setting that must be at least 1 is 0.
    Zero(&'static str),
    /// The byte limit is smaller than an SVG with no shapes.
    ByteLimitTooSmall {
        /// The requested limit.
        limit: usize,
        /// Size of the SVG with only the background.
        minimum: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyImage => write!(f, "image width and height must be at least 1"),
            Error::ImageTooLarge { width, height } => write!(
                f,
                "a {width}×{height} image is too large (resize it to a few hundred pixels first)"
            ),
            Error::BufferLength { expected, actual } => write!(
                f,
                "pixel buffer is {actual} bytes, expected {expected} (width × height × 4)"
            ),
            Error::Opacity(opacity) => {
                write!(f, "opacity must be between 0.01 and 1, got {opacity}")
            }
            Error::NoShapeKinds => write!(f, "at least one shape kind is needed"),
            Error::Zero(name) => write!(f, "{name} must be at least 1"),
            Error::ByteLimitTooSmall { limit, minimum } => write!(
                f,
                "max size of {limit} bytes is smaller than an SVG with no shapes ({minimum} bytes)"
            ),
        }
    }
}

impl std::error::Error for Error {}
