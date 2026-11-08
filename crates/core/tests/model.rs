//! Model-level tests through the public API.
//!
//! Images are tiny (32×32) and the search settings small, so these run fast.

use geometrize_core::{Error, Limit, Model, Options, Rgba, ShapeKind};

const SIZE: u32 = 32;

/// A 32×32 synthetic image: a diagonal gradient with a bright square in the
/// middle, so there is real structure for the shapes to find.
fn synthetic_image() -> Vec<u8> {
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let in_square = (10..22).contains(&x) && (10..22).contains(&y);
            if in_square {
                rgba.extend_from_slice(&[250, 230, 40, 255]);
            } else {
                rgba.extend_from_slice(&[(x * 8) as u8, (y * 8) as u8, 120, 255]);
            }
        }
    }
    rgba
}

/// Small, fast search settings.
fn quick_options(limit: Limit) -> Options {
    Options {
        limit,
        candidates: 50,
        age: 10,
        climbs: 4,
        out_width: 64,
        seed: 1234,
        ..Options::default()
    }
}

fn run(options: Options) -> Model {
    let mut model = Model::new(&synthetic_image(), SIZE, SIZE, options).unwrap();
    while model.step().is_some() {}
    model
}

#[test]
fn shape_limit_gives_exactly_that_many_shapes() {
    let mut model = Model::new(
        &synthetic_image(),
        SIZE,
        SIZE,
        quick_options(Limit::Shapes(7)),
    )
    .unwrap();
    for _ in 0..7 {
        assert!(model.step().is_some());
    }
    assert!(model.step().is_none());
    assert!(model.step().is_none(), "stays stopped");
    assert_eq!(model.shapes().len(), 7);
}

#[test]
fn byte_limit_is_never_exceeded() {
    let limit = 1500;
    let mut model = Model::new(
        &synthetic_image(),
        SIZE,
        SIZE,
        quick_options(Limit::Bytes(limit)),
    )
    .unwrap();
    while let Some(step) = model.step() {
        assert_eq!(step.bytes, model.bytes());
        assert_eq!(model.bytes(), model.svg().len());
    }
    let svg = model.svg();
    assert!(svg.len() <= limit, "{} > {limit}", svg.len());
    assert_eq!(model.bytes(), svg.len());
    assert!(!model.shapes().is_empty());
    // A triangle is ~50 bytes here, so stopping must have left little room.
    assert!(
        limit - svg.len() < 100,
        "stopped early at {} bytes",
        svg.len()
    );
}

#[test]
fn both_limits_stop_at_whichever_comes_first() {
    let by_shapes = run(quick_options(Limit::Both {
        shapes: 3,
        bytes: 100_000,
    }));
    assert_eq!(by_shapes.shapes().len(), 3);

    let by_bytes = run(quick_options(Limit::Both {
        shapes: 10_000,
        bytes: 1200,
    }));
    assert!(by_bytes.svg().len() <= 1200);
    assert!(by_bytes.shapes().len() < 10_000);
}

#[test]
fn same_seed_gives_identical_svg() {
    let a = run(quick_options(Limit::Shapes(20))).svg();
    let b = run(quick_options(Limit::Shapes(20))).svg();
    assert_eq!(a, b);

    let other_seed = run(Options {
        seed: 99,
        ..quick_options(Limit::Shapes(20))
    })
    .svg();
    assert_ne!(a, other_seed);
}

#[cfg(feature = "parallel")]
#[test]
fn thread_count_does_not_change_the_output() {
    let svg_with_threads = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| run(quick_options(Limit::Shapes(20))).svg())
    };
    let one = svg_with_threads(1);
    let four = svg_with_threads(4);
    assert_eq!(one, four);
}

#[test]
fn every_shape_improves_the_score() {
    let mut model = Model::new(
        &synthetic_image(),
        SIZE,
        SIZE,
        quick_options(Limit::Shapes(30)),
    )
    .unwrap();
    let starting_score = model.score();
    let mut previous = starting_score;
    while let Some(step) = model.step() {
        assert!(step.score < previous, "{} >= {previous}", step.score);
        assert_eq!(step.score, model.score());
        previous = step.score;
    }
    assert!(model.score() < 0.5 * starting_score);
}

#[test]
fn stops_when_no_shape_can_help() {
    // A flat image is matched exactly by its own background, so no shape can
    // lower the score.
    let flat = [30u8, 60, 90, 255].repeat((SIZE * SIZE) as usize);
    let mut model = Model::new(&flat, SIZE, SIZE, quick_options(Limit::Shapes(10))).unwrap();
    assert!(model.step().is_none());
    assert!(model.step().is_none(), "stays stopped");
    assert!(model.shapes().is_empty());
    assert_eq!(model.score(), 0.0);
}

#[test]
fn svg_structure_for_three_shapes() {
    let model = run(quick_options(Limit::Shapes(3)));
    let svg = model.svg();
    // The colour may be written in short #rgb form, so only check the frame
    // around it.
    assert!(svg.starts_with(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"><path fill=\"#"
    ));
    assert!(svg.contains("\" d=\"M0 0h64v64H0z\"/>"), "{svg}");
    assert_eq!(svg.matches("<path").count(), 4, "background + 3 shapes");
    assert_eq!(svg.matches(" d=\"M").count(), 4);
    assert_eq!(svg.matches("fill-opacity=\".5\"").count(), 3);
    assert!(svg.ends_with("z\"/></svg>"));
    assert!(!svg.contains('\n'));
}

