//! Paths and reach, over a cost function you supply.
//!
//! # Cost is what it costs to *enter* a cell, from a *direction*
//!
//! Not the cost of the cell, and not the cost of an edge. The distinction is what lets a grid hold
//! a river: entering the river cell heading downstream is cheap, heading upstream is dear. The
//! cost belongs to the river; the direction you arrive from picks which one applies. A conveyor
//! belt and a ledge you can drop off but not climb back up are the same shape.
//!
//! So the graph is **directed**, and three things follow that you must not forget:
//!
//! - A [`Path`] **cannot be reversed**. `path(a, b)` is not `path(b, a)` walked backwards.
//! - [`reachable`](Grid::reachable) means reachable-*out*. That you can get somewhere does not
//!   mean you can get back.
//! - Costs must be **non-negative** — a requirement of Dijkstra and A\*, not a matter of taste.
//!
//! # One function does the work of four
//!
//! The closure you hand [`Movement`] returns `Option<Cost>`, and `None` means "you cannot come in
//! that way". That single answer covers terrain cost, impassable terrain, cells blocked by other
//! pieces, and one-way movement — four separate mechanisms in most engines, and one here.
//!
//! ```
//! use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Movement, Sq};
//!
//! let g = FullGrid::square(8, 8, Adjacency::Four);
//! let wall = g.at(Sq::new(4, 4));
//! let river = g.at(Sq::new(2, 2));
//!
//! let walk = Movement::scan(&g, |s| match s.to {
//!     t if t == wall  => None,                                  // impassable
//!     t if t == river => Some(if s.dir == Dir8::S { 1 } else { 50 }), // the current runs south
//!     _ => Some(10),                                            // open ground
//! });
//!
//! let start = g.at(Sq::new(2, 1));
//! let end   = g.at(Sq::new(2, 3));
//!
//! // Downstream, straight through the river.
//! assert_eq!(g.path(start, end, &walk).unwrap().cost(), 11);
//!
//! // Upstream, the current is dear enough that it is cheaper to go around.
//! assert_eq!(g.path(end, start, &walk).unwrap().cost(), 40);
//! ```

use std::fmt;

use crate::coord::{Coord, Idx};
use crate::grid::{Grid, cost_ceiling};

/// What a step costs.
///
/// An integer, because A\* needs a totally ordered cost and floats are not. **The scale is yours.**
/// The usual choice is to make an ordinary step 10, which leaves room underneath for a road at 5
/// and a diagonal at 14 (√2, near enough).
///
/// # Keep them small
///
/// A path's total must fit in a [`Cost`]. [`Movement::scan`] checks that for you and panics if your
/// costs are too large for the board, so in practice you will be told. See [`Movement::new`] for the
/// unchecked way, and why you probably do not want it.
pub type Cost = u32;

/// One step: leaving `from`, entering `to`, travelling in `dir`.
///
/// `dir` is handed to you rather than derived, so a cost function never has to work out which way
/// it is facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step<C: Coord> {
    /// The cell being left. Never charged for — you are already standing on it.
    pub from: Idx,
    /// The cell being entered. This is the one the cost belongs to.
    pub to: Idx,
    /// The direction of travel, `from` towards `to`. This is what makes a river possible.
    pub dir: C::Dir,
}

/// Your game's movement rules: what a step costs, and what the cheapest step on the board is.
///
/// `enter` must be **pure** — the same step must always cost the same. A\* asks about a step more
/// than once, and a closure that answers differently each time can be made to "improve" a route
/// forever. Reading your own terrain and unit tables is exactly right; a random number generator or
/// a counter behind a `Cell` is not.
pub struct Movement<F> {
    enter: F,
    min_step: Cost,
}

impl<F> fmt::Debug for Movement<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Movement")
            .field("min_step", &self.min_step)
            .finish_non_exhaustive()
    }
}

