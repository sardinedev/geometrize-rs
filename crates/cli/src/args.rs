//! Command-line arguments: the help text, parsing with `lexopt`, and parsers
//! for sizes, colours and shape names.
//!
//! Only formats are checked here (is it a number, a size, a colour, a known
//! shape?). Ranges such as "opacity between 0.01 and 1" are left to
//! `geometrize_core::Model::new`, so the rules live in one place.

use std::ffi::OsString;
use std::fmt::{self, Display};
use std::path::PathBuf;
use std::str::FromStr;

use geometrize_core::{Limit, Options, Rgba, ShapeKind};

/// Text printed by `--help`.
pub const HELP: &str = "\
geometrize: turn a photo into a compact SVG of semi-transparent triangles

Usage: geometrize -i <photo> -o <cover.svg> [options]

Input and output:
  -i, --input <file>      JPEG or PNG photo to read (required)
  -o, --output <file>     SVG file to write, or - for stdout (required)

When to stop (with neither, 500 shapes; with only --max-size, no shape limit;
with both, whichever comes first):
  -n, --shapes <n>        Stop after n shapes
      --max-size <size>   Keep the SVG at or under this size: 30000, 30k, 1.5m
                          (k = 1000 bytes, m = 1,000,000 bytes)

Look:
      --opacity <0.01-1>  Opacity of every shape [default: 0.5]
      --shape <kind>      Shape to use; repeat for several [default: triangle]
      --width <px>        Width of the SVG's viewBox [default: 1200]
      --bg <rrggbb>       Background colour, e.g. ff8800 or f80 (in a shell,
                          quote a leading #: '#f80') [default: the photo's
                          average]. Transparent pixels are blended onto it,
                          or onto white when it isn't given

Search:
      --seed <n>          Random seed; the same seed gives the same SVG
                          [default: picked from the clock and printed]
      --work-size <px>    Shrink the photo to this many pixels on its long side
                          before searching [default: 256]
      --candidates <n>    Random shapes tried per search [default: 1000]
      --age <n>           Failed tweaks in a row before a search stops [default: 100]
      --climbs <n>        Independent searches per shape [default: 16]
  -j, --threads <n>       Threads to use [default: all cores]

Other:
  -q, --quiet             Print nothing except errors
  -h, --help              Print this help
  -V, --version           Print the version
";

/// Where the finished SVG goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Write to this file.
    File(PathBuf),
    /// Write to standard output (`-o -`).
    Stdout,
}

impl Display for Output {
    /// The file path, or `stdout`, as shown in messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Output::File(path) => write!(f, "{}", path.display()),
            Output::Stdout => write!(f, "stdout"),
        }
    }
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Make an SVG.
    Run(Args),
    /// Print the help text.
    Help,
    /// Print the version.
    Version,
}

/// Everything needed to make one SVG.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// Photo to read.
    pub input: PathBuf,
    /// Where to write the SVG.
    pub output: Output,
    /// Longest side of the working image, in pixels.
    pub work_size: u32,
    /// Size of the thread pool; `None` uses every core.
    pub threads: Option<usize>,
    /// Print nothing except errors.
    pub quiet: bool,
    /// The `--seed` value; `None` means pick one from the clock.
    pub seed: Option<u64>,
    /// Settings for the model. Its `seed` is filled in later from
    /// [`Args::seed`].
    pub options: Options,
}

