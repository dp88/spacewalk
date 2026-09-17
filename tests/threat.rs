//! Acceptance test: "who can reach *this* cell?" — the threat map.
//!
//! `reachable` answers "where can I go". A tactics AI mostly wants the other question: *which cells
//! is this square threatened from*, so it can avoid standing there. On an undirected graph they are
//! the same question. This graph is directed — that is the whole point of rivers and one-way ledges
//! — so they are not, and answering the second used to mean one forward search per enemy on the
//! board.
//!
//! The grid can ask who steps *into* a cell, so it is one backward Dijkstra.

use spacewalk::{Adjacency, Coord, Dir8, FullGrid, Grid, GridError, Hex, Metric, Movement, Sq};

mod common;

#[test]
fn on_open_ground_reaching_and_reachable_agree() {
    // The symmetric case. If every step is reversible at the same cost, "where can I go" and "who
    // can get to me" describe the same set — and if they disagreed here, the reverse search is wrong.
    let g = FullGrid::square(9, 9, Adjacency::Four);
    let m = Movement::scan(&g, |_| Some(10));
    let centre = g.at(Sq::new(4, 4));

    let out: Vec<_> = g.reachable(centre, 30, &m);
    let inn: Vec<_> = g.reaching(centre, 30, &m);

    let mut a: Vec<_> = out.iter().map(|&(i, c)| (i, c)).collect();
    let mut b: Vec<_> = inn.iter().map(|&(i, c)| (i, c)).collect();
    a.sort_unstable();
    b.sort_unstable();

    assert_eq!(a, b);
}

#[test]
fn a_one_way_ledge_makes_them_disagree() {
    // And the asymmetric case, which is why the method exists.
    let g = FullGrid::square(1, 5, Adjacency::Four);
    let m = Movement::scan(&g, |s| (s.dir == Dir8::S).then_some(10));

    let bottom = g.at(Sq::new(0, 4));

    assert_eq!(
        g.reaching(bottom, 100, &m).len(),
        5,
        "everything above can fall to the bottom"
    );
    assert_eq!(
        g.reachable(bottom, 100, &m).len(),
        1,
        "but from the bottom you go nowhere"
    );

    let top = g.at(Sq::new(0, 0));
    assert_eq!(
        g.reaching(top, 100, &m).len(),
        1,
        "and nothing can climb up to the top"
    );
    assert_eq!(g.reachable(top, 100, &m).len(), 5);
}

#[test]
fn a_river_costs_what_it_costs_in_the_direction_it_is_crossed() {
    // The reverse search must charge the FORWARD cost of each step, not some mirrored guess.
    // Reaching the river's mouth from upstream is cheap; from downstream it is dear.
    let g = FullGrid::square(1, 4, Adjacency::Four);
    let m = Movement::scan(&g, |s| Some(if s.dir == Dir8::S { 1 } else { 50 }));

    let mouth = g.at(Sq::new(0, 3));
    let source = g.at(Sq::new(0, 0));

    let to_mouth: Vec<_> = g.reaching(mouth, 10, &m);
    assert!(
        to_mouth.iter().any(|&(i, c)| i == source && c == 3),
        "three cells downstream at 1 apiece: {to_mouth:?}"
    );

    // The other way is 50 a cell, so a budget of 10 gets nobody home.
    let to_source: Vec<_> = g.reaching(source, 10, &m);
    assert_eq!(to_source.len(), 1, "only the source itself: {to_source:?}");
}

#[test]
fn one_backward_search_replaces_one_forward_search_per_enemy() {
    // The use it is for. Three enemies, each with two moves. Which cells are threatened?
    let g = FullGrid::square(11, 11, Adjacency::Four);
    let m = Movement::scan(&g, |_| Some(10));

    let enemies = [Sq::new(1, 1), Sq::new(9, 1), Sq::new(5, 9)].map(|c| g.at(c));

    // For each cell, is any enemy able to arrive within two moves?
    let threatened: Vec<_> = g
        .indices()
        .filter(|&cell| {
            g.reaching(cell, 20, &m)
                .iter()
                .any(|&(from, _)| enemies.contains(&from))
        })
        .collect();

    // The same answer the slow way: union of each enemy's reach.
    let mut expected: Vec<_> = enemies
        .iter()
        .flat_map(|&e| g.reachable(e, 20, &m).into_iter().map(|(i, _)| i))
        .collect();
    expected.sort_unstable();
    expected.dedup();

    assert_eq!(threatened, expected);
    assert!(!threatened.is_empty());
}

