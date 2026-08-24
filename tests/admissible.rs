//! A\* must return the *cheapest* path, not merely a path.
//!
//! This is the regression test BattleCore never had, for a bug it still has. Its A\* heuristic
//! charges one movement point per remaining step, on the stated grounds that "terrain can only make
//! a step cost *more* than 1.0" — and then its own terrain table gives a road a cost of 0.5. The
//! heuristic overestimates, A\* stops being admissible, and it quietly returns non-optimal paths
//! across any road network. Nothing fails; the unit just takes the long way and no one is told.
//!
//! The fix is that the heuristic scales by the *cheapest step on the board*, and
//! [`Movement::scan`] measures it rather than assuming it. These tests hold that shut.
//!
//! The oracle is Dijkstra, which needs no heuristic and cannot be inadmissible: `Movement::new(f, 0)`
//! makes the heuristic zero, which is exactly Dijkstra.

use spacewalk::{Adjacency, Cost, FullGrid, Grid, Movement, Sq, Step};

/// A deterministic pseudo-random terrain generator. No `rand` dependency: the crate has none, and
/// a fixed sequence makes a failure reproducible.
fn terrain(seed: u64, len: usize, costs: &[Cost]) -> Vec<Cost> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            costs[(s >> 33) as usize % costs.len()]
        })
        .collect()
}

/// The cheapest path from corner to corner, found by A\* and by Dijkstra. They must agree.
fn astar_and_dijkstra(g: &FullGrid<Sq>, cost: &[Cost]) -> (Cost, Cost) {
    let enter = |s: Step<Sq>| Some(cost[s.to.get() as usize]);

    let with_heuristic = Movement::scan(g, enter);
    let no_heuristic = Movement::new(enter, 0); // h = 0 is Dijkstra

    let from = g.at(Sq::new(0, 0));
    let to = g
        .index_of(Sq::new(
            g.cells().last().unwrap().x,
            g.cells().last().unwrap().y,
        ))
        .unwrap();

    (
        g.path(from, to, &with_heuristic).unwrap().cost(),
        g.path(from, to, &no_heuristic).unwrap().cost(),
    )
}

#[test]
fn astar_matches_dijkstra_when_a_road_is_cheaper_than_open_ground() {
    // Roads at 5, plains at 10, forest at 20 — BattleCore's own table, scaled by ten. The cheapest
    // step is 5, not 10, and a heuristic that assumes 10 will overestimate.
    let g = FullGrid::square(20, 20, Adjacency::Four);

    for seed in 0..40 {
        let cost = terrain(seed, g.len(), &[5, 10, 20]);
        let (astar, dijkstra) = astar_and_dijkstra(&g, &cost);
        assert_eq!(
            astar, dijkstra,
            "seed {seed}: A* found a worse path than Dijkstra"
        );
    }
}

#[test]
fn astar_matches_dijkstra_on_eight_way_grids_with_diagonal_costs() {
    let g = FullGrid::square(16, 16, Adjacency::Eight);

    for seed in 100..130 {
        let base = terrain(seed, g.len(), &[5, 10, 20]);
        let enter = |s: Step<Sq>| {
            let c = base[s.to.get() as usize];
            Some(if s.dir.is_diagonal() { c * 14 / 10 } else { c })
        };

        let from = g.at(Sq::new(0, 0));
        let to = g.at(Sq::new(15, 15));

        let a = g.path(from, to, &Movement::scan(&g, enter)).unwrap().cost();
        let d = g.path(from, to, &Movement::new(enter, 0)).unwrap().cost();
        assert_eq!(a, d, "seed {seed}");
    }
}

#[test]
fn astar_matches_dijkstra_when_costs_are_directional() {
    // Rivers make the graph directed, and admissibility has to survive that too.
    let g = FullGrid::square(16, 16, Adjacency::Four);

    for seed in 200..230 {
        let base = terrain(seed, g.len(), &[3, 10, 25]);
        let enter = |s: Step<Sq>| {
            let c = base[s.to.get() as usize];
            // Half the board flows south: going with it is cheap, against it is dear.
            Some(if c == 3 {
                if s.dir == spacewalk::Dir8::S { 3 } else { 30 }
            } else {
                c
            })
        };

        let from = g.at(Sq::new(0, 0));
        let to = g.at(Sq::new(15, 15));

        let a = g.path(from, to, &Movement::scan(&g, enter)).unwrap().cost();
        let d = g.path(from, to, &Movement::new(enter, 0)).unwrap().cost();
        assert_eq!(a, d, "seed {seed}");
    }
}

#[test]
fn scan_finds_the_cheapest_step_and_not_the_commonest_one() {
    let g = FullGrid::square(10, 10, Adjacency::Four);
    let cost = terrain(7, g.len(), &[5, 10, 20]);

    let m = Movement::scan(&g, |s| Some(cost[s.to.get() as usize]));
    assert_eq!(
        m.min_step(),
        5,
        "one road anywhere on the board sets the floor for the heuristic"
    );
}

#[test]
fn an_overstated_minimum_is_what_makes_astar_go_wrong() {
    // The bug itself, reproduced deliberately — so that if anyone ever "optimises" the heuristic by
    // assuming a floor, this test tells them exactly what they broke.
    let g = FullGrid::square(12, 3, Adjacency::Four);

    // A road along the top row at 2; everything else costs 10.
    let enter = |s: Step<Sq>| Some(if g.coord(s.to).y == 0 { 2 } else { 10 });

    let from = g.at(Sq::new(0, 2));
    let to = g.at(Sq::new(11, 2));

    let truth = g.path(from, to, &Movement::new(enter, 0)).unwrap().cost();
    let honest = g.path(from, to, &Movement::scan(&g, enter)).unwrap().cost();
    let liar = g.path(from, to, &Movement::new(enter, 10)).unwrap().cost();

    assert_eq!(
        honest, truth,
        "an honest minimum finds the true cheapest path"
    );
    assert!(
        liar > truth,
        "and a minimum that assumes every step costs 10 misses the road entirely: \
         it returned {liar} against a true cost of {truth}"
    );
}