/// Parses the arguments, not including the program name.
///
/// # Errors
/// Returns a message for an unknown flag, a missing value or a malformed one.
pub fn parse_from<I>(arguments: I) -> Result<Command, lexopt::Error>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    use lexopt::prelude::*;

    let mut input = None;
    let mut output = None;
    let mut max_shapes = None;
    let mut max_bytes = None;
    let mut shape_kinds = Vec::new();
    let mut work_size = 256;
    let mut threads = None;
    let mut quiet = false;
    let mut seed = None;
    let mut options = Options::default();

    let mut parser = lexopt::Parser::from_args(arguments);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('i') | Long("input") => input = Some(PathBuf::from(parser.value()?)),
            Short('o') | Long("output") => {
                let value = parser.value()?;
                output = Some(if value == "-" {
                    Output::Stdout
                } else {
                    Output::File(PathBuf::from(value))
                });
            }
            Short('n') | Long("shapes") => {
                max_shapes = Some(parse_value(&mut parser, "--shapes", parse_number)?);
            }
            Long("max-size") => {
                max_bytes = Some(parse_value(&mut parser, "--max-size", parse_size)?)
            }
            Long("opacity") => {
                options.opacity = parse_value(&mut parser, "--opacity", parse_number)?
            }
            Long("shape") => shape_kinds.push(parse_value(&mut parser, "--shape", parse_shape)?),
            Long("width") => options.out_width = parse_value(&mut parser, "--width", parse_number)?,
            Long("bg") => options.bg = Some(parse_value(&mut parser, "--bg", parse_color)?),
            Long("seed") => seed = Some(parse_value(&mut parser, "--seed", parse_number)?),
            Long("work-size") => work_size = parse_value(&mut parser, "--work-size", parse_number)?,
            Long("candidates") => {
                options.candidates = parse_value(&mut parser, "--candidates", parse_number)?;
            }
            Long("age") => options.age = parse_value(&mut parser, "--age", parse_number)?,
            Long("climbs") => options.climbs = parse_value(&mut parser, "--climbs", parse_number)?,
            Short('j') | Long("threads") => {
                threads = Some(parse_value(&mut parser, "--threads", parse_number)?);
            }
            Short('q') | Long("quiet") => quiet = true,
            Short('h') | Long("help") => return Ok(Command::Help),
            Short('V') | Long("version") => return Ok(Command::Version),
            _ => return Err(arg.unexpected()),
        }
    }

    let input = input.ok_or("missing -i/--input <file>")?;
    let output = output.ok_or("missing -o/--output <file> (use - for stdout)")?;
    // These two can't be checked by the core: it never sees them.
    if work_size == 0 {
        return Err("--work-size must be at least 1".into());
    }
    if threads == Some(0) {
        return Err("--threads must be at least 1".into());
    }
    if let Some(limit) = Limit::from_parts(max_shapes, max_bytes) {
        options.limit = limit;
    }
    if !shape_kinds.is_empty() {
        options.shapes = shape_kinds;
    }

    Ok(Command::Run(Args {
        input,
        output,
        work_size,
        threads,
        quiet,
        seed,
        options,
    }))
}