#[test]
fn opacity_is_written_to_the_svg() {
    let faint = run(Options {
        opacity: 0.3,
        ..quick_options(Limit::Shapes(2))
    });
    assert_eq!(faint.svg().matches("fill-opacity=\".3\"").count(), 2);
    assert!(faint.shapes().iter().all(|(_, color)| color.a == 77));

    let opaque = run(Options {
        opacity: 1.0,
        ..quick_options(Limit::Shapes(2))
    });
    assert!(!opaque.svg().contains("fill-opacity"));
}

#[test]
fn opacity_is_rounded_once_for_both_maths_and_svg() {
    // 0.996 rounds to 100%: drawn fully opaque, and no fill-opacity written.
    let nearly_opaque = run(Options {
        opacity: 0.996,
        ..quick_options(Limit::Shapes(2))
    });
    assert!(!nearly_opaque.svg().contains("fill-opacity"));
    assert!(nearly_opaque.shapes().iter().all(|(_, c)| c.a == 255));

    // 0.333 rounds to 33%: fitted at alpha 84 (33% of 255), written as .33.
    let third = run(Options {
        opacity: 0.333,
        ..quick_options(Limit::Shapes(2))
    });
    assert_eq!(third.svg().matches("fill-opacity=\".33\"").count(), 2);
    assert!(third.shapes().iter().all(|(_, c)| c.a == 84));
}

#[test]
fn debug_output_leaves_out_the_pixels() {
    let model = run(quick_options(Limit::Shapes(2)));
    let debug = format!("{model:?}");
    assert!(
        debug.starts_with("Model { width: 32, height: 32,"),
        "{debug}"
    );
    assert!(debug.len() < 600, "{} characters", debug.len());
}

#[test]
fn background_defaults_to_average_and_can_be_set() {
    let flat = [30u8, 60, 90, 255].repeat(16);
    let model = Model::new(&flat, 4, 4, quick_options(Limit::Shapes(1))).unwrap();
    assert_eq!(model.background(), Rgba::new(30, 60, 90, 255));
    assert_eq!(model.score(), 0.0);

    let chosen = Rgba::new(1, 2, 3, 255);
    let model = Model::new(
        &flat,
        4,
        4,
        Options {
            bg: Some(chosen),
            ..quick_options(Limit::Shapes(1))
        },
    )
    .unwrap();
    assert_eq!(model.background(), chosen);
}

#[test]
fn invalid_options_are_rejected() {
    let image = synthetic_image();
    let new = |options: Options| Model::new(&image, SIZE, SIZE, options).err();
    let base = || quick_options(Limit::Shapes(1));

    assert_eq!(Model::new(&[], 0, 5, base()).err(), Some(Error::EmptyImage));
    assert_eq!(
        Model::new(&[], 70_000, 1, base()).err(),
        Some(Error::ImageTooLarge {
            width: 70_000,
            height: 1
        })
    );
    assert_eq!(
        Model::new(&[], 65_536, 65_536, base()).err(),
        Some(Error::BufferLength {
            expected: 65_536 * 65_536 * 4,
            actual: 0
        }),
        "the largest allowed size gets as far as the buffer check on 64-bit"
    );
    assert_eq!(
        Model::new(&image, SIZE, SIZE - 1, base()).err(),
        Some(Error::BufferLength {
            expected: (SIZE * (SIZE - 1) * 4) as usize,
            actual: image.len()
        })
    );
    for opacity in [0.0, 0.001, 0.009, 1.001, 1.5, -1.0, f32::NAN] {
        assert!(
            matches!(new(Options { opacity, ..base() }), Some(Error::Opacity(_))),
            "opacity {opacity}"
        );
    }
    assert_eq!(
        new(Options {
            shapes: vec![],
            ..base()
        }),
        Some(Error::NoShapeKinds)
    );
    assert_eq!(
        new(Options {
            candidates: 0,
            ..base()
        }),
        Some(Error::Zero("candidates"))
    );
    assert_eq!(
        new(Options {
            climbs: 0,
            ..base()
        }),
        Some(Error::Zero("climbs"))
    );
    assert_eq!(
        new(Options {
            out_width: 0,
            ..base()
        }),
        Some(Error::Zero("output width"))
    );
    assert!(matches!(
        new(Options {
            limit: Limit::Bytes(50),
            ..base()
        }),
        Some(Error::ByteLimitTooSmall { limit: 50, .. })
    ));
    assert_eq!(
        new(Options {
            shapes: vec![ShapeKind::Triangle; 2],
            ..base()
        }),
        None
    );
}

#[test]
fn other_output_widths_scale_the_view_box() {
    let model = run(quick_options(Limit::Shapes(3)));
    assert!(model.svg_at(320).contains("viewBox=\"0 0 320 320\""));
    assert!(model.svg_at(320).contains("d=\"M0 0h320v320H0z\""));
}

/// Writes a larger result to /tmp/core.svg for a visual check:
/// `cargo test -p geometrize-core --release --features parallel -- --ignored`
#[test]
#[ignore]
fn write_preview_svg() {
    let (w, h) = (256u32, 171u32);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 - 128.0;
            let dy = y as f64 - 85.0;
            let sun = (dx * dx + dy * dy).sqrt() < 40.0;
            let sky = [(40 + y / 2) as u8, (80 + y / 3) as u8, (200 - y / 2) as u8];
            let pixel = if sun { [255, 200, 60] } else { sky };
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    let mut model = Model::new(
        &rgba,
        w,
        h,
        Options {
            limit: Limit::Shapes(300),
            ..Options::default()
        },
    )
    .unwrap();
    while model.step().is_some() {}
    std::fs::write("/tmp/core.svg", model.svg()).unwrap();
    println!("score {:.4}, {} bytes", model.score(), model.bytes());
}
