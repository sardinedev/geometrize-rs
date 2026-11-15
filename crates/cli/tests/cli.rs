//! End-to-end tests: run the real `geometrize` binary on generated images.
//!
//! Every run uses tiny search settings so the whole file takes seconds.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Search settings small enough to keep each run fast.
const FAST: &[&str] = &["--candidates", "20", "--age", "5", "--climbs", "2"];

/// A scratch directory for one test, deleted when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> TempDir {
        let path =
            std::env::temp_dir().join(format!("geometrize-cli-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes a `width × height` PNG with a gradient and a bright square.
fn write_photo(path: &Path, width: u32, height: u32) {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let in_square =
            (width / 3..2 * width / 3).contains(&x) && (height / 3..2 * height / 3).contains(&y);
        if in_square {
            image::Rgb([250, 230, 40])
        } else {
            image::Rgb([(x * 255 / width) as u8, (y * 255 / height) as u8, 120])
        }
    });
    image.save(path).unwrap();
}

/// A `geometrize` command with these arguments, ready to run.
fn command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_geometrize"));
    command.args(args);
    command
}

/// Runs `geometrize` with these arguments.
fn geometrize(args: &[&str]) -> Output {
    command(args).output().unwrap()
}

/// The arguments for turning `photo` into `svg` with the fast settings, plus
/// `extra`.
fn svg_args<'a>(photo: &'a Path, svg: &'a Path, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["-i", photo.to_str().unwrap(), "-o", svg.to_str().unwrap()];
    args.extend_from_slice(FAST);
    args.extend_from_slice(extra);
    args
}

/// Runs `geometrize` on `photo` with the fast settings plus `extra`, writing
/// to `svg`, and returns the result.
fn make_svg(photo: &Path, svg: &Path, extra: &[&str]) -> Output {
    geometrize(&svg_args(photo, svg, extra))
}

/// The usual setup: a scratch directory holding a 64×48 `photo.png`, plus
/// the path of a `cover.svg` next to it (not yet written). Keep the returned
/// `TempDir` in a named binding (`_dir` is fine): the directory is deleted
/// when it's dropped.
fn scratch_with_photo(test: &str) -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new(test);
    let (photo, svg) = (dir.join("photo.png"), dir.join("cover.svg"));
    write_photo(&photo, 64, 48);
    (dir, photo, svg)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn writes_a_valid_svg() {
    let (_dir, photo, svg) = scratch_with_photo("valid");

    let output = make_svg(&photo, &svg, &["-n", "5", "--seed", "1"]);
    assert!(output.status.success(), "{}", stderr(&output));

    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(
        text.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1200 900\">")
    );
    assert_eq!(text.matches("<path").count(), 6, "background + 5 shapes");
    assert!(text.ends_with("</svg>"));
    assert!(stderr(&output).contains("done: 5 shapes"));
}

#[test]
fn same_seed_and_any_thread_count_give_the_same_file() {
    let (dir, photo, _) = scratch_with_photo("seed");

    let mut files = Vec::new();
    for (i, threads) in ["1", "4", "4"].into_iter().enumerate() {
        let svg = dir.join(&format!("cover{i}.svg"));
        let output = make_svg(&photo, &svg, &["-n", "10", "--seed", "42", "-j", threads]);
        assert!(output.status.success(), "{}", stderr(&output));
        files.push(std::fs::read_to_string(svg).unwrap());
    }
    assert_eq!(files[0], files[1], "1 thread vs 4");
    assert_eq!(files[1], files[2], "repeat run");
}

#[test]
fn printed_seed_reproduces_the_run() {
    let dir = TempDir::new("printed-seed");
    let (photo, first, second) = (dir.join("photo.png"), dir.join("a.svg"), dir.join("b.svg"));
    write_photo(&photo, 64, 48);

    let output = make_svg(&photo, &first, &["-n", "5"]);
    let log = stderr(&output);
    let seed = log
        .strip_prefix("seed ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no seed line in: {log}"));

    assert!(
        make_svg(&photo, &second, &["-n", "5", "--seed", seed])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(first).unwrap(),
        std::fs::read_to_string(second).unwrap()
    );
}

#[test]
fn dash_writes_the_svg_to_stdout() {
    let (_dir, photo, _) = scratch_with_photo("stdout");

    let output = make_svg(&photo, Path::new("-"), &["-n", "3", "--seed", "1"]);
    assert!(output.status.success());
    let svg = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    assert!(
        !stderr(&output).contains("<svg"),
        "progress stays on stderr"
    );
}