#[test]
fn a_blocked_cell_threatens_nobody_and_is_threatened_by_nobody() {
    let g = FullGrid::square(5, 5, Adjacency::Four);
    let wall = g.at(Sq::new(2, 2));
    let m = Movement::scan(&g, |s| (s.to != wall).then_some(10));

    assert_eq!(
        g.reaching(wall, 100, &m).len(),
        1,
        "only itself: nothing can enter a wall"
    );
}

#[test]
fn a_step_that_cannot_be_undone_is_refused_when_the_board_is_built() {
    // The grid keeps no table of in-edges. It finds who steps into a cell by undoing the step, and
    // that only works when a step is one offset everywhere.
    //
    // A `step` that halves its position sends several cells into one. Undoing it cannot find them
    // all, and the cost of a silent miss is an enemy who can reach you but never appears on the
    // threat overlay. So the board is refused, loudly, when it is built.
    common::coord_1d!(Funnel, Down, |x| Funnel(x.0 / 2)); // 4 and 5 both fall into 2

    // A metric of 0, so that the metric check passes and the refusal below is about the step.
    let built = FullGrid::try_new((0..8).map(Funnel), Funnel::DIRS, Metric::scanning(|_, _| 0));

    assert_eq!(built.unwrap_err(), GridError::StepNotInvertible);
}

/// Every edge of a board, read from its far end, must be the same set of edges.
fn assert_in_edges_mirror_out_edges<B: Grid>(b: &B, what: &str) {
    let mut out: Vec<(u32, u32)> = b
        .indices()
        .flat_map(|i| b.neighbors(i).map(move |(_, j)| (i.get(), j.get())))
        .collect();
    let mut inn: Vec<(u32, u32)> = b
        .indices()
        .flat_map(|j| b.in_neighbors(j).map(move |(_, i)| (i.get(), j.get())))
        .collect();
    out.sort_unstable();
    inn.sort_unstable();

    assert!(!out.is_empty(), "{what}: the board has edges to compare");
    assert_eq!(out, inn, "{what}");
}

#[test]
fn who_steps_in_is_exactly_who_steps_out_read_backwards() {
    // `in_neighbors` is computed, not stored, so it has to be held against `neighbors` on every kind
    // of board: both adjacencies, holes, hexes, a region, and steps that clamp or wrap.
    assert_in_edges_mirror_out_edges(&FullGrid::square(6, 5, Adjacency::Four), "four-way");
    assert_in_edges_mirror_out_edges(&FullGrid::square(6, 5, Adjacency::Eight), "eight-way");
    assert_in_edges_mirror_out_edges(
        &FullGrid::square(8, 8, Adjacency::Eight).filtered(|c| (c.x + c.y) % 2 == 1),
        "a board with holes",
    );
    assert_in_edges_mirror_out_edges(
        &FullGrid::hexagon(3).filtered(|c| c != Hex::new(1, -1)),
        "a hexagon with a hole",
    );

    let board = FullGrid::square(7, 7, Adjacency::Eight);
    let blast = board.within(board.at(Sq::new(3, 3)), 1, 2);
    assert_in_edges_mirror_out_edges(&blast, "a region");

    // The last cell steps onto itself, which is not an edge. Every other edge is one offset.
    common::coord_1d!(Clamp, East, |x| Clamp((x.0 + 1).min(4)));
    let clamped = FullGrid::new((0..=4).map(Clamp), Clamp::DIRS, Metric::scanning(|_, _| 0));
    assert_in_edges_mirror_out_edges(&clamped, "a step that clamps at the edge");
    assert_eq!(
        clamped.in_neighbors(clamped.at(Clamp(4))).count(),
        1,
        "the clamped cell is still entered from the cell before it"
    );
}

#[test]
fn reaching_is_deterministic() {
    let g = FullGrid::square(9, 9, Adjacency::Eight);
    let m = Movement::scan(&g, |s| Some(if s.dir.is_diagonal() { 14 } else { 10 }));
    let goal = g.at(Sq::new(4, 4));

    let first = g.reaching(goal, 40, &m);
    for _ in 0..20 {
        assert_eq!(g.reaching(goal, 40, &m), first);
    }
}

#[test]
fn reaching_survives_the_costs_that_used_to_hang_the_search() {
    // It runs the same closure through the same Dijkstra, so it inherits the saturating total for
    // free. That is the payoff for fixing the class rather than the instance.
    let g = FullGrid::square(10, 1, Adjacency::Four);
    let m = Movement::new(|_| Some(700_000_000u32), 0);
    let end = g.at(Sq::new(9, 0));

    assert_eq!(g.reaching(end, u32::MAX, &m).len(), 10);
}
