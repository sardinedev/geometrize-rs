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

use crate::Options;
use crate::pixels::{Canvas, compute_color, difference_after_drawing};
use crate::raster::Scanline;
use crate::rng::Rng;
use crate::shape::Shape;

/// Everything a search reads but never changes.
pub(crate) struct SearchContext<'a> {
    /// The image being approximated.
    pub target: &'a Canvas,
    /// The picture so far.
    pub current: &'a Canvas,
    /// Total error between `target` and `current` (see
    /// [`crate::pixels::difference_full`]).
    pub total: u64,
    /// Alpha every shape is drawn with (from the opacity).
    pub alpha: u8,
    /// Settings: shape kinds, seed, `candidates`, `age` and `climbs`.
    pub options: &'a Options,
    /// Index of the step being searched (part of each search's seed).
    pub step: u64,
}

impl SearchContext<'_> {
    /// The total error the picture would have if `shape` were added to it
    /// (lower is better).
    ///
    /// Port of `Worker.Energy`. `lines` is scratch space for the shape's
    /// scanlines, reused between candidates so the hot loop doesn't allocate.
    fn energy(&self, shape: &Shape, lines: &mut Vec<Scanline>) -> u64 {
        let (width, height) = self.target.size();
        shape.rasterize(width, height, lines);
        let color = compute_color(self.target, self.current, lines, self.alpha);
        difference_after_drawing(self.target, self.current, self.total, color, lines)
    }

    /// A random shape of one of the configured kinds.
    fn random_shape(&self, rng: &mut Rng) -> Shape {
        let (width, height) = self.target.size();
        Shape::random_of(&self.options.shapes, width, height, rng)
    }
}

/// The best shape one search found.
pub(crate) struct Candidate {
    /// The shape.
    pub shape: Shape,
    /// Its energy: the total error after adding it.
    pub energy: u64,
    /// Which search found it, used to break ties.
    pub search: u32,
}

impl Candidate {
    /// Sort key, best first: lower energy, then lower search index.
    fn rank(&self) -> (u64, u32) {
        (self.energy, self.search)
    }
}

/// The best of `ctx.options.candidates` random shapes, with its energy.
///
/// Port of `BestRandomState`.
fn best_random(ctx: &SearchContext, lines: &mut Vec<Scanline>, rng: &mut Rng) -> (Shape, u64) {
    let mut best_shape = ctx.random_shape(rng);
    let mut best_energy = ctx.energy(&best_shape, lines);
    for _ in 1..ctx.options.candidates {
        let shape = ctx.random_shape(rng);
        let energy = ctx.energy(&shape, lines);
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
    lines: &mut Vec<Scanline>,
    rng: &mut Rng,
    mut shape: Shape,
    mut energy: u64,
) -> (Shape, u64) {
    let (width, height) = ctx.target.size();
    let mut age = 0;
    while age < ctx.options.age {
        let previous = shape;
        shape.mutate(width, height, rng);
        let new_energy = ctx.energy(&shape, lines);
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
fn run_search(ctx: &SearchContext, lines: &mut Vec<Scanline>, search: u32) -> Candidate {
    let mut rng = Rng::for_search(ctx.options.seed, ctx.step, u64::from(search));
    let (shape, energy) = best_random(ctx, lines, &mut rng);
    let (shape, energy) = hill_climb(ctx, lines, &mut rng, shape, energy);
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
/// scanline buffer.
#[cfg(feature = "parallel")]
fn run_searches_in_parallel(ctx: &SearchContext) -> Option<Candidate> {
    use rayon::prelude::*;
    (0..ctx.options.climbs)
        .into_par_iter()
        .map_init(Vec::new, |lines, search| run_search(ctx, lines, search))
        .min_by_key(Candidate::rank)
}

/// Runs the searches one after another, sharing one scanline buffer.
#[cfg(not(feature = "parallel"))]
fn run_searches_in_sequence(ctx: &SearchContext) -> Option<Candidate> {
    let mut lines = Vec::new();
    (0..ctx.options.climbs)
        .map(|search| run_search(ctx, &mut lines, search))
        .min_by_key(Candidate::rank)
}
