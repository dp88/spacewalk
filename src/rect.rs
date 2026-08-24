//! A full rectangle of square cells, computed rather than stored.
//!
//! [`FullGrid`](crate::FullGrid) holds any set of cells, so it must store the cells, the index of
//! each, and the step table both ways. On a full `w × h` rectangle every one of those answers is
//! arithmetic: the index is `y * w + x`, the coordinate is a divmod, a step is a coordinate add and
//! a bounds check, and a predecessor is the same step the other way. [`RectGrid`] does that
//! arithmetic and stores three numbers.
//!
//! Use it when the board is a plain rectangle. Use [`FullGrid`](crate::FullGrid) for anything else:
//! holes, a disc, a hex board, or a coordinate of your own.

use crate::coord::{Coord, Dir8, Idx, Metric, Sq, Tag};
use crate::full::{Adjacency, MAX_CELLS};
use crate::grid::{Grid, same_grid, slot};

/// A `w × h` rectangle of square cells, in row-major order, with no derived geometry stored.
///
/// It answers exactly as `FullGrid::square(w, h, adj)` does, cell for cell and index for index —
/// `rect.rs`'s own test holds the two side by side. The difference is what it costs: three fields,
/// whatever the size of the board. A 512 × 512 map keeps a step table of a quarter of a million
/// cells times eight directions, and the coordinate list and index map beside it, or it keeps
/// twelve bytes.
///
/// The adjacency picks the metric to match it, exactly as [`FullGrid::square`](crate::FullGrid::square)
/// does; see [`Adjacency`].
///
/// ```
/// use spacewalk::{Adjacency, Dir8, Grid, RectGrid, Sq};
///
/// let g = RectGrid::new(8, 8, Adjacency::Four);
/// let a1 = g.at(Sq::new(0, 7));
///
/// assert_eq!(g.len(), 64);
/// assert!(g.step(a1, Dir8::S).is_none(), "the bottom row has no south");
/// assert!(g.step(a1, Dir8::N).is_some());
/// assert!(g.step(a1, Dir8::Ne).is_none(), "a four-way grid has no diagonals at all");
/// ```
///
/// # It is a lattice, and only a lattice
///
/// [`in_neighbors`](Grid::in_neighbors) is computed rather than inverted: the cell that steps into
/// `i` heading `d` is the cell `i` reaches heading the opposite way. That holds because every step
/// here is a coordinate add and edges are symmetric. A board with a portal, a one-way ledge in its
/// *geometry*, or a wrapping edge breaks it — none of which a rectangle has, and all of which
/// belong to [`FullGrid`](crate::FullGrid), which inverts its table instead. A one-way rule that
/// comes from the *game* rather than the lattice is a cost function and is unaffected; see
/// [`crate::path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectGrid {
    w: i32,
    h: i32,
    adj: Adjacency,
    /// This board's numbering. Hashed from the same cell sequence a [`FullGrid::square`] of this
    /// size emits, so the two agree and their indices travel between them — which is what the
    /// promise above ("same answers, same indices") is worth only if it is checked.
    ///
    /// The generator below is lazy and is never walked in release, where a [`Tag`] is zero-sized.
    tag: Tag,
}

impl RectGrid {
    /// A `w × h` rectangle of square cells, in row-major order.
    ///
    /// # Panics
    ///
    /// If `w` or `h` is negative, or if the two together exceed [`MAX_CELLS`].
    ///
    /// This grid allocates nothing, so the bound is not about its own memory — it is about what
    /// every consumer sizes from [`Grid::len`]. A [`CellMap`](crate::CellMap), the search's parent
    /// vector, and [`Grid::component`]'s visited set are all one entry per cell. The limit is the
    /// same number for the same reason: past it, asking the question takes the machine down rather
    /// than returning an answer.
    #[must_use]
    pub fn new(w: i32, h: i32, adj: Adjacency) -> Self {
        assert!(w >= 0 && h >= 0, "a grid cannot be {w} x {h}");
        assert!(
            w as u64 * h as u64 <= MAX_CELLS,
            "{w} x {h} is {} cells; a grid may hold at most {MAX_CELLS}",
            w as u64 * h as u64,
        );

        Self {
            w,
            h,
            adj,
            tag: Tag::of((0..h).flat_map(move |y| (0..w).map(move |x| Sq::new(x, y)))),
        }
    }

    /// Mint one of this grid's indices. The single door between a bare number and an [`Idx`].
    const fn idx(&self, i: u32) -> Idx {
        Idx::new(self.tag, i)
    }
}

impl Grid for RectGrid {
    type Cell = Sq;
    type Root = Self;

    fn tag(&self) -> Tag {
        self.tag
    }

