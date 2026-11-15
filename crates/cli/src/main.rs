//! `geometrize`: turn a photo into a compact SVG of semi-transparent
//! triangles.
//!
//! This binary is a thin layer over `geometrize-core`. It reads and shrinks
//! the photo, turns the command line into [`Options`], runs the model while
//! reporting progress on stderr, and writes the SVG. Run `geometrize --help`
//! for the flags.

mod args;

use std::fmt;
use std::io::{self, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use geometrize_core::{Error, Limit, Model, Options, Rgba, StepResult};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};

use args::{Args, Command, Output};

fn main() -> ExitCode {
    let command = match args::parse_from(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => return usage_error(&error),
    };
    match command {
        Command::Help => {
            print!("{}", args::HELP);
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("geometrize {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Run(args) => match run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error.is_usage() => usage_error(&error),
            Err(error) => {
                eprintln!("geometrize: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Reports a problem with the command line itself and exits with code 2.
fn usage_error(error: &dyn fmt::Display) -> ExitCode {
    eprintln!("geometrize: {error}");
    eprintln!("Try 'geometrize --help' for more information.");
    ExitCode::from(2)
}

/// Something that stopped a run, with enough context to explain it.
#[derive(Debug)]
enum CliError {
    /// The photo couldn't be opened or decoded.
    Read {
        path: PathBuf,
        source: image::ImageError,
    },
    /// The core rejected the settings or the image.
    Model(Error),
    /// The thread pool for `--threads` couldn't be created.
    Threads(rayon::ThreadPoolBuildError),
    /// The SVG couldn't be written.
    Write { target: String, source: io::Error },
}

impl CliError {
    /// Whether the fix is a different flag value, so the error should be
    /// reported like any other command-line mistake (exit code 2).
    fn is_usage(&self) -> bool {
        match self {
            CliError::Model(error) => match error {
                Error::Opacity(_)
                | Error::NoShapeKinds
                | Error::Zero(_)
                | Error::ByteLimitTooSmall { .. }
                | Error::ImageTooLarge { .. } => true,
                Error::EmptyImage | Error::BufferLength { .. } => false,
            },
            CliError::Read { .. } | CliError::Threads(_) | CliError::Write { .. } => false,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Read { path, source } => {
                write!(f, "can't read {}: {source}", path.display())
            }
            CliError::Model(error) => write!(f, "{}", explain_core_error(error)),
            CliError::Threads(error) => write!(f, "can't start the thread pool: {error}"),
            CliError::Write { target, source } => write!(f, "can't write {target}: {source}"),
        }
    }
}

/// Rewords a core error in terms of the flag that causes it. The core talks
/// about settings (`candidates`, `output width`); here they are `--candidates`
/// and `--width`.
fn explain_core_error(error: &Error) -> String {
    match error {
        Error::Opacity(opacity) => format!("--opacity must be between 0.01 and 1, got {opacity}"),
        Error::NoShapeKinds => "at least one --shape is needed".to_string(),
        Error::Zero(setting) => {
            let flag = match *setting {
                "output width" => "--width",
                "candidates" => "--candidates",
                "climbs" => "--climbs",
                other => other,
            };
            format!("{flag} must be at least 1")
        }
        Error::ByteLimitTooSmall { limit, minimum } => format!(
            "--max-size {limit} is too small: an SVG with no shapes is already {minimum} bytes"
        ),
        Error::ImageTooLarge { width, height } => {
            format!("a {width}×{height} working image is too large; use a smaller --work-size")
        }
        Error::EmptyImage | Error::BufferLength { .. } => error.to_string(),
    }
}

/// Makes one SVG from the parsed arguments.
fn run(args: Args) -> Result<(), CliError> {
    let seed = args.seed.unwrap_or_else(seed_from_clock);
    let options = Options {
        seed,
        ..args.options
    };
    // Check the flags before the (possibly slow) photo decoding, so a typo
    // fails straight away.
    options.validate().map_err(CliError::Model)?;

    // The core treats every image as opaque, so transparent pixels are
    // blended onto the background first: `--bg` if given, otherwise white.
    let background = options.bg.unwrap_or(Rgba::new(255, 255, 255, 255));
    let photo = load_image(&args.input, args.work_size, background)?;
    let (width, height) = photo.dimensions();
    let limit = options.limit;
    let mut model = Model::new(photo.as_raw(), width, height, options).map_err(CliError::Model)?;

    let mut progress = Progress::start(args.quiet, limit, seed);
    match args.threads {
        Some(threads) => rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(CliError::Threads)?
            .install(|| add_shapes(&mut model, &mut progress)),
        None => add_shapes(&mut model, &mut progress),
    }
    // Finish the live progress line before anything else is printed (an
    // error, or the SVG itself with `-o -`).
    progress.end_line();

    write_output(&args.output, &model.svg())?;
    progress.finish(&model, &args.output);
    Ok(())
}

/// Adds shapes until the model reaches a limit, reporting each one.
fn add_shapes(model: &mut Model, progress: &mut Progress) {
    while let Some(step) = model.step() {
        progress.step(model.shapes().len(), &step);
    }
}

/// Opens and decodes a photo, turns it upright, blends any transparency onto
/// `background`, and shrinks it so its longest side is at most `work_size`
/// pixels.
fn load_image(path: &Path, work_size: u32, background: Rgba) -> Result<RgbaImage, CliError> {
    let read_error = |source: image::ImageError| CliError::Read {
        path: path.to_path_buf(),
        source,
    };
    let mut decoder = ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|error| read_error(error.into()))?
        .into_decoder()
        .map_err(read_error)?;
    // Phones (and some cameras) store photos sideways with an EXIF note
    // saying how to rotate them; apply it so the cover isn't sideways.
    let orientation = decoder.orientation().map_err(read_error)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(read_error)?;
    image.apply_orientation(orientation);

    // Blend away transparency *before* shrinking. Shrinking averages each
    // channel separately, so transparent pixels (usually stored as black)
    // would otherwise darken the edges next to them. Only images with an
    // alpha channel (PNGs, in practice) need this; JPEGs skip a full-size
    // copy and loop that would change nothing.
    if image.color().has_alpha() {
        let mut rgba = image.into_rgba8();
        flatten_onto(&mut rgba, background);
        image = DynamicImage::ImageRgba8(rgba);
    }

    // Only ever shrink: enlarging a small photo adds no detail, just time.
    if image.width().max(image.height()) > work_size {
        image = image.thumbnail(work_size, work_size);
    }
    Ok(image.into_rgba8())
}

/// Blends every partly or fully transparent pixel onto `background`, making
/// the image opaque. Without this, transparent pixels (usually stored as
/// black) would come out black.
fn flatten_onto(image: &mut RgbaImage, background: Rgba) {
    for pixel in image.pixels_mut() {
        let [r, g, b, alpha] = pixel.0;
        if alpha == 255 {
            continue;
        }
        // `color × α + background × (1 − α)` in 8-bit, rounded.
        let alpha = u32::from(alpha);
        let blend = |color: u8, back: u8| {
            ((u32::from(color) * alpha + u32::from(back) * (255 - alpha) + 127) / 255) as u8
        };
        pixel.0 = [
            blend(r, background.r),
            blend(g, background.g),
            blend(b, background.b),
            255,
        ];
    }
}

/// A seed for runs without `--seed`: milliseconds since 1970. It's printed,
/// so the run can be repeated exactly.
fn seed_from_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes the SVG to its file or to stdout.
fn write_output(output: &Output, svg: &str) -> Result<(), CliError> {
    let result = match output {
        Output::Stdout => {
            // Flush explicitly: stdout is buffered, and an error while the
            // buffer is flushed at exit would otherwise be silently ignored.
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(svg.as_bytes())
                .and_then(|()| stdout.flush())
        }
        Output::File(path) => std::fs::write(path, svg),
    };
    result.map_err(|source| CliError::Write {
        target: output.to_string(),
        source,
    })
}

/// Progress messages on stderr.
struct Progress {
    /// False with `--quiet`: print nothing.
    enabled: bool,
    /// True when stderr is a terminal: keep rewriting one line in place.
    /// Otherwise (a log file, CI) print a line every [`Progress::LOG_EVERY`]
    /// shapes so the log stays short.
    live: bool,
    /// True while a live line has been printed but not yet ended.
    line_open: bool,
    /// The run's limits, shown as "120/500" and "12.3/30.0 KB".
    limit: Limit,
    /// When the search started, for the shapes-per-second rate.
    started: Instant,
}

impl Progress {
    /// How often to print a progress line when stderr isn't a terminal.
    const LOG_EVERY: usize = 50;

    /// Prints the seed and starts the clock.
    fn start(quiet: bool, limit: Limit, seed: u64) -> Progress {
        if !quiet {
            eprintln!("seed {seed} (pass --seed {seed} to reproduce)");
        }
        Progress {
            enabled: !quiet,
            live: io::stderr().is_terminal(),
            line_open: false,
            limit,
            started: Instant::now(),
        }
    }

    /// Reports that shape number `shapes` was added.
    fn step(&mut self, shapes: usize, step: &StepResult) {
        if !self.enabled {
            return;
        }
        let line = self.line(shapes, step);
        if self.live {
            // `\r` returns to the start of the line; `\x1b[K` clears what's
            // left of the previous, possibly longer, line.
            eprint!("\r{line}\x1b[K");
            self.line_open = true;
        } else if shapes.is_multiple_of(Self::LOG_EVERY) {
            eprintln!("{line}");
        }
    }

    /// One progress line: `shape 120/500  score 0.0712  12.3/30.0 KB  85 shapes/s`.
    fn line(&self, shapes: usize, step: &StepResult) -> String {
        let mut line = format!("shape {shapes}");
        if let Some(max) = self.limit.max_shapes() {
            line.push_str(&format!("/{max}"));
        }
        line.push_str(&format!(
            "  score {:.4}  {:.1}",
            step.score,
            kilobytes(step.bytes)
        ));
        if let Some(max) = self.limit.max_bytes() {
            line.push_str(&format!("/{:.1}", kilobytes(max)));
        }
        let rate = shapes as f64 / seconds(self.started.elapsed());
        line.push_str(&format!(" KB  {rate:.0} shapes/s"));
        line
    }

    /// Ends the live progress line, if one is showing.
    fn end_line(&mut self) {
        if self.line_open {
            eprintln!();
            self.line_open = false;
        }
    }

    /// Prints the summary once the SVG is written.
    fn finish(&self, model: &Model, output: &Output) {
        if !self.enabled {
            return;
        }
        eprintln!(
            "done: {} shapes, score {:.4}, {} bytes in {:.1} s -> {output}",
            model.shapes().len(),
            model.score(),
            model.bytes(),
            seconds(self.started.elapsed()),
        );
    }
}

/// Bytes as kilobytes of 1000 bytes, the unit `--max-size 30k` uses.
fn kilobytes(bytes: usize) -> f64 {
    bytes as f64 / 1000.0
}

/// A duration in seconds, never zero (so rates can't divide by zero).
fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64().max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_is_blended_before_shrinking() {
        // A checkerboard of opaque red and transparent black. Blended onto
        // white, every pixel is red or white, so red is 255 everywhere, and
        // stays 255 after shrinking. Shrinking first would average in the
        // transparent black and darken it.
        let path =
            std::env::temp_dir().join(format!("geometrize-flatten-{}.png", std::process::id()));
        image::RgbaImage::from_fn(512, 256, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        })
        .save(&path)
        .unwrap();

        let white = Rgba::new(255, 255, 255, 255);
        let loaded = load_image(&path, 256, white);
        std::fs::remove_file(&path).unwrap();
        let image = loaded.expect("the test image loads");

        assert_eq!(image.dimensions(), (256, 128));
        assert!(
            image
                .pixels()
                .all(|pixel| pixel.0[0] == 255 && pixel.0[3] == 255)
        );
    }

    #[test]
    fn transparency_is_blended_onto_the_background() {
        let white = Rgba::new(255, 255, 255, 255);
        let mut image = RgbaImage::from_vec(
            3,
            1,
            vec![
                255, 0, 0, 255, // opaque red: unchanged
                255, 0, 0, 128, // half-transparent red: pink
                0, 0, 0, 0, // fully transparent: the background
            ],
        )
        .unwrap();
        flatten_onto(&mut image, white);
        assert_eq!(
            image.into_raw(),
            [255, 0, 0, 255, 255, 127, 127, 255, 255, 255, 255, 255]
        );
    }
}
