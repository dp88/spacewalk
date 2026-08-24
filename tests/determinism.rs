//! The same question, asked twice, must get the same answer.
//!
//! A tactics game that replays a battle from a seed, or an AI search that must not wobble, needs
//! every query here to be a pure function of its inputs. The danger is hash order: BattleCore's
//! `find_path_toward` picks the closest reachable cell out of a `HashMap`, so when two cells tie it
//! chooses whichever the hasher happened to yield first — and a chasing enemy dithers between two
//! equally good squares from run to run.
//!
//! Here ties break on `(distance, cost, index)`, which is a total order: there are no ties left to
//! break arbitrarily.

use spacewalk::{Adjacency, FullGrid, Grid, Sq};

mod common;
use common::open;

#[test]
fn path_toward_breaks_a_perfect_tie_the_same_way_every_time() {
    // The enemy sits due east. Going north-then-east and south-then-east are exactly as good, and
    // there are four such symmetric routes. Something must choose, and it must always choose alike.
    let g = FullGrid::square(9, 9, Adjacency::Eight);
    let me = g.at(Sq::new(4, 4));
    let enemy = g.at(Sq::new(8, 4));
    let m = open(&g);

    let first = g.path_toward(me, enemy, 20, &m).unwrap();
    for _ in 0..50 {
        assert_eq!(g.path_toward(me, enemy, 20, &m).unwrap(), first);
    }
}

#[test]
fn path_toward_is_stable_across_freshly_built_grids() {
    // Rebuilding the grid rebuilds its HashMap with a fresh hasher state. The answer must not move.
    let expect = {
        let g = FullGrid::square(11, 11, Adjacency::Eight);
        let m = open(&g);
        let me = g.at(Sq::new(5, 5));
        let enemy = g.at(Sq::new(10, 10));
        g.coords_of(
            g.path_toward(me, enemy, 30, &m)
                .unwrap()
                .steps()
                .iter()
                .copied(),
        )
        .collect::<Vec<_>>()
    };

    for _ in 0..20 {
        let g = FullGrid::square(11, 11, Adjacency::Eight);
        let m = open(&g);
        let me = g.at(Sq::new(5, 5));
        let enemy = g.at(Sq::new(10, 10));
        let got = g.path_toward(me, enemy, 30, &m).unwrap();
        assert_eq!(
            g.coords_of(got.steps().iter().copied()).collect::<Vec<_>>(),
            expect,
        );
    }
}

#[test]
fn reach_comes_back_in_the_same_order_every_time() {
    let g = FullGrid::square(9, 9, Adjacency::Eight);
    let m = open(&g);
    let start = g.at(Sq::new(4, 4));

    let first = g.reachable(start, 30, &m);
    for _ in 0..20 {
        assert_eq!(g.reachable(start, 30, &m), first);
    }
}

#[test]
fn path_is_stable_when_several_routes_cost_the_same() {
    // On open ground there are a great many equally cheap ways across a board. A* must not pick a
    // different one each run.
    let g = FullGrid::square(12, 12, Adjacency::Four);
    let m = open(&g);
    let a = g.at(Sq::new(0, 0));
    let b = g.at(Sq::new(11, 11));

    let first = g.path(a, b, &m).unwrap();
    for _ in 0..20 {
        assert_eq!(g.path(a, b, &m).unwrap(), first);
    }
}

#[test]
fn a_component_comes_back_in_the_same_order_every_time() {
    // A flood fill visits cells in whatever order its frontier pops them, which is an artefact of
    // the fill and not of the board. A map generator that rejects an island, or a save file that
    // records one, must not see that order change between runs — so the answer is swept out of the
    // board in index order rather than reported in visit order.
    let g = FullGrid::square(9, 9, Adjacency::Eight);
    let open = |i| g.coord(i).x != 4 || g.coord(i).y == 8;
    let start = g.at(Sq::new(0, 0));

    let first: Vec<Sq> = g.component(start, open).cells().collect();
    for _ in 0..20 {
        let fresh = FullGrid::square(9, 9, Adjacency::Eight);
        let open = |i| fresh.coord(i).x != 4 || fresh.coord(i).y == 8;
        let again: Vec<Sq> = fresh.component(start, open).cells().collect();
        assert_eq!(again, first);
    }
}

#[test]
fn indices_follow_the_order_the_cells_were_given_in() {
    // Determinism starts at construction: the caller's cell order fixes the indices, and the
    // direction order fixes the step table, and everything downstream inherits both.
    for _ in 0..10 {
        let g = FullGrid::square(4, 4, Adjacency::Four);
        let order: Vec<Sq> = g.cells().take(5).collect();
        assert_eq!(
            order,
            [
                Sq::new(0, 0),
                Sq::new(1, 0),
                Sq::new(2, 0),
                Sq::new(3, 0),
                Sq::new(0, 1)
            ],
        );

        // And neighbours come back in the grid's direction order, not a hash order.
        let mid = g.at(Sq::new(1, 1));
        let dirs: Vec<_> = g.neighbors(mid).map(|(d, _)| d).collect();
        assert_eq!(dirs, g.dirs().to_vec());
    }
}