#[test]
fn max_size_is_respected() {
    let (_dir, photo, svg) = scratch_with_photo("max-size");

    let output = make_svg(&photo, &svg, &["--max-size", "2k", "--seed", "1"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let size = std::fs::metadata(&svg).unwrap().len();
    assert!(size <= 2000, "{size} bytes");
    assert!(size > 1500, "stopped early at {size} bytes");
}

#[test]
fn quiet_prints_nothing() {
    let (_dir, photo, svg) = scratch_with_photo("quiet");

    let output = make_svg(&photo, &svg, &["-n", "3", "-q"]);
    assert!(output.status.success());
    assert_eq!(stderr(&output), "");
}

#[test]
fn large_photos_are_shrunk_keeping_their_shape() {
    let dir = TempDir::new("shrink");
    let (photo, svg) = (dir.join("photo.png"), dir.join("cover.svg"));
    // 512×256 shrinks to the 256×128 working size; 600 wide keeps 2:1.
    write_photo(&photo, 512, 256);

    let output = make_svg(&photo, &svg, &["-n", "2", "--width", "600"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = std::fs::read_to_string(svg).unwrap();
    assert!(text.contains("viewBox=\"0 0 600 300\""), "{}", &text[..80]);
}

#[test]
fn errors_explain_themselves() {
    let (dir, _, svg) = scratch_with_photo("errors");

    let missing = make_svg(&dir.join("nope.png"), &svg, &[]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("geometrize: can't read"));

    let unknown_flag = geometrize(&["--frobnicate"]);
    assert_eq!(unknown_flag.status.code(), Some(2));
    assert!(stderr(&unknown_flag).contains("--help"));
    assert!(!svg.exists(), "nothing is written after an error");
}

#[test]
fn bad_settings_name_their_flag_and_fail_before_reading_the_photo() {
    let dir = TempDir::new("bad-settings");
    // The photo doesn't exist: a settings error must be reported first.
    let (photo, svg) = (dir.join("nope.png"), dir.join("cover.svg"));
    let cases: [(&[&str], &str); 4] = [
        (&["--opacity", "2"], "--opacity must be between 0.01 and 1"),
        (&["--candidates", "0"], "--candidates must be at least 1"),
        (&["--width", "0"], "--width must be at least 1"),
        (&["--climbs", "0"], "--climbs must be at least 1"),
    ];
    for (flags, message) in cases {
        let output = make_svg(&photo, &svg, flags);
        assert_eq!(output.status.code(), Some(2), "{flags:?}");
        let log = stderr(&output);
        assert!(log.contains(message), "{flags:?}: {log}");
        assert!(log.contains("--help"), "{flags:?}: {log}");
    }
}

#[test]
fn too_small_max_size_names_the_flag() {
    let (_dir, photo, svg) = scratch_with_photo("small-max-size");

    let output = make_svg(&photo, &svg, &["--max-size", "50"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--max-size 50 is too small"));
}

#[test]
fn transparent_pixels_are_blended_onto_the_background() {
    let dir = TempDir::new("transparent");
    let (photo, svg) = (dir.join("logo.png"), dir.join("cover.svg"));
    // Fully transparent black, the way most PNGs store "nothing here".
    image::RgbaImage::from_pixel(64, 48, image::Rgba([0, 0, 0, 0]))
        .save(&photo)
        .unwrap();

    let cases: [(&[&str], &str); 2] = [
        // Without --bg, transparency becomes white (not black).
        (&["-n", "1"], "#fff"),
        // With --bg, it becomes that colour.
        (&["-n", "1", "--bg", "102030"], "#102030"),
    ];
    for (flags, fill) in cases {
        assert!(make_svg(&photo, &svg, flags).status.success(), "{flags:?}");
        let text = std::fs::read_to_string(&svg).unwrap();
        let background = format!("<path fill=\"{fill}\" d=\"M0 0h1200v900H0z\"/>");
        assert!(text.contains(&background), "{flags:?}: {text}");
    }
}

#[test]
fn help_and_version() {
    let help = geometrize(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: geometrize"));

    let version = geometrize(&["--version"]);
    let text = String::from_utf8_lossy(&version.stdout).into_owned();
    assert_eq!(
        text.trim(),
        format!("geometrize {}", env!("CARGO_PKG_VERSION"))
    );
}

/// `/dev/full` fails every write, which is how a full disk or a closed pipe
/// looks to the program. It exists on Linux (the CI runners) but not macOS.
#[cfg(target_os = "linux")]
#[test]
fn failed_stdout_write_is_an_error() {
    let (_dir, photo, _) = scratch_with_photo("dev-full");

    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let output = command(&svg_args(&photo, Path::new("-"), &["-n", "3"]))
        .stdout(full)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("can't write stdout"));
}

/// The real photo in `fixtures/` (see `fixtures/README.md`).
const BIG_BEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/big_ben.jpg");

/// The score printed on the `done:` line.
fn reported_score(log: &str) -> f64 {
    let done = log
        .lines()
        .find(|line| line.starts_with("done:"))
        .expect("a done line");
    let after = done.split("score ").nth(1).expect("a score");
    after.split(',').next().unwrap().parse().unwrap()
}

#[test]
fn real_photo_decodes_fits_the_budget_and_gets_close() {
    let dir = TempDir::new("big-ben");
    let (first, second) = (dir.join("a.svg"), dir.join("b.svg"));
    let photo = Path::new(BIG_BEN);
    // The byte budget is what stops this run (about 55 triangles fit).
    let settings = ["-n", "200", "--max-size", "4k", "--seed", "1"];

    let output = make_svg(photo, &first, &settings);
    assert!(output.status.success(), "{}", stderr(&output));
    let svg = std::fs::read_to_string(&first).unwrap();

    // A portrait photo: taller than wide once scaled to 1200 wide.
    assert!(
        svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1200 1499\">")
    );
    assert!(svg.len() <= 4000, "{} bytes", svg.len());
    assert!(svg.len() > 3800, "stopped early at {} bytes", svg.len());

    // With no shapes the score is about 0.23 (just the average colour). A
    // healthy search gets well below that even with these small settings; a
    // regression that makes covers worse would show up here.
    let score = reported_score(&stderr(&output));
    assert!(
        score < SCORE_LIMIT,
        "score {score} is not below {SCORE_LIMIT}"
    );

    assert!(make_svg(photo, &second, &settings).status.success());
    assert_eq!(
        svg,
        std::fs::read_to_string(&second).unwrap(),
        "same seed, same file"
    );
}

/// Highest acceptable score for the real-photo test. Measured runs with
/// these settings score 0.074–0.078 across seeds (0.23 with no shapes), so
/// this leaves room for seed and platform differences while still catching a
/// search that has become noticeably worse.
const SCORE_LIMIT: f64 = 0.09;
