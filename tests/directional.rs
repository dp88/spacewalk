//! The headline: cost is what it costs to *enter* a cell, from a *direction*.
//!
//! Rivers, conveyor belts, and ledges you can drop off but not climb back up. None of these can be
//! said by an engine whose movement cost is a property of the cell — which is most of them, and
//! which is what BattleCore does. Attaching the cost to the *crossing* rather than the *cell* is a
//! small change to the model, and it is the difference between "mud is slow" and "the current runs
//! south".
//!
//! The price is that the graph is directed, and the tests at the bottom pin down what that costs.

use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Movement, Sq};

#[test]
fn a_river_runs_one_way() {
    let g = FullGrid::square(5, 5, Adjacency::Four);
    let river: Vec<Sq> = (0..5).map(|y| Sq::new(2, y)).collect();

    // The current runs south. Swimming down it is nearly free; fighting up it is brutal.
    let m = Movement::scan(&g, |s| {
        Some(if river.contains(&g.coord(s.to)) {
            match s.dir {
                Dir8::S => 2,
                Dir8::N => 40,
                _ => 10, // crossing it sideways
            }
        } else {
            10
        })
    });

    let top = g.index_of(Sq::new(2, 0)).unwrap();
    let bottom = g.index_of(Sq::new(2, 4)).unwrap();

    // Four cells downstream at 2 apiece.
    let down = g.path(top, bottom, &m).unwrap();
    assert_eq!(down.cost, 8);
    assert!(
        down.steps.iter().all(|&i| g.coord(i).x == 2),
        "it stays in the water"
    );

    // Upstream is so dear that it is cheaper to climb out, walk the bank, and get back in.
    let up = g.path(bottom, top, &m).unwrap();
    assert!(up.cost < 4 * 40, "it did not slog up the current");
    assert!(
        up.steps.iter().any(|&i| g.coord(i).x != 2),
        "it left the river and walked the bank"
    );
}

#[test]
fn a_conveyor_carries_you_and_resists_you() {
    let g = FullGrid::square(1, 6, Adjacency::Four);

    // A belt running the length of a corridor, southward.
    let m = Movement::scan(&g, |s| {
        Some(match s.dir {
            Dir8::S => 1,
            _ => 20,
        })
    });

    let top = g.index_of(Sq::new(0, 0)).unwrap();
    let bottom = g.index_of(Sq::new(0, 5)).unwrap();

    assert_eq!(g.path(top, bottom, &m).unwrap().cost, 5, "carried");
    assert_eq!(
        g.path(bottom, top, &m).unwrap().cost,
        100,
        "and fought all the way back"
    );
}

#[test]
fn a_ledge_can_be_dropped_off_but_not_climbed() {
    let g = FullGrid::square(3, 3, Adjacency::Four);
    // A cliff edge along y = 1: you may fall south over it, never climb north back up.
    let m = Movement::scan(&g, |s| {
        let (from, to) = (g.coord(s.from), g.coord(s.to));
        let climbing = from.y == 1 && to.y == 0;
        (!climbing).then_some(10)
    });

    let above = g.index_of(Sq::new(0, 0)).unwrap();
    let below = g.index_of(Sq::new(0, 2)).unwrap();

    assert_eq!(g.path(above, below, &m).unwrap().cost, 20, "down is easy");
    assert!(
        g.path(below, above, &m).is_none(),
        "and there is no way back up"
    );
}

#[test]
fn a_path_cannot_be_reversed() {
    // The consequence to remember. There is no `Path::reverse`, and this is why.
    let g = FullGrid::square(4, 1, Adjacency::Four);
    let m = Movement::scan(&g, |s| Some(if s.dir == Dir8::E { 10 } else { 50 }));

    let a = g.index_of(Sq::new(0, 0)).unwrap();
    let b = g.index_of(Sq::new(3, 0)).unwrap();

    let there = g.path(a, b, &m).unwrap();
    let back = g.path(b, a, &m).unwrap();

    assert_eq!(there.cost, 30);
    assert_eq!(back.cost, 150);

    let mut reversed = there.steps.clone();
    reversed.reverse();
    assert_eq!(
        reversed, back.steps,
        "the route home happens to be the same cells..."
    );
    assert_ne!(
        there.cost, back.cost,
        "...but it does not cost the same to walk it"
    );
}

#[test]
fn reach_means_reach_out_not_reach_and_return() {
    // Everything downhill of you is reachable. Getting home is a different question, and the grid
    // does not answer it.
    let g = FullGrid::square(1, 5, Adjacency::Four);
    let m = Movement::scan(&g, |s| (s.dir == Dir8::S).then_some(10));

    let top = g.index_of(Sq::new(0, 0)).unwrap();
    let bottom = g.index_of(Sq::new(0, 4)).unwrap();

    let out = g.reachable(top, 40, &m);
    assert_eq!(out.len(), 5, "the whole corridor is reachable going down");

    let home = g.reachable(bottom, 40, &m);
    assert_eq!(
        home.len(),
        1,
        "and from the bottom you can reach only yourself"
    );
}

#[test]
fn a_direction_blind_cost_still_works_and_is_symmetric() {
    // The common case, unchanged: ignore the direction and you get ordinary terrain.
    let g = FullGrid::square(5, 5, Adjacency::Four);
    let m = Movement::scan(&g, |s| {
        Some(if g.coord(s.to).x == 2 { 30 } else { 10 }) // a north-south band of mud
    });

    let a = g.index_of(Sq::new(0, 2)).unwrap();
    let b = g.index_of(Sq::new(4, 2)).unwrap();

    assert_eq!(
        g.path(a, b, &m).unwrap().cost,
        g.path(b, a, &m).unwrap().cost
    );
}
