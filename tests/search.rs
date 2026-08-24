//! The engine, checked against code that shares nothing with it.
//!
//! `tests/admissible.rs` compares `path` under a metric heuristic against `path` with a promised
//! minimum of zero — which is A\* against itself with `h = 0`. That catches a wrong heuristic, and
//! it is the reason that file exists. It cannot catch a mistake that affects both runs equally,
//! because both runs are the same code.
//!
//! So the oracle here is a different algorithm. Bellman–Ford relaxes every edge `n - 1` times and
//! settles nothing early: no heap, no heuristic, no notion of a cell being finished. It is far too
//! slow to ship and exactly right to check against, because the only thing it has in common with
//! the engine is the answer.
//!
//! The rest of the file pins what the engine promises beyond being cheapest: that it is
//! deterministic, that its two directions agree, and that nothing inside it is sized by a number
//! the caller passed in.

use std::collections::HashMap;

use spacewalk::{Adjacency, Cost, FullGrid, Grid, Hex, Idx, Movement, Sq, Step};

/// A cost per cell, from a fixed generator. Some cells are walls.
fn field(len: usize, seed: u64) -> Vec<Option<Cost>> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            match (s >> 33) % 10 {
                0 => None, // impassable
                n => Some([5u32, 7, 10, 10, 10, 14, 20, 30, 30][n as usize - 1]),
            }
        })
        .collect()
}

