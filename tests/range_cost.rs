//! A range query must cost what the *range* costs, not what the *board* costs.
//!
//! `within` is the hottest query a tactics game makes — attack range, vision, blast radius, for
//! every unit, every turn. It used to scan the whole board, so a radius-2 lookup on a 200×200 map
//! cost 88µs and got slower as the map grew, which is exactly backwards.
//!
//! The fix has a trap in it, and the second half of this file is about the trap: enumerating the
//! offsets within a radius means allocating a list sized by *the caller's radius*. Done naively,
//! `within(i, 0, u32::MAX)` asks for about 7×10¹⁹ entries — fixing one memory bomb by shipping
//! another. So the grid asks how big the list *would* be before building it, and scans the board
//! instead when that is cheaper. Both answers are the same. Both are bounded by the board.

use std::time::Instant;

use spacewalk::{Adjacency, FullGrid, Grid, Hex, Sq};

#[test]
fn a_small_radius_costs_the_same_on_a_big_board_as_on_a_small_one() {
    // The claim is not "fast". The claim is "**the board's size stops mattering**" — so measure
    // exactly that, and nothing else.
    //
    // This used to assert an absolute budget: under 10µs per call. That is a proxy for the real
    // property, and a bad one — it measures the machine as much as the code. A shared CI runner is
    // several times slower than a dev box and has noisy neighbours, so the number that passes here
    // fails there, and the test becomes a coin toss that everyone learns to re-run.
    //
    // A ratio has no such problem. Both boards are timed on the same machine in the same second, so
    // the machine cancels out. A radius-2 query on a 100× bigger board must cost about the same; if
    // `within` ever goes back to scanning, the big board costs ~100× the small one and this fails
    // by two orders of magnitude, on any hardware.
    let big = FullGrid::square(200, 200, Adjacency::Eight); // 40,000 cells
    let small = FullGrid::square(20, 20, Adjacency::Eight); //     400 cells
    let (bi, si) = (
        big.index_of(Sq::new(100, 100)).unwrap(),
        small.index_of(Sq::new(10, 10)).unwrap(),
    );

    let time = |g: &FullGrid<Sq>, i| {
        for _ in 0..1_000 {
            let _ = g.within(i, 1, 2); // warm the cache and the branch predictor
        }
        let t = Instant::now();
        for _ in 0..20_000 {
            let _ = g.within(i, 1, 2);
        }
        t.elapsed().as_nanos().max(1) // never zero, so the ratio is always defined
    };

    // Interleaved, so a passing thermal cloud or a busy neighbour hits both alike.
    let (b1, s1) = (time(&big, bi), time(&small, si));
    let (s2, b2) = (time(&small, si), time(&big, bi));
    let ratio = (b1 + b2) as f64 / (s1 + s2) as f64;

    assert_eq!(
        big.within(bi, 1, 2).len(),
        small.within(si, 1, 2).len(),
        "same work, either way"
    );
    assert!(
        ratio < 10.0,
        "a radius-2 query cost {ratio:.1}× more on a board 100× bigger. It should cost about the \
         same — that is what the offset table is for. Scanning the board would put this near 100."
    );
}

#[test]
fn the_answer_is_the_same_whichever_way_it_is_computed() {
    // The offset walk and the board scan must agree exactly — including order, since callers may
    // compare the results. `Metric::scanning` forces the scan; the shipped metrics take the offsets.
    let fast = FullGrid::square(40, 40, Adjacency::Eight);
    let i = fast.index_of(Sq::new(20, 20)).unwrap();

    for (min, max) in [(0, 0), (0, 1), (1, 1), (2, 3), (0, 5), (3, 3)] {
        let by_offsets: Vec<Sq> = fast.within(i, min, max).cells().collect();

        // The same question, answered by brute force over every cell.
        let by_scan: Vec<Sq> = fast
            .indices()
            .filter(|&j| {
                let d = fast.distance(i, j);
                d >= min && d <= max
            })
            .map(|j| fast.coord(j))
            .collect();

        assert_eq!(by_offsets, by_scan, "range {min}..={max}");
    }
}