impl<F> Movement<F> {
    /// Work out the cheapest step on the board by looking at every one of them.
    ///
    /// This is the constructor to use, for two reasons.
    ///
    /// `min_step` scales the A\* heuristic, and a value even one too high makes the heuristic
    /// overestimate — at which point A\* still returns *a* path, but no longer the cheapest one, and
    /// says nothing about it. Measuring is O(cells × directions), which is cheap and always right.
    ///
    /// And since it is walking every edge anyway, it also checks that your costs are **small enough
    /// for this board** — see the panic below. That check is why `scan` is worth its cost.
    ///
    /// # Build it when the board changes, not when the frame does
    ///
    /// It is O(cells × directions), which is nothing *once*: 369µs on a 128×128 eight-way board.
    /// It is not nothing sixty times a second. Twenty units each rebuilding their own `Movement`
    /// every frame is 7.4ms of a 16.7ms budget, spent re-measuring a board that did not move.
    ///
    /// So hold on to it. A `Movement` depends on the board and your terrain, not on whose turn it
    /// is — rebuild it when *those* change. If you genuinely must build one in a hot loop, and you
    /// already know the cheapest step your rules can produce, [`Movement::new`] skips the walk.
    ///
    /// # Panics
    ///
    /// If a step costs so much that a path across this board could exceed [`Cost::MAX`]. The ceiling
    /// is `Cost::MAX / (cells - 1)`, because no simple path visits a cell twice — on a 40,000-cell
    /// board that still allows any step up to about 107,000, which is generous at any sane scale.
    ///
    /// This is a loud, early failure by design. The alternative is a total that silently overflows
    /// deep inside the search, and *that* does not give you a wrong number — it gives you a hang.
    /// See the module docs for why a wrapped total is so much worse than a wrong one.
    pub fn scan<B: Grid + ?Sized>(g: &B, enter: F) -> Self
    where
        F: Fn(Step<B::Cell>) -> Option<Cost>,
    {
        let costs: Vec<Cost> = g
            .indices()
            .flat_map(|from| {
                g.neighbors(from)
                    .map(move |(dir, to)| Step { from, to, dir })
            })
            .filter_map(&enter)
            .collect();

        let ceiling = cost_ceiling(g.len());
        if let Some(&worst) = costs.iter().max() {
            assert!(
                worst <= ceiling,
                "a step costs {worst}, but on a board of {} cells no step may cost more than \
                 {ceiling} or a path's total could overflow Cost ({}). Scale your costs down.",
                g.len(),
                Cost::MAX,
            );
        }

        Self {
            enter,
            min_step: costs.into_iter().min().unwrap_or(0),
        }
    }

    /// Promise the cheapest step yourself, and skip the scan.
    ///
    /// Faster to build, and **unchecked in both directions**. Promise a `min_step` that is too high
    /// and the heuristic overestimates, so A\* quietly stops returning the cheapest path. And unlike
    /// [`Movement::scan`], nothing here verifies your costs are small enough for the board.
    ///
    /// The search will not overflow even so — the running total saturates rather than wraps, so it stays
    /// bounded and terminates — but a saturated total is a wrong total. When in doubt, promise 0:
    /// the heuristic goes to nothing, A\* degrades into Dijkstra, and the answer stays correct.
    #[must_use]
    pub fn new(enter: F, min_step: Cost) -> Self {
        Self { enter, min_step }
    }

    /// The cheapest step this movement can make anywhere on the board.
    #[must_use]
    pub fn min_step(&self) -> Cost {
        self.min_step
    }

    /// What one step costs, or `None` where the rules forbid it.
    ///
    /// Crate-internal: the searches ask, callers answer. Exposing it would invite a caller to price
    /// a step that no board would ever offer.
    pub(crate) fn enter<C: Coord>(&self, s: Step<C>) -> Option<Cost>
    where
        F: Fn(Step<C>) -> Option<Cost>,
    {
        (self.enter)(s)
    }
}

impl Movement<()> {
    /// Every step costs the same, and every cell is passable.
    ///
    /// The prototype's movement rules, and the right answer whenever the board itself is the only
    /// constraint — a reachability question, a connectivity check, a distance in steps rather than
    /// in terrain. It is exact rather than measured: the cheapest step is the only step, so there is
    /// nothing to walk the board for, and [`Movement::scan`]'s O(cells × directions) pass is skipped.
    ///
    /// The board is still borrowed, because the ceiling below depends on how big it is.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let walk = Movement::uniform(&g, 1);
    ///
    /// let p = g.path(g.at(Sq::new(0, 0)), g.at(Sq::new(7, 7)), &walk).unwrap();
    /// assert_eq!(p.len(), 14);
    /// assert_eq!(p.cost(), 14);
    /// ```
    ///
    /// # Panics
    ///
    /// If `cost` is so large that a path across this board could exceed [`Cost::MAX`] — the same
    /// ceiling [`Movement::scan`] enforces, for the same reason, checked here in one comparison
    /// rather than one per edge.
    #[must_use]
    pub fn uniform<B: Grid + ?Sized>(
        g: &B,
        cost: Cost,
    ) -> Movement<impl Fn(Step<B::Cell>) -> Option<Cost>> {
        let ceiling = cost_ceiling(g.len());
        assert!(
            cost <= ceiling,
            "a step costs {cost}, but on a board of {} cells no step may cost more than {ceiling} \
             or a path's total could overflow Cost ({}). Scale your costs down.",
            g.len(),
            Cost::MAX,
        );

        Movement {
            enter: move |_| Some(cost),
            min_step: cost,
        }
    }
}

