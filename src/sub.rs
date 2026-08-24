//! A region of a board, which is a board.
//!
//! A movement range, a blast, a room, what a unit can see. Each is a set of cells you want to
//! *show* and then *reason about* — and reasoning about it means asking the same questions you ask
//! a board. So it is one: [`SubGrid`] implements [`Grid`] exactly as [`FullGrid`](crate::FullGrid) does, and a
//! function that takes one takes either.
//!
//! It costs a sorted `Vec<Idx>` and nothing else. Every question is answered by the root board and
//! filtered through membership, which is why [`within`](Grid::within),
//! [`component`](Grid::component) and [`visible_from`](Grid::visible_from) can hand one back
//! without building anything.

use crate::coord::{Idx, Metric, Tag};
use crate::grid::{Dir, Grid, same_grid, slot};
use alloc::vec::Vec;

/// Some of a board's cells, as a board of their own.
///
/// Build one with [`Grid::subset`], or take one from a range query. It borrows the board it came
/// from, which is deliberate: a region is a thing you make, use, and drop within a turn — a
/// highlighted attack, a reachable set, the room a unit is standing in. Nothing about it needs to
/// outlive the board, and storing it long-term is a sign you wanted the coordinates instead.
///
/// # Two numberings are live at once
///
/// A `SubGrid` numbers its own cells from zero, and so does its root. Both are valid; both are in
/// range; they mean different cells. This is the one way to get a wrong answer out of this crate
/// without hearing about it, and it bites in two places:
///
/// - A [`Movement`](crate::Movement) scanned against the root, handed to a sub-grid. Its closure
///   reads `Step::to` as a root index and prices the wrong cells. Scan it against the board you
///   will search.
/// - A [`CellMap`](crate::CellMap) — or your own `Vec<Terrain>` — built over the root and
///   subscripted by a sub-grid index.
///
/// [`to_root`](Grid::to_root) and [`of_root`](Grid::of_root) are the bridge, and they are the only
/// correct one:
///
/// ```
/// use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Sq};
///
/// let g = FullGrid::square(8, 8, Adjacency::Four);
/// let mud = CellMap::from_fn(&g, |c: Sq| c.y > 5);          // keyed by the root
///
/// let north = g.subset(g.indices().filter(|&i| g.coord(i).y < 4));
/// let here = north.at(Sq::new(0, 0));
///
/// assert!(!mud[north.to_root(here)], "read the root's data through the bridge");
/// ```
///
/// # It is a board, not a mask
///
/// A step that would leave the region returns `None`, so the region has a real edge — which is what
/// makes a path inside a movement range stay inside it. The consequence is worth stating plainly:
/// a sub-grid cannot see what is outside itself. The threat map of a room omits the archer standing
/// one cell beyond the door. When the question is about the whole map, ask the root.
///
/// Distances are untouched. The metric measures coordinates, and a subset does not move any cell,
/// so a range-2 archer inside a region still reaches two cells away.
#[derive(Debug, Clone)]
pub struct SubGrid<'a, B: Grid> {
    root: &'a B,
    /// The root's index for each of this board's cells, strictly ascending. One `Vec` serves both
    /// directions: `to_root` indexes it, `of_root` searches it.
    ///
    /// Bare `u32`, not [`Idx`]: every entry is the root's by construction, and the root's tag is
    /// held once in `tag` rather than once per cell.
    cells: Vec<u32>,
    /// This board's own numbering, hashed from the root's tag and the cell list above. A region
    /// numbers from zero, so its indices and the root's are mutually wrong — this is what makes
    /// mixing them a panic in a debug build rather than a wrong cell.
    tag: Tag,
}

impl<'a, B: Grid> SubGrid<'a, B> {
    /// Sort and dedup the root indices, which is the whole of building one.
    ///
    /// The order a caller lists cells in does not reach the result. `FullGrid::new` honours caller
    /// order because the save round-trip depends on it; nobody has a numbering intent for a region,
    /// so this picks the root's order and the `cells` table is ascending by construction rather
    /// than by an assertion the caller can trip.
    pub(crate) fn of(root: &'a B, cells: impl IntoIterator<Item = Idx>) -> Self {
        let root_tag = root.tag();
        let mut cells: Vec<u32> = cells
            .into_iter()
            .map(|i| {
                same_grid(root_tag, i);
                i.raw()
            })
            .collect();
        cells.sort_unstable();
        cells.dedup();

        // The root's tag is mixed in so that two regions holding the same local index list, taken
        // from different boards, do not collide.
        let tag = Tag::of(
            core::iter::once(u64::from(u32::MAX) + 1).chain(cells.iter().map(|&i| u64::from(i))),
        );
        Self { root, cells, tag }
    }

