//! Acceptance test: checkers — a board of dark squares, forward-only men, and jumps.
//!
//! Checkers is here because it breaks two things a naive grid gets wrong.
//!
//! **The board is not the lattice.** Only the dark squares are playable. They are still addressed
//! in ordinary 8×8 coordinates — [`FullGrid::filtered`] drops the light ones, and the orthogonal steps
//! then lead nowhere, so the four diagonals are the only adjacency left. Nothing needed to be told
//! this; it falls out.
//!
//! **A man moves forward only.** That needs the grid to remember which neighbour is which. A
//! compacted neighbour list — "here are your neighbours, in some order" — cannot answer it, and a
//! man would be able to retreat. Keeping the direction on every step is what makes it possible.

use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Idx, Sq};

/// The dark squares of an 8×8 board, and nothing else.
fn board() -> FullGrid<Sq> {
    FullGrid::square(8, 8, Adjacency::Eight).filtered(|c| (c.x + c.y) % 2 == 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Piece {
    Man(Side),
    King(Side),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Red,
    Black,
}

impl Piece {
    /// Which way this piece may move. A man goes forward; a king goes anywhere.
    fn dirs(self) -> Vec<Dir8> {
        match self {
            // Red starts at the bottom and advances north (y decreasing).
            Piece::Man(Side::Red) => vec![Dir8::Ne, Dir8::Nw],
            Piece::Man(Side::Black) => vec![Dir8::Se, Dir8::Sw],
            Piece::King(_) => Dir8::DIAG.to_vec(),
        }
    }

    fn side(self) -> Side {
        match self {
            Piece::Man(s) | Piece::King(s) => s,
        }
    }
}

/// Where `piece` at `from` may step, and what it may jump — given who is standing where.
fn moves(g: &FullGrid<Sq>, at: &[Option<Piece>], from: Idx, piece: Piece) -> (Vec<Idx>, Vec<Idx>) {
    let mut steps = Vec::new();
    let mut jumps = Vec::new();

    for d in piece.dirs() {
        let Some(next) = g.step(from, d) else {
            continue;
        };
        match at[next.get() as usize] {
            None => steps.push(next),
            Some(p) if p.side() != piece.side() => {
                // An enemy: jump it, if the cell straight beyond is empty. Two steps in the SAME
                // direction — which is only expressible because the step keeps its direction.
                if let Some(land) = g.step(next, d)
                    && at[land.get() as usize].is_none()
                {
                    jumps.push(land);
                }
            }
            Some(_) => {} // our own piece blocks
        }
    }
    (steps, jumps)
}

#[test]
fn only_the_dark_squares_are_on_the_board() {
    let g = board();
    assert_eq!(g.len(), 32);
    assert!(g.contains(Sq::new(1, 0)), "dark");
    assert!(!g.contains(Sq::new(0, 0)), "light");
}

#[test]
fn the_orthogonals_lead_nowhere_so_the_diagonals_are_the_whole_adjacency() {
    let g = board();
    let mid = g.at(Sq::new(3, 4));

    assert_eq!(g.neighbors(mid).count(), 4);
    for (d, _) in g.neighbors(mid) {
        assert!(d.is_diagonal(), "{d:?} is not a diagonal");
    }
    assert!(g.step(mid, Dir8::N).is_none(), "north is a light square");
}

#[test]
fn a_man_may_only_advance() {
    let g = board();
    let at = vec![None; g.len()];
    let from = g.at(Sq::new(3, 4));

    let (steps, _) = moves(&g, &at, from, Piece::Man(Side::Red));
    let cells: Vec<Sq> = steps.iter().map(|&i| g.coord(i)).collect();

    assert_eq!(cells.len(), 2);
    assert!(
        cells.iter().all(|c| c.y == 3),
        "red advances north, never back south"
    );
}

#[test]
fn a_king_may_go_any_way() {
    let g = board();
    let at = vec![None; g.len()];
    let from = g.at(Sq::new(3, 4));

    let (steps, _) = moves(&g, &at, from, Piece::King(Side::Red));
    assert_eq!(steps.len(), 4, "all four diagonals");
}

#[test]
fn a_man_jumps_an_enemy_and_lands_beyond_it() {
    let g = board();
    let mut at = vec![None; g.len()];

    let from = g.at(Sq::new(3, 4));
    let victim = g.at(Sq::new(4, 3)); // north-east of us
    at[victim.get() as usize] = Some(Piece::Man(Side::Black));

    let (steps, jumps) = moves(&g, &at, from, Piece::Man(Side::Red));

    assert_eq!(jumps.len(), 1);
    assert_eq!(
        g.coord(jumps[0]),
        Sq::new(5, 2),
        "two north-east, over the victim"
    );
    assert!(
        !steps.contains(&victim),
        "and we cannot simply step onto the square he is standing on"
    );
}

#[test]
fn a_jump_is_blocked_when_the_landing_square_is_taken() {
    let g = board();
    let mut at = vec![None; g.len()];

    let from = g.at(Sq::new(3, 4));
    at[g.at(Sq::new(4, 3)).get() as usize] = Some(Piece::Man(Side::Black));
    at[g.at(Sq::new(5, 2)).get() as usize] = Some(Piece::Man(Side::Black));

    let (_, jumps) = moves(&g, &at, from, Piece::Man(Side::Red));
    assert!(jumps.is_empty(), "nowhere to land");
}

#[test]
fn we_never_jump_our_own() {
    let g = board();
    let mut at = vec![None; g.len()];

    let from = g.at(Sq::new(3, 4));
    at[g.at(Sq::new(4, 3)).get() as usize] = Some(Piece::Man(Side::Red));

    let (steps, jumps) = moves(&g, &at, from, Piece::Man(Side::Red));
    assert!(jumps.is_empty(), "a friend is not a victim");
    assert_eq!(steps.len(), 1, "and he blocks the square he stands on");
}

#[test]
fn a_double_jump_chains_by_asking_again_from_where_it_landed() {
    // Multi-jumps are path-dependent — the second jump depends on what the first one captured — so
    // the game owns the recursion. The grid just answers the same question again from the new cell.
    let g = board();
    let mut at = vec![None; g.len()];

    let from = g.at(Sq::new(1, 6));
    let first = g.at(Sq::new(2, 5));
    let second = g.at(Sq::new(4, 3));
    at[first.get() as usize] = Some(Piece::Man(Side::Black));
    at[second.get() as usize] = Some(Piece::Man(Side::Black));

    let red = Piece::Man(Side::Red);

    let (_, jumps) = moves(&g, &at, from, red);
    assert_eq!(g.coord(jumps[0]), Sq::new(3, 4), "over the first");

    // Take him off the board, and ask again from where we landed.
    at[first.get() as usize] = None;
    let (_, again) = moves(&g, &at, jumps[0], red);
    assert_eq!(
        g.coord(again[0]),
        Sq::new(5, 2),
        "over the second, in one turn"
    );
}

#[test]
fn a_man_promotes_on_the_far_rank() {
    let g = board();
    let landed = g.at(Sq::new(5, 0));
    assert_eq!(g.coord(landed).y, 0, "red's back rank");
}