/// A route, and what walking it costs.
///
/// [`steps`](Path::steps)`[0]` is where you started — you are never charged for the cell you are
/// already on — and the last is where you end up. A path of one cell is a path that goes nowhere,
/// at no cost.
///
/// **Do not reverse it.** The graph is directed; the way back may be dearer, or may not exist.
///
/// # Only a search builds one
///
/// The fields are private, and that is what makes [`destination`](Path::destination) able to return
/// an [`Idx`] rather than an `Option<Idx>`. A `Path` always has at least one step because the only
/// things that make one are the searches in this module, and they always set out from somewhere.
/// When the fields were public that guarantee was not the crate's to give — a caller could write
/// `Path { steps: vec![], cost: 0 }`, or a route through cells that do not touch — so every method
/// had to defend against a value the type should never have allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    steps: Vec<Idx>,
    cost: Cost,
}

impl Path {
    /// Build one. Crate-internal: a route is something a search found, not something you assert.
    pub(crate) fn of(steps: Vec<Idx>, cost: Cost) -> Self {
        debug_assert!(!steps.is_empty(), "a search always sets out from somewhere");
        Self { steps, cost }
    }

    /// Every cell walked through, starting with the one you set out from.
    ///
    /// Use [`Grid::coords_of`] to turn these into coordinates for drawing.
    #[must_use]
    pub fn steps(&self) -> &[Idx] {
        &self.steps
    }

    /// What walking it costs. Entering cells only — standing still is free.
    #[must_use]
    pub fn cost(&self) -> Cost {
        self.cost
    }

    /// Where the path ends.
    #[must_use]
    pub fn destination(&self) -> Idx {
        *self
            .steps
            .last()
            .expect("a Path always has a first cell, so it always has a last")
    }

    /// Where the path sets out from.
    #[must_use]
    pub fn start(&self) -> Idx {
        self.steps[0]
    }