    /// This region's cells, numbered as the [`root`](Grid::root) numbers them.
    ///
    /// A `SubGrid` borrows, so it cannot be kept in a struct beside the board it came from. Keep
    /// *this* instead — it is what [`Grid::subset`] takes — and rebuild the region when you next
    /// need to ask it something.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Idx, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let highlight: Vec<Idx> = g.within(g.at(Sq::new(2, 2)), 0, 2).root_indices().collect();
    ///
    /// // Later, and as often as you like:
    /// assert!(g.subset(highlight.iter().copied()).contains(Sq::new(2, 4)));
    /// ```
    pub fn root_indices(&self) -> impl Iterator<Item = Idx> + '_ {
        let tag = self.root.tag();
        self.cells.iter().map(move |&i| Idx::new(tag, i))
    }

    /// Mint one of the root's indices from a slot in `cells`.
    fn of_cells(&self, at: usize) -> Idx {
        Idx::new(self.root.tag(), self.cells[at])
    }
}

impl<B: Grid> Grid for SubGrid<'_, B> {
    type Cell = B::Cell;
    type Root = B;

    fn tag(&self) -> Tag {
        self.tag
    }

    fn len(&self) -> usize {
        self.cells.len()
    }

    fn coord(&self, i: Idx) -> B::Cell {
        let at = slot(self.len(), self.tag, i);
        self.root.coord(self.of_cells(at))
    }

    fn index_of(&self, c: B::Cell) -> Option<Idx> {
        self.of_root(self.root.index_of(c)?)
    }

    fn dirs(&self) -> &[Dir<Self>] {
        self.root.dirs()
    }

    fn step(&self, i: Idx, d: Dir<Self>) -> Option<Idx> {
        let at = slot(self.len(), self.tag, i);
        self.of_root(self.root.step(self.of_cells(at), d)?)
    }

    fn neighbors(&self, i: Idx) -> impl Iterator<Item = (Dir<Self>, Idx)> {
        let at = slot(self.len(), self.tag, i);
        self.root
            .neighbors(self.of_cells(at))
            .filter_map(move |(d, j)| Some((d, self.of_root(j)?)))
    }

    fn in_neighbors(&self, j: Idx) -> impl Iterator<Item = (Dir<Self>, Idx)> {
        let at = slot(self.len(), self.tag, j);
        self.root
            .in_neighbors(self.of_cells(at))
            .filter_map(move |(d, i)| Some((d, self.of_root(i)?)))
    }

    fn metric(&self) -> Metric<B::Cell> {
        self.root.metric()
    }

    fn root(&self) -> &B {
        self.root
    }

    fn to_root(&self, i: Idx) -> Idx {
        let at = slot(self.len(), self.tag, i);
        self.of_cells(at)
    }

    fn of_root(&self, i: Idx) -> Option<Idx> {
        same_grid(self.root.tag(), i);
        #[allow(clippy::cast_possible_truncation)]
        self.cells
            .binary_search(&i.raw())
            .ok()
            .map(|k| Idx::new(self.tag, k as u32))
    }
}

#[cfg(test)]
mod tests {
    use crate::coord::{Dir8, Idx, Sq};
    use crate::full::{Adjacency, FullGrid};
    use crate::grid::Grid;
    use crate::path::{Cost, Movement, Step};
    use alloc::vec::Vec;

    /// The 3x3 block in the corner of an 8x8 board, and the board it came from.
    fn corner() -> (FullGrid<Sq>, Vec<Idx>) {
        let g = FullGrid::square(8, 8, Adjacency::Four);
        let block: Vec<Idx> = g
            .indices()
            .filter(|&i| g.coord(i).x < 3 && g.coord(i).y < 3)
            .collect();
        (g, block)
    }

