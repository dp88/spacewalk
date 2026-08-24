//! Corner-cutting: whether a diagonal may squeeze past an obstacle.
//!
//! This lives outside [`Grid`] on purpose. Whether the north-east step exists depends on whether
//! the north and east cells are passable *right now, for this piece* — which is your game's state,
//! not the board's geometry. So the rule is a gate you put in front of your own cost function, and
//! the grid stays pure geometry.

use crate::coord::{Idx, Sq};
use crate::grid::Grid;
use crate::path::Step;

/// Whether a diagonal step may pass an obstacle on its corner.
///
/// Going from `A` to `D`, with `B` and `C` the two cells it squeezes between:
///
/// ```text
///   [B][D]
///   [A][C]
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerRule {
    /// Both `B` and `C` must be open. No slipping past the corner of a wall, and no squeezing
    /// between two of them. What most tactics games do, and the default.
    #[default]
    Strict,
    /// One of `B` or `C` will do. You may clip a single corner, but not thread a diagonal gap
    /// between two obstacles.
    Loose,
    /// Diagonals always work, walls notwithstanding.
    Free,
}

/// A gate that answers: may this step be taken, given the corner rule?
///
/// `enterable` is asked about the *flanking* cells — the ones the diagonal squeezes between. Give
/// it the same test your cost function uses, so a diagonal cannot slip past something a straight
/// step would refuse to enter.
///
/// Orthogonal steps always pass. So do all steps on a four-way grid, which has no diagonals.
///
/// ```
/// use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq, Step};
/// use spacewalk::square::{corner_gate, CornerRule};
///
/// let g = FullGrid::square(3, 3, Adjacency::Eight);
///
/// // Two walls, meeting at the corner north-east of the origin.
/// let walls = [Sq::new(1, 0), Sq::new(0, 1)];
/// let open = |i| !walls.contains(&g.coord(i));
///
/// let strict = corner_gate(&g, CornerRule::Strict, open);
/// let free   = corner_gate(&g, CornerRule::Free,   open);
///
/// let from = g.at(Sq::new(0, 0));
/// let to   = g.at(Sq::new(1, 1));
///
/// let strict_walk = Movement::scan(&g, |s| (open(s.to) && strict(s)).then_some(10));
/// let free_walk   = Movement::scan(&g, |s| (open(s.to) && free(s)).then_some(10));
///
/// // Strict corners: the diagonal is sealed, and there is no way through at all.
/// assert!(g.path(from, to, &strict_walk).is_none());
///
/// // Free corners: you slip between the two walls in a single step.
/// assert_eq!(free_walk.min_step(), 10);
/// assert_eq!(g.path(from, to, &free_walk).unwrap().len(), 1);
/// ```
pub fn corner_gate<'a, B: Grid<Cell = Sq> + ?Sized>(
    g: &'a B,
    rule: CornerRule,
    enterable: impl Fn(Idx) -> bool + 'a,
) -> impl Fn(Step<Sq>) -> bool + 'a {
    move |s| {
        let Some((a, b)) = s.dir.flanks() else {
            return true; // an orthogonal step has no corner to cut
        };
        let open = |d| g.step(s.from, d).is_some_and(&enterable);

        match rule {
            CornerRule::Strict => open(a) && open(b),
            CornerRule::Loose => open(a) || open(b),
            CornerRule::Free => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full::{Adjacency, FullGrid};
    use crate::path::{Cost, Movement};

    /// A 3×3 grid with walls, and a walk that honours `rule`. Returns the path length from the
    /// top-left corner to the middle, if there is one.
    fn diagonal_through(walls: &[Sq], rule: CornerRule) -> Option<usize> {
        let g = FullGrid::square(3, 3, Adjacency::Eight);
        let open = |i: Idx| !walls.contains(&g.coord(i));
        let gate = corner_gate(&g, rule, open);
        let m = Movement::scan(&g, |s| (open(s.to) && gate(s)).then_some(10 as Cost));

        let from = g.at(Sq::new(0, 0));
        let to = g.at(Sq::new(1, 1));
        g.path(from, to, &m).map(|p| p.len())
    }

    #[test]
    fn with_no_walls_every_rule_takes_the_diagonal() {
        for rule in [CornerRule::Strict, CornerRule::Loose, CornerRule::Free] {
            assert_eq!(diagonal_through(&[], rule), Some(1), "{rule:?}");
        }
    }

    #[test]
    fn strict_corners_stop_a_unit_clipping_past_a_single_wall() {
        // One wall on the north flank. Strict refuses the diagonal and goes the long way round;
        // loose and free clip the corner.
        use CornerRule::{Free, Loose, Strict};
        for (rule, n) in [(Strict, 2), (Loose, 1), (Free, 1)] {
            let len = diagonal_through(&[Sq::new(1, 0)], rule);
            assert_eq!(len, Some(n), "{rule:?}");
        }
    }

    #[test]
    fn only_free_corners_squeeze_between_two_walls() {
        // Both flanks walled: the diagonal gap is sealed to everything but Free, and with both
        // orthogonal routes gone there is no way in at all.
        let both = [Sq::new(1, 0), Sq::new(0, 1)];
        assert_eq!(diagonal_through(&both, CornerRule::Strict), None);
        assert_eq!(diagonal_through(&both, CornerRule::Loose), None);
        assert_eq!(diagonal_through(&both, CornerRule::Free), Some(1));
    }

    #[test]
    fn a_unit_standing_on_a_corner_blocks_it_just_as_a_wall_does() {
        // Occupancy flows through the same predicate as terrain, so a body in the gap stops the
        // squeeze. BattleCore checked only terrain here, and let units slide past each other.
        assert_eq!(
            diagonal_through(&[Sq::new(1, 0)], CornerRule::Strict),
            Some(2)
        );
    }

    #[test]
    fn orthogonal_steps_never_have_a_corner_to_cut() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let gate = corner_gate(&g, CornerRule::Strict, |_| false);
        let from = g.at(Sq::new(1, 1));

        // Even with every flank refused, the four orthogonals pass — they have no flanks.
        for (dir, to) in g.neighbors(from) {
            assert!(gate(Step { from, to, dir }), "{dir:?}");
        }
    }
}
