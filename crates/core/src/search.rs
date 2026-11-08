//! Finding the next shape: random search followed by hill climbing.
//!
//! Port of primitive's `worker.go` (`Energy`, `BestHillClimbState`,
//! `BestRandomState`) and `optimize.go` (`HillClimb`). Each step runs
//! `climbs` independent searches. Each search takes the best of `candidates`
//! random shapes, then improves it by mutation until `age` mutations in a row
//! fail to help. The best result over all searches wins.
//!
//! Searches never share a random generator: search `i` of step `s` always
//! uses `Rng::for_search(seed, s, i)`, and ties go to the lowest `i`. So the
//! winner is the same whether the searches run one after another or spread
//! across any number of threads.

use std::cmp::Ordering;

use crate::Options;
use crate::pixels::{Canvas, compute_color, copy_lines, difference_partial, draw_lines};
use crate::raster::Scanline;
use crate::rng::Rng;
use crate::shape::Shape;

/// Everything a search reads but never changes.
pub(crate) struct SearchContext<'a> {
    /// The image being approximated.
    pub target: &'a Canvas,
    /// The picture so far.
    pub current: &'a Canvas,
    /// Difference between `target` and `current`.
    pub score: f64,
    /// Alpha every shape is drawn with (from the opacity).
    pub alpha: u8,
    /// Settings: shape kinds, seed, `candidates`, `age` and `climbs`.
    pub options: &'a Options,
    /// Index of the step being searched (part of each search's seed).
    pub step: u64,
}

impl SearchContext<'_> {
    fn width(&self) -> i32 {
        self.target.width as i32
    }

    fn height(&self) -> i32 {
        self.target.height as i32
    }
}

/// Scratch memory for one search, reused between candidates so the hot loop
/// doesn't allocate.
pub(crate) struct Worker {
    /// Canvas the candidate shapes are test-drawn on.
    buffer: Canvas,
    /// Scanlines of the shape being evaluated.
    lines: Vec<Scanline>,
}

impl Worker {
    /// Creates scratch memory for a `width × height` image.
    pub(crate) fn new(width: usize, height: usize) -> Worker {
        Worker {
            buffer: Canvas::filled(width, height, crate::Rgba::new(0, 0, 0, 0)),
            lines: Vec::new(),
        }
    }

    /// The score the picture would have if `shape` were added to it (lower is
    /// better).
    ///
    /// Port of `Worker.Energy`. The shape is drawn on the scratch buffer,
    /// which is first reset to `current` on the shape's scanlines only.
    /// Pixels outside those lines may hold leftovers from earlier candidates,
    /// but `difference_partial` never reads them.
    fn energy(&mut self, ctx: &SearchContext, shape: &Shape) -> f64 {
        shape.rasterize(ctx.width(), ctx.height(), &mut self.lines);
        let color = compute_color(ctx.target, ctx.current, &self.lines, ctx.alpha);
        copy_lines(&mut self.buffer, ctx.current, &self.lines);
        draw_lines(&mut self.buffer, color, &self.lines);
        difference_partial(
            ctx.target,
            ctx.current,
            &self.buffer,
            ctx.score,
            &self.lines,
        )
    }
}

/// The best shape one search found.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Candidate {
    /// The shape.
    pub shape: Shape,
    /// Its energy (the score after adding it).
    pub energy: f64,
    /// Which search found it, used to break ties.
    pub search: u32,
}

/// Orders candidates best first: lower energy, then lower search index.
fn compare(a: &Candidate, b: &Candidate) -> Ordering {
    a.energy
        .total_cmp(&b.energy)
        .then_with(|| a.search.cmp(&b.search))
}

/// The best of `ctx.options.candidates` random shapes, with its energy.
///
/// Port of `BestRandomState`.
fn best_random(ctx: &SearchContext, worker: &mut Worker, rng: &mut Rng) -> (Shape, f64) {
    let mut best_shape = Shape::random_of(&ctx.options.shapes, ctx.width(), ctx.height(), rng);
    let mut best_energy = worker.energy(ctx, &best_shape);
    for _ in 1..ctx.options.candidates {
        let shape = Shape::random_of(&ctx.options.shapes, ctx.width(), ctx.height(), rng);
        let energy = worker.energy(ctx, &shape);
        if energy < best_energy {
            best_shape = shape;
            best_energy = energy;
        }
    }
    (best_shape, best_energy)
}

/// Improves `shape` by random mutations, keeping each one only if it lowers
/// the energy, until `ctx.options.age` mutations in a row have failed.
///
/// Port of `HillClimb`. Shapes are `Copy`, so undoing a mutation is just
/// putting the previous value back.
fn hill_climb(
    ctx: &SearchContext,
    worker: &mut Worker,
    rng: &mut Rng,
    mut shape: Shape,
    mut energy: f64,
) -> (Shape, f64) {
    let mut age = 0;
    while age < ctx.options.age {
        let previous = shape;
        shape.mutate(ctx.width(), ctx.height(), rng);
        let new_energy = worker.energy(ctx, &shape);
        if new_energy < energy {
            energy = new_energy;
            age = 0;
        } else {
            shape = previous;
            age += 1;
        }
    }
    (shape, energy)
}

/// Runs search number `search`: random search, then hill climbing.
fn run_search(ctx: &SearchContext, worker: &mut Worker, search: u32) -> Candidate {
    let mut rng = Rng::for_search(ctx.options.seed, ctx.step, u64::from(search));
    let (shape, energy) = best_random(ctx, worker, &mut rng);
    let (shape, energy) = hill_climb(ctx, worker, &mut rng, shape, energy);
    Candidate {
        shape,
        energy,
        search,
    }
}

/// Runs `ctx.options.climbs` searches and returns the best result.
///
/// Port of `BestHillClimbState` (spread over workers by `runWorkers` in
/// primitive). With the `parallel` feature the searches run on rayon's thread
/// pool; otherwise they run in a loop. Both give the same answer.
pub(crate) fn best_hill_climb(ctx: &SearchContext) -> Candidate {
    let best = cfg_select! {
        feature = "parallel" => { run_searches_in_parallel(ctx) }
        _ => { run_searches_in_sequence(ctx) }
    };
    best.expect("climbs is at least 1 (checked by Model::new)")
}

/// Runs the searches on rayon's thread pool, each thread with its own
/// [`Worker`].
#[cfg(feature = "parallel")]
fn run_searches_in_parallel(ctx: &SearchContext) -> Option<Candidate> {
    use rayon::prelude::*;
    (0..ctx.options.climbs)
        .into_par_iter()
        .map_init(
            || Worker::new(ctx.target.width, ctx.target.height),
            |worker, search| run_search(ctx, worker, search),
        )
        .min_by(compare)
}

/// Runs the searches one after another with a single [`Worker`].
#[cfg(not(feature = "parallel"))]
fn run_searches_in_sequence(ctx: &SearchContext) -> Option<Candidate> {
    let mut worker = Worker::new(ctx.target.width, ctx.target.height);
    (0..ctx.options.climbs)
        .map(|search| run_search(ctx, &mut worker, search))
        .min_by(compare)
}
