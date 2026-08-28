//! Elevation: what a hill hides, and what a ledge refuses.
//!
//! Height is game state, not geometry. A cell's ground level belongs beside its terrain and its
//! occupant, in whatever you already keep those in — a [`CellMap`](crate::CellMap) is the usual
//! answer. So this module holds no heights. It holds the two rules that read them, as gates you put
//! in front of your own closures, exactly as [`corner_gate`](crate::corner_gate) does. The grid
//! stays pure geometry.
//!
//! Both gates are integer throughout. Sight and cost must give the same answer on every machine
//! that replays the same game, and a float would not promise that.

use crate::coord::{Coord, Idx};
use crate::grid::{Grid, Sight};
use crate::path::Step;

/// A gate that answers: does the cell under test rise high enough to stop this view?
///
/// Returns a **blocker** predicate, so it goes straight into [`Grid::los_by`] and
/// [`Grid::visible_from_by`]. Sight runs in a straight line from the eye to the target, and a cell
/// blocks when whatever stands on it pokes above that line.
///
/// `top` is the height sight must clear at a cell — the ground plus whatever sits on it. `looks_from`
/// is where an eye or a mark sits at a cell, which is normally the ground plus a body. A cell may
/// well be both: a unit on a tower stands *at* the tower's height and is hidden *by* nothing lower.
///
/// ```
/// use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Sq};
/// use spacewalk::height::height_gate;
///
/// let g = FullGrid::square(9, 3, Adjacency::Eight);
///
/// // Flat ground, with one hill five units high in the middle of it.
/// let mut ground = CellMap::new(&g, 0i32);
/// ground[g.at(Sq::new(4, 1))] = 5;
///
/// let eye = g.at(Sq::new(0, 1));
/// let across = g.at(Sq::new(8, 1));
///
/// // Everyone stands one unit above the ground they are on.
/// let sight = height_gate(&g, |i| ground[i], |i| ground[i] + 1);
/// assert!(!g.los_by(eye, across, &sight), "the hill is in the way");
///
/// // A tower is terrain, so it raises whoever stands on it — and the hill hides nothing.
/// let tower = |i| ground[i] + if i == eye { 20 } else { 1 };
/// assert!(g.los_by(eye, across, height_gate(&g, |i| ground[i], tower)));
/// ```
///
/// # One height per cell, so sight stays symmetric
///
/// `looks_from` is asked about the eye and about the target, and it is the **same** closure for
/// both. That is what keeps A seeing B exactly when B sees A. Writing `T` for `top(at)`, `A` for the
/// eye's height and `B` for the target's, the forward test below is `(T - A)·n > (B - A)·t`; the
/// reverse test is `(T - B)·n > (A - B)·(n - t)`, and it reduces to the same inequality. Separate
/// eye and target heights would not, and one-sided sight is a thing players notice — it is why
/// [`Grid::line`] goes to the trouble of computing from the lower coordinate and flipping.
///
/// A rule that really is asymmetric — a searchlight, a unit that can only peek downhill — wants its
/// own gate rather than this one. It is a few lines; take the arithmetic below as the starting
/// point.
///
/// # The arithmetic
///
/// With `t` the distance from the eye to the cell under test and `n` the distance from the eye to
/// the target, the sight line stands at `A + (B - A)·t/n`, and the cell blocks when `top` exceeds
/// it. Cross-multiplied to clear the division:
///
/// ```text
/// (top(at) - looks_from(eye)) * n  >  (looks_from(target) - looks_from(eye)) * t
/// ```
///
/// Computed in `i128`. The widest `i32` height difference times the widest `u32` distance needs 65
/// bits, which is one more than an `i64` has — an `i64` here would wrap on a hostile save file
/// rather than merely give a strange answer.
///
/// This assumes a cell on the line lies between its endpoints, `distance(eye, at) +
/// distance(at, target) == distance(eye, target)`. Every metric this crate ships upholds it. A
/// custom metric that does not wants its own gate.
pub fn height_gate<'a, B: Grid + ?Sized>(
    g: &'a B,
    top: impl Fn(Idx) -> i32 + 'a,
    looks_from: impl Fn(Idx) -> i32 + 'a,
) -> impl Fn(Sight) -> bool + 'a {
    move |s| {
        let eye = i128::from(looks_from(s.eye));
        let rise = i128::from(top(s.at)) - eye;
        let fall = i128::from(looks_from(s.target)) - eye;

        rise * i128::from(g.distance(s.eye, s.target)) > fall * i128::from(g.distance(s.eye, s.at))
    }
}

