//! Acceptance test: elevation — high ground, dead ground, and a cliff you must walk around.
//!
//! Height is the one thing a tactics map has that the crate deliberately does not store. It is game
//! state, so it lives in a `CellMap` the application owns, and two gates read it: `height_gate` for
//! what a hill hides, `climb_gate` for what a ledge refuses.
//!
//! The mission below is one board and one height field, driving both. That is the claim worth
//! testing — not that either gate works alone, but that the same `CellMap` answers sight and
//! movement without the grid learning what a hill is.

use spacewalk::height::{climb_gate, height_gate};
use spacewalk::{Adjacency, CellMap, Cost, FullGrid, Grid, Hex, Idx, Movement, Sq};

mod common;

/// A valley running east to west, with a ridge four units high across the middle of it.
///
/// ```text
///   y=0  . . . . # . . . .
///   y=1  . . . . # . . . .     # is the ridge, four units up
///   y=2  . . . . . . . . .     y=2 is the pass through it, at ground level
///   y=3  . . . . # . . . .
/// ```
fn valley() -> (FullGrid<Sq>, CellMap<i32>) {
    let g = FullGrid::square(9, 4, Adjacency::Eight);
    let ground = CellMap::from_fn(&g, |c: Sq| i32::from(c.x == 4 && c.y != 2) * 4);
    (g, ground)
}

/// Sight over `ground`, with every eye and every mark two units above the earth it stands on.
fn eyes<'a>(
    g: &'a FullGrid<Sq>,
    ground: &'a CellMap<i32>,
) -> impl Fn(spacewalk::Sight) -> bool + 'a {
    height_gate(g, |i| ground[i], |i| ground[i] + 2)
}

#[test]
fn a_ridge_casts_dead_ground_behind_it() {
    let (g, ground) = valley();
    let sight = eyes(&g, &ground);

    let scout = g.at(Sq::new(0, 1));
    assert!(
        !g.los_by(scout, g.at(Sq::new(8, 1)), &sight),
        "the far side of the ridge is dead ground"
    );
    assert!(
        g.los_by(scout, g.at(Sq::new(8, 2)), &sight),
        "but the pass is open, and you can see clean through it"
    );
}

#[test]
fn the_high_ground_sees_what_the_valley_floor_cannot() {
    // The whole reason a tactics game wants elevation. Same board, same target, two observers who
    // differ only in what they are standing on.
    let (g, ground) = valley();
    let sight = eyes(&g, &ground);

    let target = g.at(Sq::new(8, 1));
    let on_the_floor = g.at(Sq::new(0, 1));
    let on_the_ridge = g.at(Sq::new(4, 1));

    assert!(!g.los_by(on_the_floor, target, &sight));
    assert!(
        g.los_by(on_the_ridge, target, &sight),
        "from up here, plainly"
    );
}

#[test]
fn a_field_of_view_over_a_ridge_is_still_a_board() {
    // Sight comes back as a `SubGrid`, so the cells an archer can see are the cells you then reason
    // over — that promise must survive the height gate like any other blocker.
    let (g, ground) = valley();
    let sight = eyes(&g, &ground);

    let archer = g.at(Sq::new(0, 1));
    let seen = g.visible_from_by(archer, 8, &sight);

    assert!(seen.contains(Sq::new(4, 1)), "the ridge itself is visible");
    assert!(!seen.contains(Sq::new(8, 1)), "what it hides is not");
    assert!(seen.contains(Sq::new(8, 2)), "and the pass is");

    // It is a board: every cell of it maps back, and a route inside it cannot leave it.
    let walk = common::open(&g);
    let inside = seen.path(seen.at(Sq::new(0, 1)), seen.at(Sq::new(8, 2)), &walk);
    assert!(
        inside.is_some(),
        "the pass is walkable without leaving sight"
    );
    for i in seen.indices() {
        assert!(g.contains(g.coord(seen.to_root(i))));
    }
}

#[test]
fn a_cliff_turns_a_straight_march_into_a_detour() {
    // The same height field, now driving movement. Two units of climb is the limit, and the ridge
    // is four — so the only way east is the pass at y = 2.
    let (g, ground) = valley();
    let climb = climb_gate(|i: Idx| ground[i], 2);
    let walk = Movement::scan(&g, |s| climb(s).then_some(10 as Cost));

    let west = g.at(Sq::new(0, 0));
    let east = g.at(Sq::new(8, 0));

    let route = g.path(west, east, &walk).expect("the pass is open");
    let through: Vec<Sq> = route.cells(&g).collect();

    assert!(
        through.contains(&Sq::new(4, 2)),
        "it must use the pass: {through:?}"
    );
    assert!(
        !through.iter().any(|c| c.x == 4 && c.y != 2),
        "and never sets foot on the ridge"
    );
}

#[test]
fn you_may_drop_off_the_ridge_you_could_not_climb() {
    let (g, ground) = valley();
    let climb = climb_gate(|i: Idx| ground[i], 2);
    let walk = Movement::scan(&g, |s| climb(s).then_some(10 as Cost));

    let ridge = g.at(Sq::new(4, 1));
    let floor = g.at(Sq::new(4, 2));

    // Off the ridge and into the pass is one step, a four-unit drop notwithstanding.
    assert_eq!(g.path(ridge, floor, &walk).unwrap().len(), 1);

    // There is no way back up. Four units is beyond climbing from every side, and this ridge has no
    // ramp — so its top is a place you can leave and never reach. Only a directed graph can say
    // that, and `reachable` and `reaching` disagree about it by design.
    assert!(g.path(floor, ridge, &walk).is_none());
    assert!(
        !g.reachable(floor, 10_000, &walk)
            .iter()
            .any(|&(i, _)| i == ridge)
    );
    assert!(
        g.reaching(floor, 10_000, &walk)
            .iter()
            .any(|&(i, _)| i == ridge)
    );
}

#[test]
fn sight_over_a_height_field_is_symmetric_on_hexes_too() {
    // The symmetry the one-closure design keeps, checked on the other lattice the crate ships. A
    // hex line rounds through cube coordinates, so it is a genuinely different rounding path.
    let g = FullGrid::hexagon(4);
    let ground = CellMap::from_fn(&g, |c: Hex| (c.q * 5 + c.r * 11).rem_euclid(7));
    let sight = height_gate(&g, |i| ground[i], |i| ground[i] + 2);

    for a in g.indices() {
        for b in g.indices() {
            assert_eq!(
                g.los_by(a, b, &sight),
                g.los_by(b, a, &sight),
                "{:?} <-> {:?}",
                g.coord(a),
                g.coord(b)
            );
        }
    }
}

#[test]
fn a_flat_height_field_answers_exactly_as_no_height_field_at_all() {
    // The degenerate case, and the one that would catch an off-by-one in the comparison: with the
    // ground level everywhere, the gate must block nothing and agree with plain sight.
    let g = FullGrid::square(11, 11, Adjacency::Eight);
    let flat = CellMap::new(&g, 0i32);
    let sight = height_gate(&g, |i| flat[i], |i| flat[i] + 2);

    for a in g.indices() {
        for b in g.indices() {
            assert!(
                g.los_by(a, b, &sight),
                "nothing on a flat board blocks sight"
            );
            assert_eq!(g.los_by(a, b, &sight), g.los(a, b, |_| false));
        }
    }
}