    /// Every answer a board gives about itself, in one place, so two boards can be compared.
    fn survey<B: Grid<Cell = Sq>>(b: &B) -> (usize, Vec<Sq>, Vec<usize>, Vec<u32>, bool) {
        (
            b.len(),
            b.cells().collect(),
            b.indices().map(|i| b.neighbors(i).count()).collect(),
            b.indices()
                .map(|i| b.distance(b.at(Sq::new(0, 0)), i))
                .collect(),
            b.is_connected(|_| true),
        )
    }

    #[test]
    fn a_subgrid_answers_every_query_the_same_way_the_grid_it_came_from_does() {
        // The trait's whole claim: given the same cells, the two are indistinguishable. A wrong
        // primitive on `SubGrid` shows up here and nowhere obvious otherwise.
        let g = FullGrid::square(5, 4, Adjacency::Eight);
        let all = g.subset(g.indices());

        assert_eq!(survey(&g), survey(&all));

        // Each board is asked in its own indices — which is the point. They agree because the
        // subset covers every cell in the same order, not because an index was reused.
        let (from, to) = (Sq::new(0, 0), Sq::new(4, 3));
        assert_eq!(
            g.path(g.at(from), g.at(to), &Movement::scan(&g, |_| Some(10)))
                .unwrap(),
            all.path(
                all.at(from),
                all.at(to),
                &Movement::scan(&all, |_| Some(10))
            )
            .unwrap(),
        );
    }

    #[test]
    fn a_subgrid_maps_every_cell_back_to_the_root_it_came_from() {
        let (g, block) = corner();
        let sub = g.subset(block.iter().copied());

        assert_eq!(sub.len(), 9);
        for i in sub.indices() {
            assert_eq!(sub.of_root(sub.to_root(i)), Some(i));
            assert_eq!(sub.coord(i), g.coord(sub.to_root(i)));
        }

        let outside = g.at(Sq::new(7, 7));
        assert_eq!(sub.of_root(outside), None, "not every root cell is in here");
    }

    #[test]
    fn a_subgrid_numbers_its_cells_in_the_roots_order_whatever_order_it_is_given_them() {
        // The sort-and-dedup that `of_root`'s binary search rests on. If this regresses, the map
        // back returns confidently wrong indices rather than failing.
        let (g, mut block) = corner();
        block.reverse();
        block.push(block[4]);

        let sub = g.subset(block);
        assert_eq!(sub.len(), 9, "the duplicate collapsed");
        assert!(
            sub.indices()
                .map(|i| sub.to_root(i))
                .collect::<Vec<_>>()
                .windows(2)
                .all(|w| w[0] < w[1]),
        );
    }

    #[test]
    fn a_step_that_leaves_a_subgrid_is_the_edge_of_the_board() {
        let (g, block) = corner();
        let sub = g.subset(block);
        let rim = sub.at(Sq::new(2, 1));

        assert!(sub.step(rim, Dir8::E).is_none(), "east is off this board");
        assert!(sub.step(rim, Dir8::W).is_some());

        // And a search cannot escape it either: the far corner of the root is simply not here.
        let m: Movement<fn(Step<Sq>) -> Option<Cost>> = Movement::new(|_| Some(10), 10);
        let ends: Vec<Sq> = sub
            .reachable(sub.at(Sq::new(0, 0)), 1000, &m)
            .iter()
            .map(|&(i, _)| sub.coord(i))
            .collect();
        assert_eq!(ends.len(), 9);
        assert!(ends.iter().all(|c| c.x < 3 && c.y < 3));
    }

    #[test]
    fn a_subgrid_keeps_the_distances_of_the_grid_it_came_from() {
        // A region is a smaller board, not a shorter ruler. The metric measures coordinates, so
        // dropping cells between two others must not bring them closer.
        let g = FullGrid::square(8, 8, Adjacency::Eight);
        let ends = g.subset([g.at(Sq::new(0, 0)), g.at(Sq::new(5, 0))]);

        assert_eq!(ends.len(), 2);
        assert_eq!(
            ends.distance(ends.at(Sq::new(0, 0)), ends.at(Sq::new(5, 0))),
            5,
            "and not 1, which is how far apart they are numbered"
        );
    }
}