    /// How many cells are actually moved through — one fewer than [`steps`](Path::steps), which
    /// counts the cell you set out from.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len() - 1
    }

    /// Whether the path goes nowhere: it starts and ends on the same cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::{Dir8, Sq};
    use crate::full::{Adjacency, FullGrid};

    /// Open ground: every step costs 10.
    fn open(g: &FullGrid<Sq>) -> Movement<impl Fn(Step<Sq>) -> Option<Cost>> {
        Movement::scan(g, |_| Some(10))
    }

    #[test]
    fn a_path_includes_its_start_and_is_never_charged_for_it() {
        let g = FullGrid::square(5, 5, Adjacency::Four);
        let a = g.at(Sq::new(0, 0));
        let b = g.at(Sq::new(2, 0));

        let p = g.path(a, b, &open(&g)).unwrap();
        assert_eq!(p.steps().len(), 3, "start, middle, end");
        assert_eq!(p.steps()[0], a);
        assert_eq!(p.destination(), b);
        assert_eq!(p.len(), 2, "two cells moved through");
        assert_eq!(
            p.cost(),
            20,
            "two steps at 10, and nothing for standing still"
        );
    }

    #[test]
    fn a_path_to_where_you_already_are_costs_nothing() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let a = g.at(Sq::new(1, 1));

        let p = g.path(a, a, &open(&g)).unwrap();
        assert_eq!(p.cost(), 0);
        assert_eq!(p.steps(), vec![a]);
        assert!(p.is_empty());
    }

    #[test]
    fn a_walled_off_cell_has_no_path_to_it() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let target = g.at(Sq::new(2, 2));

        // Wall off the corner completely.
        let walls = [Sq::new(1, 2), Sq::new(2, 1)];
        let m = Movement::scan(&g, |s| (!walls.contains(&g.coord(s.to))).then_some(10));

        assert!(g.path(g.at(Sq::new(0, 0)), target, &m).is_none());
    }

    #[test]
    fn a_cheap_road_is_worth_a_detour() {
        let g = FullGrid::square(5, 3, Adjacency::Four);
        // A road runs along the top row at a third the cost of the mud below it.
        let m = Movement::scan(&g, |s| Some(if g.coord(s.to).y == 0 { 10 } else { 30 }));

        let a = g.at(Sq::new(0, 1));
        let b = g.at(Sq::new(4, 1));

        // Straight through the mud is 4 x 30 = 120. Up onto the road and back down is
        // 10 (up) + 4 x 10 (along) + 30 (down) = 80.
        let p = g.path(a, b, &m).unwrap();
        assert_eq!(p.cost(), 80);
        assert!(
            p.steps().iter().any(|&i| g.coord(i).y == 0),
            "it used the road"
        );
    }

    #[test]
    fn min_step_is_the_cheapest_step_anywhere_and_a_road_drags_it_down() {
        let g = FullGrid::square(4, 4, Adjacency::Four);
        let m = Movement::scan(&g, |s| {
            Some(if g.coord(s.to) == Sq::new(0, 0) {
                5
            } else {
                10
            })
        });

        // The heuristic must assume the cheapest step, not the typical one. This is precisely the
        // bug BattleCore has: it assumes 1.0 while its own roads cost 0.5.
        assert_eq!(m.min_step(), 5);
    }

    #[test]
    fn an_impassable_board_yields_a_zero_minimum_and_still_answers() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let m = Movement::scan(&g, |_| None);

        let (from, to) = (g.at(Sq::new(0, 0)), g.at(Sq::new(2, 2)));
        assert_eq!(m.min_step(), 0, "no steps at all: promise nothing");
        assert!(g.path(from, to, &m).is_none());
        assert_eq!(
            g.reachable(from, 100, &m),
            vec![(from, 0)],
            "you can still stand still"
        );
    }

    #[test]
    fn reach_is_bounded_by_the_budget() {
        let g = FullGrid::square(9, 9, Adjacency::Four);
        let centre = g.at(Sq::new(4, 4));
        let m = open(&g);

        // A Manhattan diamond of radius n holds 2n(n+1)+1 cells.
        for n in 0..4u32 {
            let want = 2 * n * (n + 1) + 1;
            assert_eq!(
                g.reachable(centre, n * 10, &m).len() as u32,
                want,
                "budget {n}"
            );
        }
    }

    #[test]
    fn reach_comes_back_cheapest_first() {
        let g = FullGrid::square(5, 5, Adjacency::Four);
        let costs: Vec<Cost> = g
            .reachable(g.at(Sq::new(2, 2)), 40, &open(&g))
            .iter()
            .map(|&(_, c)| c)
            .collect();

        assert!(costs.windows(2).all(|w| w[0] <= w[1]), "{costs:?}");
        assert_eq!(costs[0], 0, "you are the first thing you can reach");
    }

    #[test]
    fn path_toward_closes_the_distance_when_it_cannot_arrive() {
        let g = FullGrid::square(10, 1, Adjacency::Four);
        let start = g.at(Sq::new(0, 0));
        let far = g.at(Sq::new(9, 0));

        // Two moves' worth of budget against a target nine cells away.
        let p = g.path_toward(start, far, 20, &open(&g)).unwrap();
        assert_eq!(
            g.coord(p.destination()),
            Sq::new(2, 0),
            "as near as it can get"
        );
        assert_eq!(p.cost(), 20);
    }

    #[test]
    fn path_toward_arrives_when_the_target_is_in_reach() {
        let g = FullGrid::square(10, 1, Adjacency::Four);
        let start = g.at(Sq::new(0, 0));
        let near = g.at(Sq::new(2, 0));

        let p = g.path_toward(start, near, 100, &open(&g)).unwrap();
        assert_eq!(p.destination(), near);
    }

    #[test]
    fn path_toward_stays_put_when_it_is_already_as_close_as_it_can_be() {
        let g = FullGrid::square(3, 1, Adjacency::Four);
        let a = g.at(Sq::new(0, 0));

        // Nowhere to go: every step costs more than the budget.
        let m = Movement::new(|_| Some(10), 10);
        let p = g.path_toward(a, g.at(Sq::new(2, 0)), 0, &m).unwrap();

        assert_eq!(p.destination(), a);
        assert_eq!(p.cost(), 0);
    }

    #[test]
    fn a_diagonal_costs_more_than_an_orthogonal_if_you_say_so() {
        let g = FullGrid::square(5, 5, Adjacency::Eight);
        // √2, on a scale where an orthogonal step is 10.
        let m = Movement::scan(&g, |s| Some(if s.dir.is_diagonal() { 14 } else { 10 }));

        let a = g.at(Sq::new(0, 0));
        let b = g.at(Sq::new(2, 2));

        // Two diagonals (28) beat four orthogonals (40).
        assert_eq!(g.path(a, b, &m).unwrap().cost(), 28);
        assert_eq!(m.min_step(), 10);
    }

    #[test]
    fn a_one_way_ledge_can_be_dropped_off_but_not_climbed() {
        let g = FullGrid::square(1, 3, Adjacency::Four);
        let top = g.at(Sq::new(0, 0));
        let bottom = g.at(Sq::new(0, 2));

        // You may only ever travel south.
        let m = Movement::scan(&g, |s| (s.dir == Dir8::S).then_some(10));

        assert_eq!(g.path(top, bottom, &m).unwrap().cost(), 20, "down is fine");
        assert!(g.path(bottom, top, &m).is_none(), "up is not");
    }
}
