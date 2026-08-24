//! Acceptance test: a tactical battle on a square grid — the classic turn-based tactics shape,
//! where units with a movement allowance take turns crossing terrain to reach and strike.
//!
//! These port BattleCore's `tests/movement_rules.rs`, which is the behaviour a tactics game needs
//! and the reason `MovementRules` exists over there. The point of porting them is to prove the
//! generic crate did not lose anything on the way out.
//!
//! Two of BattleCore's ten tests deliberately do *not* come across: "units path through allies but
//! cannot stop on them" and "allies can be made to block". Those are about *who* blocks, which is
//! occupancy, which is a game concept. The grid answers *reach*; the game filters *destinations*.

use spacewalk::{Adjacency, Cost, Dir8, FullGrid, Grid, Movement, Sq, Step};

mod common;
use common::{diagonal_aware, open as plain};

fn reached(
    g: &FullGrid<Sq>,
    from: Sq,
    budget: Cost,
    m: &Movement<impl Fn(Step<Sq>) -> Option<Cost>>,
) -> Vec<Sq> {
    let start = g.at(from);
    let mut cells: Vec<Sq> = g
        .reachable(start, budget, m)
        .into_iter()
        .map(|(i, _)| g.coord(i))
        .collect();
    cells.sort();
    cells
}

#[test]
fn four_way_movement_has_no_diagonals() {
    let g = FullGrid::square(16, 16, Adjacency::Four);
    let cells = reached(&g, Sq::new(8, 8), 40, &plain(&g));

    assert!(cells.contains(&Sq::new(12, 8)), "four cells east");
    assert!(cells.contains(&Sq::new(10, 10)), "two east and two south");
    assert!(
        !cells.contains(&Sq::new(12, 12)),
        "four east and four south is eight steps, not four"
    );
}

#[test]
fn eight_way_movement_reaches_diagonally() {
    let g = FullGrid::square(16, 16, Adjacency::Eight);
    let m = diagonal_aware(&g);

    // Two diagonals cost 28, inside a budget of 40. Three cost 42, outside it.
    let cells = reached(&g, Sq::new(8, 8), 40, &m);
    assert!(cells.contains(&Sq::new(10, 10)));
    assert!(!cells.contains(&Sq::new(11, 11)));
}

#[test]
fn eight_way_melee_can_hit_a_diagonally_adjacent_enemy() {
    // The bug this whole design guards against. Under eight-way movement a unit may *step* onto
    // the diagonal — so it had better measure it as one cell away, or it can stand beside an enemy
    // and be unable to swing at it.
    let g = FullGrid::square(8, 8, Adjacency::Eight);
    let me = g.at(Sq::new(3, 3));
    let enemy = g.at(Sq::new(4, 4));

    assert_eq!(g.distance(me, enemy), 1, "diagonally adjacent is ONE away");
    assert!(
        g.within(me, 1, 1).contains(Sq::new(4, 4)),
        "so melee reaches it"
    );
    assert!(g.step(me, Dir8::Se).is_some(), "and movement agrees");
}

#[test]
fn four_way_melee_cannot_hit_diagonally() {
    // And the mirror image: under four-way movement the diagonal is genuinely two away, and the
    // unit genuinely cannot step there either. The metric and the adjacency agree, which is the
    // whole point.
    let g = FullGrid::square(8, 8, Adjacency::Four);
    let me = g.at(Sq::new(3, 3));
    let enemy = g.at(Sq::new(4, 4));

    assert_eq!(g.distance(me, enemy), 2, "diagonally adjacent is TWO away");
    assert!(
        !g.within(me, 1, 1).contains(Sq::new(4, 4)),
        "so melee does not reach it"
    );
    assert!(g.step(me, Dir8::Se).is_none(), "and movement agrees");
}

#[test]
fn archers_get_a_diamond_under_four_way_and_a_square_under_eight() {
    let mid = Sq::new(5, 5);

    // A bow with range 2-3: it cannot fire at what is next to it.
    let four = FullGrid::square(11, 11, Adjacency::Four);
    let i = four.at(mid);
    let diamond = four.within(i, 2, 3);
    assert!(!diamond.contains(Sq::new(6, 5)), "too close");
    assert!(diamond.contains(Sq::new(8, 5)), "three east");
    assert!(!diamond.contains(Sq::new(9, 5)), "too far");
    assert!(!diamond.contains(Sq::new(7, 7)), "the diamond's corner");

    let eight = FullGrid::square(11, 11, Adjacency::Eight);
    let j = eight.at(mid);
    let square = eight.within(j, 2, 3);
    assert!(
        square.contains(Sq::new(7, 7)),
        "under Chebyshev that corner is only two away, so the range is a square"
    );
}

#[test]
fn terrain_makes_a_unit_go_around() {
    let g = FullGrid::square(7, 3, Adjacency::Four);
    // A wall of mountains down the middle, with one gap at the bottom.
    let mountains = [Sq::new(3, 0), Sq::new(3, 1)];
    let m = Movement::scan(&g, |s| (!mountains.contains(&g.coord(s.to))).then_some(10));

    let from = g.at(Sq::new(0, 0));
    let to = g.at(Sq::new(6, 0));

    let p = g.path(from, to, &m).unwrap();
    assert!(
        p.steps().iter().any(|&i| g.coord(i) == Sq::new(3, 2)),
        "it must funnel through the gap"
    );
    // Six east if the ridge were not there, plus two south and two north to get round it.
    assert_eq!(p.cost(), 100);
}

#[test]
fn a_unit_cannot_walk_through_another() {
    let g = FullGrid::square(5, 1, Adjacency::Four);
    let blocker = Sq::new(2, 0);

    // Occupancy is just another reason a cell cannot be entered. The grid never learns what a unit
    // is; BattleCore threaded a `blocked: &HashSet<Position>` through every signature to say this.
    let m = Movement::scan(&g, |s| (g.coord(s.to) != blocker).then_some(10));

    let from = g.at(Sq::new(0, 0));
    let to = g.at(Sq::new(4, 0));
    assert!(g.path(from, to, &m).is_none(), "the corridor is plugged");
}

#[test]
fn a_pursuing_unit_closes_on_a_target_it_cannot_reach_this_turn() {
    let g = FullGrid::square(12, 1, Adjacency::Four);
    let m = plain(&g);

    let me = g.at(Sq::new(0, 0));
    let enemy = g.at(Sq::new(11, 0));

    let p = g.path_toward(me, enemy, 30, &m).unwrap();
    assert_eq!(
        g.coord(p.destination()),
        Sq::new(3, 0),
        "spends its whole move closing"
    );
    assert_eq!(g.distance(p.destination(), enemy), 8);
}
