//! Benchmarks for the public `Model` API.
//!
//! Run them with `cargo bench -p geometrize-core`, or through CodSpeed with
//! `cargo codspeed build -p geometrize-core && cargo codspeed run`.
//!
//! The image is a synthetic 128×85 "sun in the sky" picture, about half the
//! usual working size, and the search settings are scaled down so a single
//! iteration stays in the millisecond range.

use divan::{Bencher, black_box};
use geometrize_core::{Limit, Model, Options};

fn main() {
    divan::main();
}

const WIDTH: u32 = 128;
const HEIGHT: u32 = 85;

/// A sky gradient with a bright disc in the middle, so the search has real
/// structure to find.
fn synthetic_image(width: u32, height: u32) -> Vec<u8> {
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let radius = f64::from(height) / 4.0;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            let pixel = if (dx * dx + dy * dy).sqrt() < radius {
                [255, 200, 60]
            } else {
                let t = y * 255 / height;
                [(40 + t / 4) as u8, (80 + t / 6) as u8, (200 - t / 4) as u8]
            };
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    rgba
}

/// Reduced search settings that keep the shape of the default workload.
fn options(limit: Limit) -> Options {
    Options {
        limit,
        candidates: 100,
        age: 20,
        climbs: 4,
        seed: 42,
        ..Options::default()
    }
}

fn model(limit: Limit) -> Model {
    Model::new(
        &synthetic_image(WIDTH, HEIGHT),
        WIDTH,
        HEIGHT,
        options(limit),
    )
    .unwrap()
}

/// A model that already holds `shapes` shapes.
fn model_with_shapes(shapes: u32) -> Model {
    let mut model = model(Limit::Shapes(shapes));
    while model.step().is_some() {}
    model
}

/// Validating the input, building the canvases and computing the average
/// background colour and initial error.
#[divan::bench]
fn new_model(bencher: Bencher) {
    let rgba = synthetic_image(WIDTH, HEIGHT);
    bencher.bench_local(|| {
        Model::new(black_box(&rgba), WIDTH, HEIGHT, options(Limit::Shapes(100))).unwrap()
    });
}

/// One full step (random search, hill climbing, blending) on a fresh model.
#[divan::bench]
fn first_step(bencher: Bencher) {
    bencher
        .with_inputs(|| model(Limit::Shapes(1)))
        .bench_local_refs(|model| black_box(model.step()));
}

/// One step on a model that already holds 50 shapes, where finding an
/// improvement is harder than on the flat background.
#[divan::bench]
fn step_after_50_shapes(bencher: Bencher) {
    let mut base = model(Limit::Shapes(51));
    for _ in 0..50 {
        base.step().unwrap();
    }
    bencher
        .with_inputs(|| base.clone())
        .bench_local_refs(|model| black_box(model.step()));
}

/// A complete run with a shape limit, the way the CLI uses the model.
#[divan::bench(args = [10, 25])]
fn run_to_shape_limit(bencher: Bencher, shapes: u32) {
    bencher
        .with_inputs(|| model(Limit::Shapes(shapes)))
        .bench_local_values(|mut model| {
            while model.step().is_some() {}
            black_box(model.bytes())
        });
}

/// A complete run with a byte limit, which also exercises the SVG size
/// accounting on every step.
#[divan::bench]
fn run_to_byte_limit(bencher: Bencher) {
    bencher
        .with_inputs(|| model(Limit::Bytes(1_500)))
        .bench_local_values(|mut model| {
            while model.step().is_some() {}
            black_box(model.bytes())
        });
}

/// Writing the SVG document for a model with 100 shapes.
#[divan::bench]
fn svg_100_shapes(bencher: Bencher) {
    let model = model_with_shapes(100);
    bencher.bench_local(|| black_box(&model).svg());
}

/// Writing the SVG at a different output width, which rescales every shape.
#[divan::bench]
fn svg_at_100_shapes(bencher: Bencher) {
    let model = model_with_shapes(100);
    bencher.bench_local(|| black_box(&model).svg_at(black_box(640)));
}