/// The cheapest cost from `start` to every cell, by repeated relaxation.
///
/// Bellman–Ford, deliberately naive. It shares no code with the crate's search: it keeps no queue,
/// settles nothing, and simply sweeps every edge until nothing improves. `None` where a cell cannot
/// be reached at all.
fn bellman_ford<B: Grid>(b: &B, start: Idx, cost: &[Option<Cost>]) -> HashMap<Idx, Cost> {
    let mut best: HashMap<Idx, u64> = HashMap::new();
    best.insert(start, 0);

    // At most `len - 1` edges in any simple path, so that many sweeps settle everything.
    for _ in 0..b.len().max(1) {
        let mut moved = false;
        for from in b.indices() {
            let Some(&here) = best.get(&from) else {
                continue;
            };
            for (_, to) in b.neighbors(from) {
                let Some(step) = cost[to.get() as usize] else {
                    continue;
                };
                let total = here + u64::from(step);
                if total < *best.get(&to).unwrap_or(&u64::MAX) {
                    best.insert(to, total);
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }

    best.into_iter()
        .map(|(i, c)| {
            (
                i,
                u32::try_from(c).expect("no total in these tests saturates"),
            )
        })
        .collect()
}

/// Costs are per-cell, so a step is priced by the cell it enters.
fn movement(cost: &[Option<Cost>]) -> impl Fn(Step<Sq>) -> Option<Cost> + '_ {
    move |s: Step<Sq>| cost[s.to.get() as usize]
}

#[test]
fn every_path_costs_what_bellman_ford_says_it_costs() {
    for (w, h, adj) in [
        (9, 7, Adjacency::Four),
        (9, 7, Adjacency::Eight),
        (12, 12, Adjacency::Eight),
    ] {
        for seed in [1u64, 0x9E37_79B9_7F4A_7C15, 0xDEAD_BEEF_CAFE_F00D] {
            let g = FullGrid::square(w, h, adj);
            let cost = field(g.len(), seed);
            let m = Movement::scan(&g, movement(&cost));

            let start = g.at(Sq::new(0, 0));
            let truth = bellman_ford(&g, start, &cost);

            for goal in g.indices() {
                let got = g.path(start, goal, &m);
                match (got, truth.get(&goal)) {
                    (Some(p), Some(&want)) => assert_eq!(
                        p.cost(),
                        want,
                        "{w}x{h} {adj:?} seed {seed:x}: cell {goal} is reachable for {want}",
                    ),
                    (None, None) => {}
                    (Some(p), None) => {
                        panic!(
                            "found a route to {goal} costing {} that does not exist",
                            p.cost()
                        )
                    }
                    (None, Some(&want)) => panic!("missed a route to {goal} costing {want}"),
                }
            }
        }
    }
}

#[test]
fn a_path_is_a_real_walk_and_its_cost_is_the_sum_of_its_steps() {
    // Cheapest is not enough. The route must also be one you could actually take: each cell a
    // neighbour of the last, and the total the sum of what entering them costs.
    let g = FullGrid::square(14, 11, Adjacency::Eight);
    let cost = field(g.len(), 0x5EED);
    let m = Movement::scan(&g, movement(&cost));
    let start = g.at(Sq::new(0, 0));

    for goal in g.indices() {
        let Some(p) = g.path(start, goal, &m) else {
            continue;
        };
        assert_eq!(p.start(), start);
        assert_eq!(p.destination(), goal);

        let mut total = 0;
        for pair in p.steps().windows(2) {
            let (from, to) = (pair[0], pair[1]);
            assert!(
                g.neighbors(from).any(|(_, n)| n == to),
                "{to} does not touch {from}",
            );
            total += cost[to.get() as usize].expect("a route may not enter a wall");
        }
        assert_eq!(total, p.cost(), "the total is the sum of the steps");
        assert_eq!(p.len(), p.steps().len() - 1);
    }
}

#[test]
fn hex_boards_agree_with_bellman_ford_too() {
    let g = FullGrid::hexagon(5);
    let cost = field(g.len(), 0xA11CE);
    let m = Movement::scan(&g, |s: Step<Hex>| cost[s.to.get() as usize]);

    let start = g.at(Hex::new(0, 0));
    let truth = bellman_ford(&g, start, &cost);

    for goal in g.indices() {
        assert_eq!(
            g.path(start, goal, &m).map(|p| p.cost()),
            truth.get(&goal).copied(),
        );
    }
}

#[test]
fn reachable_reports_the_cost_that_path_would_charge() {
    // Two different searches — A* with a heuristic, and a budget-bounded Dijkstra — must price the
    // same journey the same way, or one of them is lying about what a turn affords.
    let g = FullGrid::square(16, 16, Adjacency::Eight);
    let cost = field(g.len(), 0xB0A7);
    let m = Movement::scan(&g, movement(&cost));
    let start = g.at(Sq::new(8, 8));

    for (cell, budgeted) in g.reachable(start, 60, &m) {
        let walked = g.path(start, cell, &m).expect("reachable, so walkable");
        assert_eq!(walked.cost(), budgeted, "cell {cell}");
        assert!(budgeted <= 60, "and inside the budget it was given");
    }
}

#[test]
fn a_budget_hides_nothing_that_fits_inside_it() {
    // The early exit is the risky part: the search stops at the first cell past the budget, which
    // is only sound if cells really do come back cheapest-first.
    let g = FullGrid::square(12, 12, Adjacency::Four);
    let cost = field(g.len(), 0xFEED);
    let m = Movement::scan(&g, movement(&cost));
    let start = g.at(Sq::new(6, 6));

    let budget = 45;
    let bounded: Vec<Idx> = g
        .reachable(start, budget, &m)
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    let truth = bellman_ford(&g, start, &cost);

    for (&cell, &c) in &truth {
        assert_eq!(
            bounded.contains(&cell),
            c <= budget,
            "cell {cell} costs {c} against a budget of {budget}",
        );
    }
}

#[test]
fn cells_come_back_cheapest_first() {
    let g = FullGrid::square(16, 16, Adjacency::Eight);
    let cost = field(g.len(), 0x600D);
    let m = Movement::scan(&g, movement(&cost));

    let reached = g.reachable(g.at(Sq::new(0, 0)), 200, &m);
    assert!(
        reached.windows(2).all(|w| w[0].1 <= w[1].1),
        "the budget's early exit rests on this order",
    );
}

#[test]
fn the_same_question_always_gets_the_same_answer() {
    // The guarantee chosen when the engine was written: *which* of several equally cheap routes
    // comes back is not promised, but it does not wander. The heap breaks ties by index, and an
    // index compares by its number, so a rebuilt board answers alike.
    let make = || {
        let g = FullGrid::square(11, 11, Adjacency::Eight);
        let cost = field(g.len(), 7);
        (g, cost)
    };

    let (g, cost) = make();
    let m = Movement::scan(&g, movement(&cost));
    let (a, b) = (g.at(Sq::new(0, 0)), g.at(Sq::new(10, 10)));
    let first = g.path(a, b, &m).unwrap();

    for _ in 0..25 {
        let (g2, cost2) = make();
        let m2 = Movement::scan(&g2, movement(&cost2));
        let again = g2
            .path(g2.at(Sq::new(0, 0)), g2.at(Sq::new(10, 10)), &m2)
            .unwrap();
        assert_eq!(again.steps(), first.steps(), "same route");
        assert_eq!(again.cost(), first.cost());
    }
}

#[test]
fn an_equal_cost_route_is_still_a_cheapest_route() {
    // Stated where someone will look for it: ties may be broken differently than another
    // implementation would break them, and that is allowed. Being cheapest is not.
    let g = FullGrid::square(9, 9, Adjacency::Four);
    let m = Movement::uniform(&g, 10);
    let (a, b) = (g.at(Sq::new(0, 0)), g.at(Sq::new(4, 4)));

    let p = g.path(a, b, &m).unwrap();
    assert_eq!(p.cost(), 80, "eight steps, whichever way it went");
    assert_eq!(p.len(), 8);
}

#[test]
fn nothing_in_the_search_is_sized_by_a_number_you_passed_in() {
    // A budget far past anything the board can cost must not make the search bigger — it simply
    // runs out of board. If the queue were sized by the budget this would not return.
    let g = FullGrid::square(40, 40, Adjacency::Eight);
    let m = Movement::uniform(&g, 1);
    let start = g.at(Sq::new(20, 20));

    let all = g.reachable(start, Cost::MAX, &m);
    assert_eq!(
        all.len(),
        g.len(),
        "every cell, and no more than every cell"
    );

    let far = g.reaching(start, Cost::MAX, &m);
    assert_eq!(far.len(), g.len());
}