#[test]
fn a_preposterous_radius_does_not_try_to_allocate_the_universe() {
    // The trap. `(2r + 1)²` at r = u32::MAX is about 7×10¹⁹ offsets. Building that list first and
    // asking questions later is how you fill 32GB of swap. The grid counts before it builds, sees
    // the count exceeds the board, and scans instead.
    //
    // If this test ever stops returning, that is the bug.
    let g = FullGrid::square(30, 30, Adjacency::Eight);
    let centre = g.index_of(Sq::new(15, 15)).unwrap();

    let t = Instant::now();
    let all = g.within(centre, 0, u32::MAX);
    let took = t.elapsed();

    assert_eq!(
        all.len(),
        g.len(),
        "every cell is within an infinite radius"
    );
    assert!(
        took.as_millis() < 100,
        "and it took {took:?}, not the rest of your life"
    );
}

#[test]
fn the_crossover_is_invisible_from_outside() {
    // Small radius takes the offsets; large radius takes the scan. A caller should never be able to
    // tell which, except by timing it.
    let g = FullGrid::square(20, 20, Adjacency::Four);
    let i = g.index_of(Sq::new(10, 10)).unwrap();

    let small = g.within(i, 0, 3); // offsets: 25 of them
    let large = g.within(i, 0, 500); // scan: 500 is far bigger than the board

    assert!(small.len() < large.len());
    assert_eq!(large.len(), g.len());

    // And a radius either side of the crossover still agrees with itself.
    assert_eq!(g.within(i, 0, 40).len(), g.len());
}

#[test]
fn an_inverted_range_is_empty_rather_than_expensive() {
    // `min > max` used to build the whole radius-max disc and then filter every one of them away.
    let g = FullGrid::square(10, 10, Adjacency::Four);
    let i = g.index_of(Sq::new(5, 5)).unwrap();

    assert!(g.within(i, 5, 2).is_empty());
    assert!(g.within(i, 1, 0).is_empty());
}

#[test]
fn hex_rings_still_jump_over_holes() {
    // The clone-and-jump game's whole move set, and the reason `within` measures COORDINATES rather
    // than walking the graph. It must survive the switch to an offset table.
    let g = FullGrid::hexagon(4).filtered(|c| c != Hex::new(1, 0));
    let from = g.index_of(Hex::new(2, 0)).unwrap();

    let across = g.index_of(Hex::new(0, 0)).unwrap();
    assert_eq!(g.distance(from, across), 2);
    assert!(
        g.ring(from, 2).contains(Hex::new(0, 0)),
        "two away across the hole, so it is a legal jump"
    );
}

#[test]
fn the_shapes_are_still_right() {
    // The offset table has to reproduce the metric exactly, or an archer's range changes shape.
    let four = FullGrid::square(11, 11, Adjacency::Four);
    let i = four.index_of(Sq::new(5, 5)).unwrap();
    assert_eq!(four.within(i, 1, 1).len(), 4, "a plus sign");
    assert_eq!(four.within(i, 1, 2).len(), 12, "a diamond");

    let eight = FullGrid::square(11, 11, Adjacency::Eight);
    let j = eight.index_of(Sq::new(5, 5)).unwrap();
    assert_eq!(eight.within(j, 1, 1).len(), 8, "a ring");
    assert_eq!(
        eight.within(j, 1, 2).len(),
        24,
        "a 5x5 square, less the centre"
    );

    let hex = FullGrid::hexagon(3);
    let c = hex.index_of(Hex::new(0, 0)).unwrap();
    assert_eq!(hex.within(c, 1, 1).len(), 6);
    assert_eq!(hex.within(c, 1, 2).len(), 18, "6 + 12");
}
