//! Every test here is a bug we actually had. Several of them could take a machine down.
//!
//! The theme, and the reason they are collected in one file: **the crate does arithmetic on numbers
//! it did not choose** — costs from your closure, coordinates from your [`Coord`], radii and indices
//! from your call. Rust does not check integer overflow in release builds; it wraps. And a wrapped
//! number in a graph search is not a wrong answer, it is a *hang*: Dijkstra and A\* are licensed to
//! settle a cell and stop reconsidering it only because extending a path can never make it cheaper.
//! Break that and cells re-open forever, the heap grows without bound, and the process eats memory
//! until the machine dies.
//!
//! **Run these in release too** (`cargo test --release`). Debug's overflow checks mask exactly the
//! bugs that only exist in release, which is the build a game ships.

use std::ops::{Add, Sub};

use spacewalk::{Adjacency, Coord, Dir8, FullGrid, Grid, Hex, Metric, Movement, Path, Sq, Step};

mod common;

// ---------------------------------------------------------------------------------------------
// Costs: the one that took the machine down
// ---------------------------------------------------------------------------------------------

#[test]
fn costs_too_large_for_the_board_are_refused_at_the_door() {
    // A 10-cell corridor. Nine steps at 600 million is 5.4 billion, which does not fit in a u32.
    // `scan` walks every edge anyway, so it can see this coming and say so.
    let g = FullGrid::square(10, 1, Adjacency::Four);

    let panic = std::panic::catch_unwind(|| Movement::scan(&g, |_: Step<Sq>| Some(600_000_000)));
    let msg = *panic.unwrap_err().downcast::<String>().unwrap();

    assert!(
        msg.contains("600000000"),
        "it names the offending cost: {msg}"
    );
    assert!(msg.contains("overflow"), "and says why it matters: {msg}");
}

#[test]
fn a_total_that_would_overflow_saturates_instead_of_hanging() {
    // THE regression test. `Movement::new` skips the scan, so nothing refuses these costs — and
    // before the fix, the total wrapped, a longer path started looking cheaper than a short one,
    // Dijkstra re-opened cells forever, and the heap ate all memory plus 32GB of swap.
    //
    // If this test ever hangs again, that is the bug back.
    let g = FullGrid::square(10, 1, Adjacency::Four);
    let m = Movement::new(|_: Step<Sq>| Some(600_000_000), 0);

    let a = g.index_of(Sq::new(0, 0)).unwrap();
    let b = g.index_of(Sq::new(9, 0)).unwrap();

    let p = g
        .path(a, b, &m)
        .expect("it must terminate, and it must find the corridor");
    assert_eq!(p.len(), 9);
    assert_eq!(
        p.cost,
        u32::MAX,
        "the total pegs at the ceiling rather than wrapping to a lie"
    );
}

#[test]
fn a_colossal_min_step_cannot_overflow_the_heuristic() {
    // The subtler half, and the one that fires at costs FAR below `u32::MAX`. A* stores `g + h`.
    // With a huge `min_step` the heuristic saturates to `u32::MAX`, and the very next addition
    // wrapped — sending garbage f-values to the front of the heap, wrecking its ordering, and
    // re-expanding nodes without bound. An 8x8 board with steps costing 10 was enough to do it.
    //
    // A dishonest `min_step` is still a bug, and in debug the `debug_assert` in `succ` says so. The
    // invariant this test pins is narrower and more important: however dishonest the numbers, the
    // search **comes back**. Panicking is an acceptable answer. Hanging is not.
    let g = FullGrid::square(8, 8, Adjacency::Four);
    let m = Movement::new(|_: Step<Sq>| Some(10), u32::MAX);

    let a = g.index_of(Sq::new(0, 0)).unwrap();
    let b = g.index_of(Sq::new(7, 7)).unwrap();

    // Ok  => release: it saturates, terminates, and finds a path.
    // Err => debug: the debug_assert caught the lying min_step first. Also an acceptable answer.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.path(a, b, &m)));
    if let Ok(found) = outcome {
        assert!(
            found.is_some(),
            "it saturates, terminates, and still finds the path"
        );
    }
}

#[test]
fn reach_and_path_toward_survive_the_same_costs() {
    let g = FullGrid::square(12, 1, Adjacency::Four);
    let m = Movement::new(|_: Step<Sq>| Some(900_000_000), 0);
    let a = g.index_of(Sq::new(0, 0)).unwrap();
    let z = g.index_of(Sq::new(11, 0)).unwrap();

    assert!(!g.reachable(a, u32::MAX, &m).is_empty());
    assert!(g.path_toward(a, z, u32::MAX, &m).is_some());
}

// ---------------------------------------------------------------------------------------------
// Coordinates: arithmetic at the edge of i32
// ---------------------------------------------------------------------------------------------