/// A gate that answers: may this step be taken, given how far it climbs?
///
/// Returns a predicate over a [`Step`], so it composes with your cost function the way
/// [`corner_gate`](crate::corner_gate) does. `z` is the ground level of a cell, and `max_rise` the
/// greatest ascent one step may make. Zero forbids all climbing. A negative value also refuses
/// level ground and any descent shallower than its magnitude.
///
/// With an ordinary, non-negative limit, **descent is unrestricted**. A cliff you cannot climb is a
/// cliff you may still drop off, which is the one-way ledge the crate's directed edges exist for.
/// If falling should hurt, or be refused past some drop, that is your cost function's business and
/// not this gate's.
///
/// It takes no grid: a step already names both cells, and their heights are all this rule reads.
///
/// ```
/// use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Movement, Sq};
/// use spacewalk::height::climb_gate;
///
/// let g = FullGrid::square(4, 1, Adjacency::Four);
/// let mut ground = CellMap::new(&g, 0i32);
/// ground[g.at(Sq::new(2, 0))] = 3;             // a ledge, three units up
///
/// let climb = climb_gate(|i| ground[i], 1);    // one unit is all anyone can haul themselves up
/// let walk = Movement::scan(&g, |s| climb(s).then_some(10));
///
/// let west = g.at(Sq::new(0, 0));
/// let ledge = g.at(Sq::new(2, 0));
///
/// assert!(g.path(west, ledge, &walk).is_none(), "three units is beyond climbing");
/// assert_eq!(g.path(ledge, west, &walk).unwrap().len(), 2, "but you may always drop off it");
/// ```
pub fn climb_gate<'a, C: Coord>(
    z: impl Fn(Idx) -> i32 + 'a,
    max_rise: i32,
) -> impl Fn(Step<C>) -> bool + 'a {
    move |s| i64::from(z(s.to)) - i64::from(z(s.from)) <= i64::from(max_rise)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cells::CellMap;
    use crate::coord::Sq;
    use crate::full::{Adjacency, FullGrid};
    use crate::path::{Cost, Movement};
    use alloc::vec::Vec;

    /// A 9x3 board, flat but for one hill of `hill` units at the middle of the centre row.
    fn ridge(hill: i32) -> (FullGrid<Sq>, CellMap<i32>) {
        let g = FullGrid::square(9, 3, Adjacency::Eight);
        let mut ground = CellMap::new(&g, 0i32);
        ground[g.at(Sq::new(4, 1))] = hill;
        (g, ground)
    }

    #[test]
    fn a_hill_hides_what_is_behind_it() {
        let (g, ground) = ridge(5);
        let sight = height_gate(&g, |i| ground[i], |i| ground[i] + 1);

        let eye = g.at(Sq::new(0, 1));
        assert!(
            !g.los_by(eye, g.at(Sq::new(8, 1)), &sight),
            "across the hill"
        );
        assert!(g.los_by(eye, g.at(Sq::new(3, 1)), &sight), "short of it");
    }

    #[test]
    fn enough_height_sees_over_the_hill() {
        let (g, ground) = ridge(5);
        let eye = g.at(Sq::new(0, 1));

        // The tower is terrain, so it lifts the eye standing on it. Ten units clears a five-unit
        // hill at the halfway mark with room to spare.
        let raised = |i: Idx| ground[i] + if i == eye { 10 } else { 1 };
        let sight = height_gate(&g, |i| ground[i], raised);

        assert!(g.los_by(eye, g.at(Sq::new(8, 1)), &sight));
    }

    #[test]
    fn you_can_always_see_the_hilltop_you_are_looking_at() {
        // The target is exempt, height or no height. Otherwise a hill would hide itself.
        let (g, ground) = ridge(500);
        let sight = height_gate(&g, |i| ground[i], |i| ground[i] + 1);

        let eye = g.at(Sq::new(0, 1));
        assert!(g.los_by(eye, g.at(Sq::new(4, 1)), &sight));
    }

    #[test]
    fn sight_over_a_height_field_is_symmetric() {
        // The property the one-closure design exists to keep. Checked over every ordered pair, the
        // way `tests/fov.rs` checks it for plain sight.
        let g = FullGrid::square(11, 11, Adjacency::Eight);
        let ground = CellMap::from_fn(&g, |c: Sq| (c.x * 7 + c.y * 13) % 9);
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
    fn a_climb_gate_measures_the_step_and_not_the_height() {
        // A staircase of single units climbs as far as you like. The same total rise in one step
        // does not. This is the difference a per-step rule makes, and the reason it is a gate on a
        // `Step` rather than a test on a cell.
        let g = FullGrid::square(5, 1, Adjacency::Four);
        let stairs = CellMap::from_fn(&g, |c: Sq| c.x);
        let climb = climb_gate(|i| stairs[i], 1);
        let walk = Movement::scan(&g, |s| climb(s).then_some(10 as Cost));

        let (bottom, top) = (g.at(Sq::new(0, 0)), g.at(Sq::new(4, 0)));
        assert_eq!(g.path(bottom, top, &walk).map(|p| p.len()), Some(4));

        // The same four units, taken as one step, is refused.
        let cliff: Vec<i32> = (0..5).map(|x| if x == 4 { 4 } else { 0 }).collect();
        let steep = climb_gate(|i: Idx| cliff[i.raw() as usize], 1);
        let hard = Movement::scan(&g, |s| steep(s).then_some(10 as Cost));
        assert!(g.path(bottom, top, &hard).is_none());
    }
}
