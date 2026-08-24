//! Acceptance test: a clone-and-jump capture game, on a hex board with holes in it.
//!
//! The rules, in one line: a piece may **clone** into an empty cell one away, or **jump** to an
//! empty cell exactly two away (leaving its origin behind); either way, every enemy adjacent to
//! where it lands defects.
//!
//! The load-bearing case is the jump. A jump is a *coordinate* hop of distance two — it is not two
//! steps along the graph, and it must be able to leap **over a hole in the board**. A depth-2
//! breadth-first search over adjacency gets this wrong twice: it includes the cells one away, and
//! it misses the cells behind a gap. That is why the crate has both [`FullGrid::step`] (a walk, which
//! respects holes) and [`FullGrid::ring`] (a measurement, which does not).

use spacewalk::{FullGrid, Grid, Hex, Idx, SubGrid};

/// A board of radius 4, with a few holes punched in it.
fn board() -> FullGrid<Hex> {
    let holes = [
        Hex::new(0, 0),
        Hex::new(2, -1),
        Hex::new(-1, 2),
        Hex::new(-1, -1),
    ];
    FullGrid::hexagon(4).filtered(|c| !holes.contains(&c))
}

/// Every move a piece at `from` could make, were the board empty: clones, then jumps.
fn moves(g: &FullGrid<Hex>, from: Idx) -> (SubGrid<'_, FullGrid<Hex>>, SubGrid<'_, FullGrid<Hex>>) {
    (g.ring(from, 1), g.ring(from, 2))
}

#[test]
fn a_piece_in_open_ground_has_six_clones_and_twelve_jumps() {
    let g = FullGrid::hexagon(4);
    let mid = g.at(Hex::new(0, 0));
    let (clones, jumps) = moves(&g, mid);

    assert_eq!(clones.len(), 6, "the six neighbours");
    assert_eq!(jumps.len(), 12, "six straight out, six between them");
}

#[test]
fn a_jump_leaps_clean_over_a_hole() {
    let g = board();
    // (1,0) sits beside the hole at (0,0), and (-1,0) is directly opposite across it.
    let from = g.at(Hex::new(1, 0));
    let across = g.at(Hex::new(-1, 0));

    assert_eq!(g.distance(from, across), 2);

    let (_, jumps) = moves(&g, from);
    assert!(
        jumps.contains(g.coord(across)),
        "the cell across the hole is exactly two away, so it is a legal jump"
    );

    // And the walk agrees that you cannot get there on foot: the hole is a dead end.
    assert!(!g.neighbors(from).any(|(_, n)| g.coord(n) == Hex::new(0, 0)));
}

#[test]
fn a_hole_is_never_a_move_target() {
    let g = board();
    let from = g.at(Hex::new(1, 0));
    let (clones, jumps) = moves(&g, from);

    // The hole at (0,0) is adjacent, but it is not a cell, so it cannot be landed on.
    for target in clones.cells().chain(jumps.cells()) {
        assert_ne!(target, Hex::new(0, 0));
    }
}

#[test]
fn a_clone_is_never_also_a_jump() {
    let g = board();
    for i in g.indices() {
        let (clones, jumps) = moves(&g, i);
        for c in clones.cells() {
            assert!(!jumps.contains(c), "a cell cannot be both one and two away");
        }
        assert!(
            !clones.contains(g.coord(i)) && !jumps.contains(g.coord(i)),
            "nor can it be itself"
        );
    }
}

#[test]
fn landing_flips_every_adjacent_enemy() {
    // The board holds no pieces, so the game does. Ownership is a Vec the game owns; the grid only
    // says who is next to whom. This is the whole of the crate's involvement in the game.
    const EMPTY: i8 = -1;
    let g = board();
    let mut owner = vec![EMPTY; g.len()];

    let landing = g.at(Hex::new(1, 0));
    for (_, n) in g.neighbors(landing) {
        owner[n.get() as usize] = 1; // ring it with enemies
    }
    let enemies = g.neighbors(landing).count();
    assert!(enemies > 0);

    // Land, and flip.
    owner[landing.get() as usize] = 0;
    for (_, n) in g.neighbors(landing) {
        if owner[n.get() as usize] == 1 {
            owner[n.get() as usize] = 0;
        }
    }

    assert_eq!(owner.iter().filter(|&&o| o == 0).count(), enemies + 1);
    assert_eq!(owner.iter().filter(|&&o| o == 1).count(), 0, "all defected");
}

#[test]
fn the_board_is_shared_not_copied_when_the_ai_searches() {
    use std::sync::Arc;

    // The reason the grid owns no game state: a search clones *positions*, thousands of them, and
    // must never clone the board. Here the position is a Vec<i8>; the board is shared, and can be
    // read while the position is mutated — which a grid that owned both would forbid.
    let g = Arc::new(board());
    let position = vec![-1i8; g.len()];

    let explore = |mut p: Vec<i8>| {
        for i in g.indices() {
            if let Some((_, n)) = g.neighbors(i).next() {
                p[n.get() as usize] = 0; // read the board, write the position
            }
        }
        p
    };

    let child = explore(position.clone());
    assert_eq!(child.len(), g.len());
    assert_eq!(Arc::strong_count(&g), 1, "the board was never cloned");
}