/// Reads the value that follows `flag` and parses it, naming the flag and
/// the value in any error.
fn parse_value<T>(
    parser: &mut lexopt::Parser,
    flag: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, lexopt::Error> {
    let raw = parser.value()?;
    let Some(text) = raw.to_str() else {
        return Err(format!("the value for {flag} isn't valid UTF-8").into());
    };
    parse(text).map_err(|message| format!("invalid value '{text}' for {flag}: {message}").into())
}

/// Parses any number type, keeping the standard library's error message.
fn parse_number<T>(text: &str) -> Result<T, String>
where
    T: FromStr,
    T::Err: Display,
{
    text.parse().map_err(|error: T::Err| error.to_string())
}

/// Parses a size in bytes: a number with an optional `k` (1000) or `m`
/// (1,000,000) suffix, in either case. `30000`, `30k` and `0.03m` are all
/// 30,000 bytes. Fractions are rounded to whole bytes.
pub fn parse_size(text: &str) -> Result<usize, String> {
    const EXPECTED: &str = "expected a size like 30000, 30k or 1.5m";
    let lowercase = text.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(number) = lowercase.strip_suffix('k') {
        (number, 1_000.0)
    } else if let Some(number) = lowercase.strip_suffix('m') {
        (number, 1_000_000.0)
    } else {
        (lowercase.as_str(), 1.0)
    };
    let value: f64 = number.parse().map_err(|_| EXPECTED)?;
    let bytes = (value * multiplier).round();
    // The range check also rejects NaN and infinity.
    if !(1.0..=usize::MAX as f64).contains(&bytes) {
        return Err(format!("{EXPECTED}, of at least 1 byte"));
    }
    Ok(bytes as usize)
}

/// Parses a colour written as `rrggbb` or `rgb` hex digits, with an optional
/// leading `#`.
pub fn parse_color(text: &str) -> Result<Rgba, String> {
    // `#` is left out of the example: unquoted, a shell treats it as the
    // start of a comment.
    const EXPECTED: &str = "expected a hex colour like ff8800 or f80";
    let hex = text.strip_prefix('#').unwrap_or(text);
    let mut digits = Vec::new();
    for character in hex.chars() {
        let digit = character.to_digit(16).ok_or(EXPECTED)?;
        digits.push(digit as u8);
    }
    let [r, g, b] = match digits[..] {
        // Short form: each digit is doubled, so `f80` means `ff8800`.
        [r, g, b] => [r * 17, g * 17, b * 17],
        [r1, r2, g1, g2, b1, b2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2],
        _ => return Err(EXPECTED.to_string()),
    };
    Ok(Rgba::new(r, g, b, 255))
}

/// Parses a shape name, listing the available ones if it's unknown.
pub fn parse_shape(text: &str) -> Result<ShapeKind, String> {
    ShapeKind::from_name(text).ok_or_else(|| {
        let names: Vec<&str> = ShapeKind::ALL.iter().map(|kind| kind.name()).collect();
        format!("unknown shape (available: {})", names.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses arguments written as one string, expecting a run.
    fn run_args(line: &str) -> Args {
        match parse_from(line.split_whitespace()) {
            Ok(Command::Run(args)) => args,
            other => panic!("expected a run for '{line}', got {other:?}"),
        }
    }

    /// Parses arguments written as one string, expecting an error message.
    fn error_for(line: &str) -> String {
        match parse_from(line.split_whitespace()) {
            Err(error) => error.to_string(),
            Ok(command) => panic!("expected an error for '{line}', got {command:?}"),
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("30000"), Ok(30_000));
        assert_eq!(parse_size("30k"), Ok(30_000));
        assert_eq!(parse_size("30K"), Ok(30_000));
        assert_eq!(parse_size("1.5m"), Ok(1_500_000));
        assert_eq!(parse_size("0.5k"), Ok(500));
        for bad in ["", "abc", "-1", "0", "30kb", "1e99", "inf", "nan"] {
            assert!(parse_size(bad).is_err(), "'{bad}' should be rejected");
        }
    }

    #[test]
    fn colors() {
        let orange = Rgba::new(0xff, 0x88, 0x00, 255);
        assert_eq!(parse_color("ff8800"), Ok(orange));
        assert_eq!(parse_color("#ff8800"), Ok(orange));
        assert_eq!(parse_color("#f80"), Ok(orange));
        assert_eq!(parse_color("#FF8800"), Ok(orange));
        for bad in ["", "#ff88", "zzzzzz", "#ff88000"] {
            assert!(parse_color(bad).is_err(), "'{bad}' should be rejected");
        }
    }

    #[test]
    fn defaults() {
        let args = run_args("-i photo.jpg -o cover.svg");
        assert_eq!(args.input, PathBuf::from("photo.jpg"));
        assert_eq!(args.output, Output::File(PathBuf::from("cover.svg")));
        assert_eq!(args.work_size, 256);
        assert_eq!(args.threads, None);
        assert_eq!(args.seed, None);
        assert!(!args.quiet);
        assert_eq!(args.options, Options::default());
    }

    #[test]
    fn every_flag() {
        let args = run_args(
            "--input a.png --output - -n 300 --max-size 30k --opacity 0.4 --shape triangle \
             --shape triangle --width 800 --bg #102030 --seed 7 --work-size 128 \
             --candidates 50 --age 20 --climbs 4 -j 2 -q",
        );
        assert_eq!(args.output, Output::Stdout);
        assert_eq!(args.work_size, 128);
        assert_eq!(args.threads, Some(2));
        assert_eq!(args.seed, Some(7));
        assert!(args.quiet);
        let options = args.options;
        assert_eq!(
            options.limit,
            Limit::Both {
                shapes: 300,
                bytes: 30_000
            }
        );
        assert_eq!(options.opacity, 0.4);
        assert_eq!(options.shapes, [ShapeKind::Triangle, ShapeKind::Triangle]);
        assert_eq!(options.out_width, 800);
        assert_eq!(options.bg, Some(Rgba::new(0x10, 0x20, 0x30, 255)));
        assert_eq!(
            (options.candidates, options.age, options.climbs),
            (50, 20, 4)
        );
    }

    #[test]
    fn size_alone_replaces_the_default_shape_limit() {
        let args = run_args("-i a.png -o b.svg --max-size 2k");
        assert_eq!(args.options.limit, Limit::Bytes(2000));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_from(["--help"]).unwrap(), Command::Help);
        assert_eq!(parse_from(["-V"]).unwrap(), Command::Version);
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(error_for("-o b.svg").contains("--input"));
        assert!(error_for("-i a.png").contains("--output"));
        assert!(error_for("-i a.png -o b.svg --shape hexagon").contains("available: triangle"));
        assert!(error_for("-i a.png -o b.svg --seed abc").contains("--seed"));
        assert!(error_for("-i a.png -o b.svg --work-size 0").contains("--work-size"));
        assert!(error_for("-i a.png -o b.svg -j 0").contains("--threads"));
        assert!(error_for("-i a.png -o b.svg --frobnicate").contains("--frobnicate"));
    }
}