#[test]
fn distances_are_exact_at_the_extremes_of_i32() {
    // `self.x - o.x` on an i32 wraps. Two cells four billion apart used to report as ADJACENT —
    // an archer "in range" of the far side of the world. Widened to i64, so it cannot happen.
    let far = Sq::new(i32::MAX, 0).chebyshev(Sq::new(i32::MIN, 0));
    assert_eq!(far, u32::MAX, "the true distance, clamped — not 1");

    // Manhattan had a second overflow on top: the two unsigned_abs values summed past u32::MAX.
    let both = Sq::new(i32::MIN, i32::MIN).manhattan(Sq::new(0, 0));
    assert_eq!(both, u32::MAX, "clamped, not 0");

    // Hex had three: the subtraction, the negation in s(), and the three-term sum.
    let hex = Hex::new(i32::MIN, 0).distance(Hex::new(0, 0));
    assert!(hex > 1_000_000_000, "a real distance, not 0: {hex}");
}

#[test]
fn a_grid_of_extreme_coordinates_can_be_built_and_measured() {
    // In debug this used to panic during construction; in release it forged an edge between two
    // cells four billion apart, and every algorithm cheerfully walked it.
    let g = FullGrid::new(
        [Sq::new(i32::MAX, 0), Sq::new(i32::MIN, 0), Sq::new(0, 0)],
        &Dir8::ORTHO,
        Metric::MANHATTAN,
    );

    let hi = g.index_of(Sq::new(i32::MAX, 0)).unwrap();
    let lo = g.index_of(Sq::new(i32::MIN, 0)).unwrap();

    assert!(g.distance(hi, lo) > 1, "they are not neighbours");
    assert_eq!(
        g.neighbors(hi).count(),
        0,
        "and no wrap-around edge was forged"
    );
}

// ---------------------------------------------------------------------------------------------
// A Coord that wraps: the torus, which is an ordinary thing to want
// ---------------------------------------------------------------------------------------------

const W: i32 = 8;

/// A one-dimensional world that wraps around, so stepping east forever returns you to where you
/// began. Perfectly reasonable — and it makes the step table *cyclic*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Ring(i32);

