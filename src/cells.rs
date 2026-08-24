//! One value per cell, indexed by [`Idx`] and nothing else.
//!
//! The grid still holds no game state — this is not part of it, and it does not hold a grid. It is
//! the `Vec` every consumer was already writing, with the two things that were going wrong taken
//! away: it is sized from the board rather than from a number you remembered, and it is subscripted
//! by an [`Idx`] rather than by an `i as usize` cast written out by hand.
//!
//! ```
//! use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Sq};
//!
//! let g = FullGrid::square(8, 8, Adjacency::Four);
//! let mut mud = CellMap::new(&g, false);
//!
//! mud[g.index_of(Sq::new(3, 3)).unwrap()] = true;
//! assert_eq!(mud.iter().filter(|&(_, &m)| m).count(), 1);
//! ```
//!
//! `&grid` and `&mut cell_map` stay two objects, so the borrow-checker story is the one the crate
//! started with: an AI search reads the board while it mutates the position, and cloning a position
//! never copies the board.

use std::ops::{Index, IndexMut};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::coord::Idx;
use crate::grid::Grid;

/// One `T` for every cell of a grid, addressed by [`Idx`].
///
/// # It goes stale exactly as an [`Idx`] does
///
/// A `CellMap` is positional: slot `i` is whatever cell the grid called `i` when the map was built.
/// [`FullGrid::filtered`](crate::FullGrid::filtered) renumbers and a [`SubGrid`](crate::SubGrid) numbers its own cells from
/// zero, so a map built against one board is **stale** against the other — and a stale map is not
/// merely wrong-sized, it may quietly answer for a *different cell*. Build a fresh one, or read the
/// map you have through [`Grid::to_root`].
///
/// This is also why [`Grid::len`] is the only thing that ever sizes one. A `CellMap` and its grid
/// disagreeing about how many cells there are is the bug the type exists to prevent.
///
/// # Saving one
///
/// With the `serde` feature it serializes whenever `T` does — but it serializes as a *list in
/// index order*, so it is the one thing in this crate you may save that is keyed by index rather
/// than by coordinate. That is safe under exactly one rule: **save [`Grid::cells`] beside it**, and
/// rebuild the grid from those cells. [`FullGrid::new`](crate::FullGrid::new) numbers cells in the order it is given
/// them, so
/// that restores the same indices and the map lines up again. Save it beside a width and a height
/// instead, and any later change to how that board is generated moves every value one cell.
/// `tests/save.rs` walks through both.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CellMap<T> {
    of: Vec<T>,
}

impl<T> CellMap<T> {
    /// The same value in every cell.
    ///
    /// The grid is borrowed to be measured and is not kept.
    #[must_use]
    pub fn new<B: Grid + ?Sized>(g: &B, value: T) -> Self
    where
        T: Clone,
    {
        Self {
            of: vec![value; g.len()],
        }
    }

    /// A value worked out from each cell's **coordinate**, in index order.
    ///
    /// A coordinate, not an index, because that is what your map data is keyed by — a tilemap's
    /// `(col, row)`, a noise function, a room rectangle. An index would only send you back to the
    /// grid to ask what cell it meant.
    ///
    /// ```
    /// use spacewalk::{CellMap, FullGrid, Grid, Offset};
    ///
    /// // A river down one column of an authored map.
    /// let g = FullGrid::hex_rect(10, 10, Offset::OddR);
    /// let water = CellMap::from_fn(&g, |c| Offset::OddR.from_hex(c).0 == 4);
    ///
    /// assert_eq!(water.iter().filter(|&(_, &w)| w).count(), 10);
    /// ```
    #[must_use]
    pub fn from_fn<B: Grid + ?Sized>(g: &B, f: impl Fn(B::Cell) -> T) -> Self {
        Self {
            of: g.cells().map(f).collect(),
        }
    }

    /// Every cell and its value, in index order.
    pub fn iter(&self) -> impl Iterator<Item = (Idx, &T)> {
        self.of.iter().enumerate().map(|(i, v)| (i as Idx, v))
    }

    /// How many cells the map covers — the [`Grid::len`] it was built from.
    #[must_use]
    pub fn len(&self) -> usize {
        self.of.len()
    }

    /// Whether the map covers no cells at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.of.is_empty()
    }

    /// Panics with something readable if `i` is past the end of this map.
    #[track_caller]
    fn slot(&self, i: Idx) -> usize {
        assert!(
            (i as usize) < self.of.len(),
            "cell {i} is not in this CellMap, which covers {} cells (a map is positional, so \
             one built before `filtered` renumbered will not do)",
            self.of.len(),
        );
        i as usize
    }
}

impl<T> Index<Idx> for CellMap<T> {
    type Output = T;

    /// # Panics
    ///
    /// If `i` is past the end of this map.
    #[track_caller]
    fn index(&self, i: Idx) -> &T {
        &self.of[self.slot(i)]
    }
}

impl<T> IndexMut<Idx> for CellMap<T> {
    /// # Panics
    ///
    /// If `i` is past the end of this map.
    #[track_caller]
    fn index_mut(&mut self, i: Idx) -> &mut T {
        let slot = self.slot(i);
        &mut self.of[slot]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::Sq;
    use crate::full::{Adjacency, FullGrid};

    #[test]
    fn a_map_covers_every_cell_and_answers_for_the_right_one() {
        let g = FullGrid::square(4, 3, Adjacency::Four);
        let m = CellMap::from_fn(&g, |c: Sq| c.x + c.y);

        assert_eq!(m.len(), g.len());
        for i in g.indices() {
            assert_eq!(m[i], g.coord(i).x + g.coord(i).y);
        }
        assert_eq!(m.iter().count(), g.len());
    }

    #[test]
    fn a_stale_index_is_refused_rather_than_answered_for_the_wrong_cell() {
        // The bug the type exists to catch. `filtered` renumbers, so index 8 of the old grid is a
        // different cell — or no cell — under the new one.
        let full = FullGrid::square(3, 3, Adjacency::Four);
        let map = CellMap::new(&full.filtered(|c| c.x != 2), 0u8);

        assert_eq!(map.len(), 6);
        let stale = full.index_of(Sq::new(2, 2)).unwrap();
        assert!(std::panic::catch_unwind(|| map[stale]).is_err());
    }
}