    fn len(&self) -> usize {
        self.w as usize * self.h as usize
    }

    fn coord(&self, i: Idx) -> Sq {
        let cell = slot(self.len(), self.tag, i) as i32;
        Sq::new(cell % self.w, cell / self.w)
    }

    fn index_of(&self, c: Sq) -> Option<Idx> {
        let on = (0..self.w).contains(&c.x) && (0..self.h).contains(&c.y);
        on.then(|| self.idx((c.y * self.w + c.x) as u32))
    }

    fn dirs(&self) -> &[Dir8] {
        match self.adj {
            Adjacency::Four => &Dir8::ORTHO,
            Adjacency::Eight => &Dir8::ALL,
        }
    }

    fn step(&self, i: Idx, d: Dir8) -> Option<Idx> {
        let _ = slot(self.len(), self.tag, i);
        if !self.dirs().contains(&d) {
            return None;
        }
        self.index_of(self.coord(i).step(d))
    }

    fn neighbors(&self, i: Idx) -> impl Iterator<Item = (Dir8, Idx)> {
        let c = self.coord(i);
        self.dirs()
            .iter()
            .filter_map(move |&d| Some((d, self.index_of(c.step(d))?)))
    }

    fn in_neighbors(&self, j: Idx) -> impl Iterator<Item = (Dir8, Idx)> {
        let c = self.coord(j);
        self.dirs()
            .iter()
            .filter_map(move |&d| Some((d, self.index_of(c.step(d.opposite()))?)))
    }

    fn metric(&self) -> Metric<Sq> {
        match self.adj {
            Adjacency::Four => Metric::MANHATTAN,
            Adjacency::Eight => Metric::CHEBYSHEV,
        }
    }

    fn root(&self) -> &Self {
        self
    }

    fn to_root(&self, i: Idx) -> Idx {
        let _ = slot(self.len(), self.tag, i);
        i
    }

    fn of_root(&self, i: Idx) -> Option<Idx> {
        same_grid(self.tag, i);
        ((i.raw() as usize) < self.len()).then_some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::full::FullGrid;
    use crate::path::{Cost, Movement};
    use alloc::vec::Vec;

    /// Every predecessor of `j`, as a set — the two boards agree on the members, and the trait
    /// promises no order.
    fn back<B: Grid<Cell = Sq>>(b: &B, j: Idx) -> Vec<(Dir8, Idx)> {
        let mut all: Vec<(Dir8, Idx)> = b.in_neighbors(j).collect();
        all.sort_unstable_by_key(|&(d, i)| (i, d as u8));
        all
    }

    #[test]
    fn a_computed_rectangle_answers_exactly_as_a_stored_one_does() {
        // The failure class this guards: the arithmetic disagreeing with the table at an edge, or
        // numbering the cells in another order. Pathfinding would consume either silently and
        // return a confident wrong route.
        for adj in [Adjacency::Four, Adjacency::Eight] {
            let (stored, rect) = (FullGrid::square(5, 3, adj), RectGrid::new(5, 3, adj));

            assert_eq!(stored.len(), rect.len(), "{adj:?}");
            assert_eq!(stored.dirs(), rect.dirs(), "{adj:?}");

            for i in stored.indices() {
                let c = stored.coord(i);
                assert_eq!(rect.coord(i), c, "{adj:?} {i}");
                assert_eq!(rect.index_of(c), Some(i), "{adj:?} {c:?}");

                for &d in Dir8::ALL.iter() {
                    assert_eq!(stored.step(i, d), rect.step(i, d), "{adj:?} {c:?} {d:?}");
                }
                assert_eq!(back(&stored, i), back(&rect, i), "{adj:?} {c:?}");
            }

            // Off the board in every direction, including the corners a bounds check gets wrong.
            for c in [Sq::new(-1, 0), Sq::new(0, -1), Sq::new(5, 2), Sq::new(4, 3)] {
                assert_eq!(rect.index_of(c), None, "{adj:?} {c:?}");
            }

            // And one route past an obstacle, which is what all of the above is for.
            let wall = Sq::new(2, 1);
            let cost = |c: Sq| (c != wall).then_some(10 as Cost);
            let (from, to) = (Sq::new(0, 1), Sq::new(4, 1));

            let a = stored
                .path(
                    stored.at(from),
                    stored.at(to),
                    &Movement::scan(&stored, |s| cost(stored.coord(s.to))),
                )
                .unwrap();
            let b = rect
                .path(
                    rect.at(from),
                    rect.at(to),
                    &Movement::scan(&rect, |s| cost(rect.coord(s.to))),
                )
                .unwrap();

            assert_eq!((a.steps(), a.cost()), (b.steps(), b.cost()), "{adj:?}");
        }
    }
}