impl Add for Ring {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Ring((self.0 + o.0).rem_euclid(W))
    }
}
impl Sub for Ring {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Ring((self.0 - o.0).rem_euclid(W))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Spin {
    Round,
}

impl Coord for Ring {
    type Dir = Spin;
    const DIRS: &'static [Spin] = &[Spin::Round];
    fn step(self, _: Spin) -> Self {
        Ring((self.0 + 1).rem_euclid(W))
    }
}

#[test]
fn a_ray_on_a_wrapping_world_terminates() {
    // `ray` walks a direction until the board runs out. On a torus the board never runs out, so it
    // used to yield forever — and the doc examples teach you to `.collect()` it. That is an
    // unbounded Vec, which is another way of saying "all of your memory".
    let g = FullGrid::new(
        (0..W).map(Ring),
        Ring::DIRS,
        Metric::scanning(|a: Ring, b: Ring| (b - a).0.unsigned_abs()),
    );

    let start = g.index_of(Ring(0)).unwrap();
    let walked: Vec<_> = g.ray(start, Spin::Round).collect();

    assert_eq!(walked.len(), g.len(), "bounded by the board, and no longer");
}

#[test]
fn a_clamping_step_does_not_become_a_self_loop() {
    // A `step` that clamps at the edge (`x.min(w - 1)`) makes the edge cell step onto itself. Left
    // in the table that is a zero-length cycle: `ray` spins on it forever and the search sees a free
    // edge to nowhere. `FullGrid::new` drops it.
    // The last cell steps onto itself.
    common::coord_1d!(Clamp, Spin = Spin::Round, |x| Clamp((x.0 + 1).min(4)));

    let g = FullGrid::new(
        (0..=4).map(Clamp),
        Clamp::DIRS,
        Metric::scanning(|a: Clamp, b: Clamp| (b.0 - a.0).unsigned_abs()),
    );

    let last = g.index_of(Clamp(4)).unwrap();
    assert_eq!(
        g.step(last, Spin::Round),
        None,
        "the self-step is not an edge"
    );
    assert_eq!(g.ray(last, Spin::Round).count(), 0);
}

// ---------------------------------------------------------------------------------------------
// Sizes, emptiness, and indices
// ---------------------------------------------------------------------------------------------

#[test]
fn a_negative_board_is_refused_rather_than_silently_empty() {
    // `FullGrid::square(-5, 3)` used to build an EMPTY grid without a word, and then every operation on
    // it panicked somewhere far away.
    assert!(std::panic::catch_unwind(|| FullGrid::square(-5, 3, Adjacency::Four)).is_err());
}

#[test]
fn an_impossibly_large_board_is_refused_rather_than_attempted() {
    // 46341^2 is 2.1 billion cells — a 68GB step table. It used to try.
    let boom = std::panic::catch_unwind(|| FullGrid::square(46_341, 46_341, Adjacency::Eight));
    let msg = *boom.unwrap_err().downcast::<String>().unwrap();
    assert!(msg.contains("at most"), "it says what the limit is: {msg}");

    assert!(std::panic::catch_unwind(|| FullGrid::hexagon(50_000)).is_err());
    assert!(std::panic::catch_unwind(|| FullGrid::hexagon(-1)).is_err());

    // A disc guards its bounding box, and that guard has to be computed in `u64`: `2 * i32::MAX +
    // 1` squared lands 8.6 billion short of the top. One width narrower it wraps, the radius goes
    // through, and the loop walks four billion rows.
    assert!(std::panic::catch_unwind(|| FullGrid::disc(i32::MAX, Adjacency::Four)).is_err());
    assert!(std::panic::catch_unwind(|| FullGrid::disc(-1, Adjacency::Four)).is_err());
}

#[test]
fn an_empty_grid_is_harmless() {
    let g = FullGrid::square(0, 0, Adjacency::Four);
    assert_eq!(g.len(), 0);
    assert!(g.is_empty());
    assert_eq!(g.indices().count(), 0);
    assert_eq!(g.index_of(Sq::new(0, 0)), None);

    // No index is valid on it, so every index-taking call must say so rather than fault.
    assert!(std::panic::catch_unwind(|| g.coord(0)).is_err());
}

#[test]
fn a_foreign_index_says_so() {
    let g = FullGrid::square(3, 3, Adjacency::Four);
    let boom = std::panic::catch_unwind(|| g.coord(999));
    let msg = *boom.unwrap_err().downcast::<String>().unwrap();

    assert!(msg.contains("999"), "names the index: {msg}");
    assert!(msg.contains("9 cells"), "and the board it is not on: {msg}");
}

#[test]
fn a_metric_that_disagrees_with_the_directions_is_refused() {
    // The crate's own headline bug, which it warned about on the front page and then permitted one
    // line away from the front door. Eight-way movement measured with Manhattan distance: a unit can
    // *step* onto the diagonal, and is told the diagonal is TWO cells away — so it can stand beside
    // an enemy and be unable to swing at it. `FullGrid::square` made this impossible. `FullGrid::new`
    // accepted it without a murmur, which is exactly where a custom board or a restored save lands.
    let boom = std::panic::catch_unwind(|| {
        FullGrid::new(
            (0..5).flat_map(|y| (0..5).map(move |x| Sq::new(x, y))),
            &Dir8::ALL,        // diagonals ARE steps
            Metric::MANHATTAN, // ...but Manhattan calls a diagonal two away
        )
    });

    let msg = *boom.unwrap_err().downcast::<String>().unwrap();
    assert!(
        msg.contains("covers 2"),
        "it says what the step actually spans: {msg}"
    );
    assert!(msg.contains("disagrees"), "and names the problem: {msg}");

    // The pairings that agree are of course fine.
    let _ = FullGrid::square(5, 5, Adjacency::Four);
    let _ = FullGrid::square(5, 5, Adjacency::Eight);
}

#[test]
fn a_board_with_genuine_multi_cell_steps_may_opt_out_with_a_zero_metric() {
    // The escape hatch the docs promise, and it must actually work. A portal, a jump, a conveyor
    // that carries you three cells — none of those is one step by any honest measure. A metric of 0
    // is always an underestimate, so A* degrades into Dijkstra: slower, still correct.
    common::coord_1d!(Leap, Jump, |x| Leap(x.0 + 3)); // a portal: three cells at a bound

    let g = FullGrid::new((0..9).map(Leap), Leap::DIRS, Metric::scanning(|_, _| 0));
    let m = Movement::scan(&g, |_| Some(10));

    let start = g.index_of(Leap(0)).unwrap();
    let far = g.index_of(Leap(6)).unwrap();
    assert_eq!(g.path(start, far, &m).unwrap().len(), 2, "two portal hops");
}

#[test]
fn an_empty_path_does_not_claim_to_be_eighteen_quintillion_long() {
    // `Path`'s fields are public, so a caller can build this. `steps.len() - 1` on an empty vector
    // wraps to usize::MAX in release, and `is_empty()` returned FALSE — so a caller who correctly
    // guarded with `is_empty()` then looped `0..p.len()` 18 quintillion times.
    let p = Path {
        steps: vec![],
        cost: 0,
    };

    assert_eq!(p.len(), 0);
    assert!(p.is_empty());
    assert_eq!(p.destination(), None);
}
