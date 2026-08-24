//! A whole board, stored: a set of cells, a dense index over them, and one step table.
//!
//! This is the thing you build. [`FullGrid`] holds the cells and the edges between them; the
//! vocabulary you ask it questions in is [`Grid`], which it implements — and so does
//! [`SubGrid`](crate::SubGrid), a region of one of these.
//!
//! A `FullGrid` is **pure geometry**. It holds no terrain, no pieces, no owners — your game holds
//! those, and hands the grid a callback when it wants a path (see [`crate::path`]). That is what
//! keeps the grid immutable, shareable, and out of your borrow checker's way: `&FullGrid` and
//! `&mut your_state` are simply different objects.

use hashbrown::HashMap;
use hashbrown::hash_map::Entry;

use crate::coord::{Coord, Dir6, Dir8, Hex, Idx, Metric, Sq, Tag};
use crate::grid::{Grid, same_grid, slot};
use crate::layout::Offset;
use alloc::vec;
use alloc::vec::Vec;

/// The step table's empty slot. `u32::MAX` cells will never exist.
///
/// The tables below hold bare `u32`, not [`Idx`]. They are this grid's own storage, every entry in
/// them is this grid's by construction, and a tag on each would be a tag repeated `cells × dirs`
/// times to say one thing. An [`Idx`] is minted on the way out, in [`FullGrid::idx`].
const NONE: u32 = u32::MAX;

/// The most cells a grid may hold: 2²⁴, or 16,777,216. A 4096 × 4096 board.
///
/// This is a **memory** limit, not an addressing one, and the difference is the whole point. An
/// [`Idx`] is a `u32`, so a grid could in principle address four billion cells — but the step table
/// alone would then want 137GB, and asking for it does not fail cleanly, it takes the machine down
/// with it. The first version of this guard checked the addressing limit and sailed straight past a
/// 46341 × 46341 board (2.1 billion cells, a 68GB allocation), because that is *under* four billion.
///
/// So the bound is set where memory stays sane: even at eight directions the step table tops out
/// around half a gigabyte. No game board is anywhere near this. If you genuinely need a bigger
/// world, you want a chunked one, not a bigger `FullGrid`.
pub const MAX_CELLS: u64 = 1 << 24;

/// Which neighbours a square grid connects, and how it measures distance.
///
/// The two are chosen together, deliberately. An eight-way board measured with Manhattan distance
/// is the classic tactics bug: a melee unit can *step* to a diagonally adjacent enemy but reports
/// it as two away, so it cannot *attack* it. Picking the adjacency picks the metric that agrees
/// with it, and there is no second knob to get wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Adjacency {
    /// Orthogonal only, with Manhattan distance. The classic turn-based tactics shape.
    Four,
    /// Orthogonal and diagonal, with Chebyshev distance.
    Eight,
}

/// A whole board: cells, their dense indices, and the cell each direction leads to.
///
/// Cells are addressed by [`Idx`], a dense `u32` assigned at construction. Indices are stable for
/// the grid's lifetime but mean nothing to any other grid — **serialize coordinates, never
/// indices.**
///
/// # A grid is not saved, it is rebuilt
///
/// `FullGrid` is deliberately not serializable. It holds function pointers (its [`Metric`]), which
/// have no sensible serialized form — and everything else in it, the index map and the step tables,
/// is *derived*. Geometry is cheaper to rebuild than to store.
///
/// So save the grid's **definition**, not the grid: the arguments you built it from. For the shipped
/// shapes that is `(w, h, adjacency)` or a radius, all of which serialize. For a board of your own,
/// save [`Grid::cells`] — the coordinates — and hand them straight back to [`FullGrid::new`] with
/// the same directions and metric, which live in your code and not in your save file.
///
/// That round-trip is exact. `FullGrid::new` numbers cells in the order it is given them, so
/// rebuilding from `cells()` reproduces the *same indices*, not merely an equivalent board. See
/// `tests/save.rs`.
///
/// ```
/// use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Sq};
///
/// let g = FullGrid::square(8, 8, Adjacency::Four);
/// let a1 = g.at(Sq::new(0, 7));
///
/// assert_eq!(g.len(), 64);
/// assert!(g.step(a1, Dir8::S).is_none(), "the bottom row has no south");
/// assert!(g.step(a1, Dir8::N).is_some());
/// assert!(g.step(a1, Dir8::Ne).is_none(), "a four-way grid has no diagonals at all");
/// ```
#[derive(Debug, Clone)]
pub struct FullGrid<C: Coord> {
    /// Every cell's coordinate, in index order: `cells[i]` is the coordinate of cell `i`.
    cells: Vec<C>,
    /// The reverse lookup: coordinate to index. `index_of` is one probe of this map.
    ///
    /// `hashbrown`, which is what `std`'s own `HashMap` is built from, so this is the same table
    /// without the `std`. An `alloc::BTreeMap` would have kept the crate free of dependencies
    /// altogether, and it was measured: five times slower per lookup, six times slower to build a
    /// board, and 2.8x slower on `visible_from`, which asks this question once per cell it draws a
    /// line to.
    index: HashMap<C, u32>,
    /// The direction alphabet, fixing the column order of `steps`.
    dirs: Vec<C::Dir>,
    /// Flat, `cells.len() * dirs.len()`. `steps[i * dirs.len() + d]` is the cell reached by
    /// leaving cell `i` in direction `dirs[d]`, or [`NONE`].
    ///
    /// Flat and direction-indexed, not a compacted neighbour list. The distinction matters: a
    /// compacted list cannot answer "which of these is my north-east?", and a checkers man that
    /// may only move forward needs exactly that.
    steps: Vec<u32>,
    /// The step table, reversed: who can step *into* each cell.
    ///
    /// A multimap in compressed-row form, not a mirror of `steps`, and that is deliberate. Mirroring
    /// assumes each cell has at most one predecessor per direction — true of any lattice, false the
    /// moment a caller's `step` clamps at an edge or leads several cells into one portal. A mirror
    /// would silently keep the last writer and lose the rest, which in a game means an enemy who can
    /// reach you and does not appear on the threat overlay. This holds all of them.
    back: Back,
    /// How distance is measured and range queries are answered. See [`Metric`].
    metric: Metric<C>,
    /// This board's numbering, hashed from `cells`. See [`Tag`].
    tag: Tag,
}

/// In-edges, in compressed-row form: the predecessors of cell `j` are `from[start[j]..start[j+1]]`,
/// each with the direction it would travel in.
#[derive(Debug, Clone, Default)]
struct Back {
    start: Vec<u32>,
    from: Vec<u32>,
    dir: Vec<u8>,
}

impl Back {
    /// Invert the step table in two passes: count what arrives where, then place it.
    fn of(steps: &[u32], cells: usize, dirs: usize) -> Self {
        let mut start = vec![0u32; cells + 1];
        for &j in steps {
            if j != NONE {
                start[j as usize + 1] += 1;
            }
        }
        for k in 1..start.len() {
            start[k] += start[k - 1];
        }

        let edges = *start.last().unwrap_or(&0) as usize;
        let (mut from, mut dir) = (vec![0; edges], vec![0u8; edges]);
        let mut at = start.clone();

        for (slot, &j) in steps.iter().enumerate() {
            if j == NONE {
                continue;
            }
            let put = at[j as usize] as usize;
            from[put] = (slot / dirs) as u32;
            dir[put] = (slot % dirs) as u8;
            at[j as usize] += 1;
        }

        Self { start, from, dir }
    }
}

impl<C: Coord> FullGrid<C> {
    /// Build a grid from any set of cells, any direction alphabet, and a distance metric.
    ///
    /// Duplicate cells are dropped, keeping the first occurrence — so the caller's order survives,
    /// and it is the caller's order that fixes the indices.
    ///
    /// `metric` must agree with `dirs`: **one step must never cover more than one unit of
    /// distance** — and this is *checked*, not merely asked for. It used to be only asked for, and
    /// the crate's own headline bug was then one line from the front door: eight-way directions with
    /// Manhattan distance builds a board where a unit can step onto the diagonal and is told the
    /// diagonal is two cells away, so it stands beside an enemy unable to swing at it. The check
    /// costs one metric call per edge, in the loop that walks the edges anyway.
    ///
    /// If your board has genuine multi-cell steps — portals, jumps, a conveyor that carries you
    /// three cells — no honest metric can call those one step. Give it a metric that returns 0:
    /// always an underestimate, so A\* degrades into Dijkstra, which is slower and still correct.
    ///
    /// For a coordinate of your own, [`Metric::scanning`] is the safe default: it asks nothing of
    /// your metric beyond `distance`, and range queries fall back to scanning the board.
    ///
    /// # Panics
    ///
    /// If a single step covers more than one unit of `metric` distance (see above), or if the cells
    /// exceed [`MAX_CELLS`] — checked as the iterator is consumed, so an unbounded one stops at the
    /// limit rather than being counted to exhaustion first.
    #[must_use]
    pub fn new(cells: impl IntoIterator<Item = C>, dirs: &[C::Dir], metric: Metric<C>) -> Self {
        let mut ordered = Vec::new();
        let mut index = HashMap::new();
        for c in cells {
            if let Entry::Vacant(slot) = index.entry(c) {
                slot.insert(ordered.len() as u32);
                ordered.push(c);
            }

            // Checked as we go, not afterwards: a caller can hand us an unbounded iterator, and
            // counting it to completion before complaining is exactly the runaway we are stopping.
            assert!(
                ordered.len() as u64 <= MAX_CELLS,
                "a grid may hold at most {MAX_CELLS} cells",
            );
        }

        let mut steps = Vec::with_capacity(ordered.len() * dirs.len());
        for (i, &c) in ordered.iter().enumerate() {
            for &d in dirs {
                let j = index.get(&c.step(d)).copied().unwrap_or(NONE);

                // A step onto your own cell is not an edge, it is a fixed point — and it is what a
                // clamping or saturating `Coord::step` produces at the board's edge. Left in, it
                // gives `ray` an infinite loop and the search a zero-length cycle.
                if j as usize == i {
                    steps.push(NONE);
                    continue;
                }

                // The metric must agree with the directions, and this is where that is enforced
                // rather than merely asked for. If a single step can carry you *further* than the
                // metric says one step goes, the metric is lying: melee cannot reach an enemy it is
                // standing next to, and the A* heuristic overestimates and quietly stops returning
                // the cheapest path.
                //
                // It costs one metric call per edge, in the loop that was walking the edges anyway.
                if j != NONE {
                    let span = metric.distance(c, ordered[j as usize]);
                    assert!(
                        span <= 1,
                        "a step from {c:?} to {:?} covers {span} under this metric, but a step is \
                         one. The metric disagrees with the directions — the classic case is \
                         eight-way movement measured with Manhattan distance, where a unit can step \
                         onto the diagonal but is told it is two cells away. If your board has \
                         genuine multi-cell steps (portals, jumps), give it a metric that returns \
                         0: A* becomes Dijkstra, which is slower and still correct.",
                        ordered[j as usize],
                    );
                }

                steps.push(j);
            }
        }

        let back = Back::of(&steps, ordered.len(), dirs.len());
        Self {
            tag: Tag::of(ordered.iter()),
            cells: ordered,
            index,
            dirs: dirs.to_vec(),
            steps,
            back,
            metric,
        }
    }

    /// Mint one of this grid's indices. The single door between a raw table slot and an [`Idx`].
    const fn idx(&self, i: u32) -> Idx {
        Idx::new(self.tag, i)
    }

    /// A step-table slot as an `Option`: the sentinel [`NONE`] becomes `None`.
    fn reached(&self, j: u32) -> Option<Idx> {
        (j != NONE).then(|| self.idx(j))
    }

    /// A **new** board holding only the cells that pass `keep`.
    ///
    /// This is how you get holes, islands, and boards that are not rectangles — a hex board with
    /// gaps punched in it, or a draughts board of dark squares only. Steps into a dropped cell
    /// become dead ends.
    ///
    /// Named for what it returns, in the manner of `sorted` and `reversed`. It is deliberately *not*
    /// `retain`: `Vec::retain` edits in place and returns nothing, and a method that borrows the
    /// same name while doing something else is a trap.
    ///
    /// # This carves a shape; it does not select a region
    ///
    /// `filtered` builds a board that stands on its own and remembers nothing of the one it came
    /// from — the right thing when the result *is* the board you will play on. When you mean to
    /// work over part of a board and then come back, you want [`Grid::subset`], which borrows
    /// rather than copies and keeps the way back.
    ///
    /// # Indices are reassigned
    ///
    /// The new grid numbers its cells afresh, so any [`Idx`] you were holding is now **stale** — and
    /// a stale index is not merely invalid, it may quietly address a *different cell*. Look cells up
    /// again by coordinate ([`Grid::index_of`]) after filtering. This is the one way to get a wrong
    /// answer out of this crate without hearing about it.
    #[must_use]
    pub fn filtered(&self, keep: impl Fn(C) -> bool) -> Self {
        Self::new(
            self.cells.iter().copied().filter(|&c| keep(c)),
            &self.dirs,
            self.metric,
        )
    }
}

impl<C: Coord> Grid for FullGrid<C> {
    type Cell = C;
    type Root = Self;

    fn tag(&self) -> Tag {
        self.tag
    }

    fn len(&self) -> usize {
        self.cells.len()
    }

    fn coord(&self, i: Idx) -> C {
        self.cells[slot(self.len(), self.tag, i)]
    }

    fn index_of(&self, c: C) -> Option<Idx> {
        self.index.get(&c).map(|&i| self.idx(i))
    }

    fn dirs(&self) -> &[C::Dir] {
        &self.dirs
    }

    fn step(&self, i: Idx, d: C::Dir) -> Option<Idx> {
        let cell = slot(self.len(), self.tag, i);
        let at = self.dirs.iter().position(|&x| x == d)?;
        self.reached(self.steps[cell * self.dirs.len() + at])
    }

    fn neighbors(&self, i: Idx) -> impl Iterator<Item = (C::Dir, Idx)> {
        let base = slot(self.len(), self.tag, i) * self.dirs.len();
        self.dirs
            .iter()
            .enumerate()
            .filter_map(move |(at, &d)| self.reached(self.steps[base + at]).map(|j| (d, j)))
    }

    fn in_neighbors(&self, j: Idx) -> impl Iterator<Item = (C::Dir, Idx)> {
        let cell = slot(self.len(), self.tag, j);
        let (lo, hi) = (
            self.back.start[cell] as usize,
            self.back.start[cell + 1] as usize,
        );
        (lo..hi).map(move |k| {
            (
                self.dirs[self.back.dir[k] as usize],
                self.idx(self.back.from[k]),
            )
        })
    }

    fn metric(&self) -> Metric<C> {
        self.metric
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

// -- the shipped shapes ------------------------------------------------------------------------

impl FullGrid<Sq> {
    /// A `w × h` rectangle of square cells, in row-major order.
    ///
    /// The adjacency picks the metric to match it; see [`Adjacency`].
    ///
    /// # Panics
    ///
    /// If `w` or `h` is negative, or if the two together would ask for more cells than a grid can
    /// hold. Both are cheap mistakes to make and expensive to suffer: a negative side used to build
    /// an *empty* grid without complaint, and a large one used to try to allocate tens of gigabytes.
    #[must_use]
    pub fn square(w: i32, h: i32, adj: Adjacency) -> Self {
        assert!(w >= 0 && h >= 0, "a grid cannot be {w} x {h}");
        assert!(
            w as u64 * h as u64 <= MAX_CELLS,
            "{w} x {h} is {} cells; a grid may hold at most {MAX_CELLS}",
            w as u64 * h as u64,
        );

        let cells = (0..h).flat_map(move |y| (0..w).map(move |x| Sq::new(x, y)));
        match adj {
            Adjacency::Four => Self::new(cells, &Dir8::ORTHO, Metric::MANHATTAN),
            Adjacency::Eight => Self::new(cells, &Dir8::ALL, Metric::CHEBYSHEV),
        }
    }

    /// A disc of square cells centred on the origin: every cell with `x² + y² <= radius²`.
    ///
    /// The origin-centred counterpart to [`FullGrid::square`], which only ever emits `0..w` by
    /// `0..h` and so has no middle to name. An arena, a blast, a small round world all want a board
    /// whose centre is a coordinate you can write down, and `Sq::new(0, 0)` is it.
    ///
    /// This is rounder than either adjacency's own range. Under [`Adjacency::Four`] a range query
    /// gives a diamond, under [`Adjacency::Eight`] the whole square; this is the circle between
    /// them. All three agree at radius 2, and part company at 3: twenty-nine cells here, against
    /// the diamond's twenty-five and the square's forty-nine.
    ///
    /// Cells come in row-major order with `y` ascending, as [`FullGrid::square`] does. The top row
    /// of a disc holds one cell, so index 0 is `Sq::new(0, -radius)` — **the top of the circle, not
    /// its centre.** Ask [`Grid::index_of`] for the centre.
    ///
    /// A sight line between two cells near the rim may pass outside it. [`Grid::line`] skips cells
    /// that are not on the board, and a cell off the board holds no blocker, so nothing is seen
    /// through — the line simply has a gap, as it does over any hole.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let g = FullGrid::disc(3, Adjacency::Four);
    ///
    /// assert_eq!(g.len(), 29);
    /// assert_eq!(g.cells().next(), Some(Sq::new(0, -3))); // the top of the disc, not its centre
    /// assert!(g.contains(Sq::new(2, 2)));               // 8 <= 9, so the near corner is in
    /// assert!(!g.contains(Sq::new(3, 3)));              // and the corner of the box is not
    /// ```
    ///
    /// # Panics
    ///
    /// If `radius` is negative, or large enough that the disc's *bounding box* would not fit in a
    /// grid. The bound is the box, not the disc, so it is conservative by about a fifth: it refuses
    /// radius 2048, though a disc of radius 2310 would still fit. That is deliberate. The exact
    /// count is the Gauss circle problem and has no closed form to guard with, and counting the
    /// rows to find out is itself the four-billion-iteration loop the guard exists to stop.
    #[must_use]
    pub fn disc(radius: i32, adj: Adjacency) -> Self {
        assert!(radius >= 0, "radius must be >= 0, not {radius}");

        // In `u64`, and not for show. The widest box a caller can name is `2 * i32::MAX + 1` cells
        // on a side, which squares to 8.6 billion short of `u64::MAX` — it fits, but only just. One
        // integer width narrower it wraps, the guard waves the radius through, and the loop below
        // walks four billion rows.
        let side = 2 * radius as u64 + 1;
        assert!(
            side * side <= MAX_CELLS,
            "a disc of radius {radius} needs a {side} x {side} box, which is {} cells; a grid may \
             hold at most {MAX_CELLS}",
            side * side,
        );

        // The guard above caps `radius` at 2047, so `x * x + y * y` could not leave `i32`. It is
        // written in `i64` all the same, as `square_deltas` is: a reader should not have to
        // re-derive the guard to see that the arithmetic here is safe.
        let r2 = i64::from(radius) * i64::from(radius);
        let cells = (-radius..=radius).flat_map(move |y| {
            let yy = i64::from(y) * i64::from(y);
            (-radius..=radius)
                .filter(move |&x| i64::from(x) * i64::from(x) + yy <= r2)
                .map(move |x| Sq::new(x, y))
        });

        match adj {
            Adjacency::Four => Self::new(cells, &Dir8::ORTHO, Metric::MANHATTAN),
            Adjacency::Eight => Self::new(cells, &Dir8::ALL, Metric::CHEBYSHEV),
        }
    }
}

impl FullGrid<Hex> {
    /// A hexagon of hex cells with the given radius: `radius = 0` is one cell, `1` is seven.
    ///
    /// # Panics
    ///
    /// If `radius` is negative, or large enough that the hexagon would not fit in a grid. The upper
    /// bound is not pedantry: the row bounds below (`-q + radius`) overflow `i32` above about 1.07
    /// billion, and in release that wraps and quietly returns a *malformed* hexagon.
    #[must_use]
    pub fn hexagon(radius: i32) -> Self {
        assert!(radius >= 0, "radius must be >= 0, not {radius}");
        let r = radius as u64;
        assert!(
            3 * r * (r + 1) < MAX_CELLS,
            "a hexagon of radius {radius} is {} cells; a grid may hold at most {MAX_CELLS}",
            3 * r * (r + 1) + 1,
        );

        let cells = (-radius..=radius).flat_map(move |q| {
            ((-radius).max(-q - radius)..=radius.min(-q + radius)).map(move |r| Hex::new(q, r))
        });
        Self::new(cells, &Dir6::ALL, Metric::HEX)
    }

    /// A `w × h` rectangular field of hex cells, in row-major order.
    ///
    /// The shape most hex tactics maps actually are. [`FullGrid::hexagon`] is a hexagon, which is
    /// what a board game wants; a battlefield is a rectangle, and working its axial row bounds out
    /// by hand is a sum that is easy to get wrong and gives no sign when it is.
    ///
    /// This is the constructor that pairs with **loading a map an external tilemap editor
    /// authored**. Such an editor stores a hex map as a plain `(col, row)` rectangle with alternate
    /// lines nudged sideways, so its cells are exactly `0..w` by `0..h` under one of the four
    /// staggering conventions. Pick the [`Offset`] your editor uses and the cells line up one for
    /// one: cell `(col, row)` of the file is `Offset::to_hex(col, row)` here.
    ///
    /// ```
    /// use spacewalk::{FullGrid, Grid, Offset};
    ///
    /// let g = FullGrid::hex_rect(20, 12, Offset::OddR);
    /// assert_eq!(g.len(), 240);
    ///
    /// // The map file's cell (7, 3) is this one, and it knows nothing about offset coordinates.
    /// let cell = g.at(Offset::OddR.to_hex(7, 3));
    /// assert_eq!(Offset::OddR.from_hex(g.coord(cell)), (7, 3));
    /// ```
    ///
    /// # Panics
    ///
    /// If `w` or `h` is negative, or if the two together would ask for more cells than a grid can
    /// hold. See [`FullGrid::square`], which guards the same two mistakes for the same reasons.
    #[must_use]
    pub fn hex_rect(w: i32, h: i32, offset: Offset) -> Self {
        assert!(w >= 0 && h >= 0, "a grid cannot be {w} x {h}");
        assert!(
            w as u64 * h as u64 <= MAX_CELLS,
            "{w} x {h} is {} cells; a grid may hold at most {MAX_CELLS}",
            w as u64 * h as u64,
        );

        let cells = (0..h).flat_map(move |row| (0..w).map(move |col| offset.to_hex(col, row)));
        Self::new(cells, &Dir6::ALL, Metric::HEX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn a_rectangle_has_width_times_height_cells() {
        let g = FullGrid::square(5, 3, Adjacency::Four);
        assert_eq!(g.len(), 15);

        let mut cells = g.cells();
        assert_eq!(
            cells.next(),
            Some(Sq::new(0, 0)),
            "row-major: index 0 is the top-left"
        );
        assert_eq!(
            cells.nth(3),
            Some(Sq::new(4, 0)),
            "then along the first row"
        );
        assert_eq!(cells.next(), Some(Sq::new(0, 1)), "then down to the second");
    }

    #[test]
    fn indices_round_trip_through_coordinates() {
        let g = FullGrid::square(4, 4, Adjacency::Eight);
        for i in g.indices() {
            assert_eq!(g.index_of(g.coord(i)), Some(i));
        }
    }

    #[test]
    fn four_way_grids_have_no_diagonal_steps_at_all() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let mid = g.at(Sq::new(1, 1));

        assert_eq!(g.neighbors(mid).count(), 4);
        assert!(g.step(mid, Dir8::Ne).is_none());
    }

    #[test]
    fn eight_way_grids_have_eight_neighbours_in_the_middle_and_three_in_a_corner() {
        let g = FullGrid::square(3, 3, Adjacency::Eight);
        assert_eq!(g.neighbors(g.at(Sq::new(1, 1))).count(), 8);
        assert_eq!(g.neighbors(g.at(Sq::new(0, 0))).count(), 3);
    }

    #[test]
    fn steps_off_the_board_are_none() {
        let g = FullGrid::square(3, 3, Adjacency::Eight);
        let corner = g.at(Sq::new(0, 0));
        assert!(g.step(corner, Dir8::N).is_none());
        assert!(g.step(corner, Dir8::W).is_none());
        assert!(g.step(corner, Dir8::Se).is_some());
    }

    #[test]
    fn a_disc_keeps_the_cell_exactly_on_the_rim_and_drops_the_corner_of_the_box() {
        // 1, 5, 13, 29, 49: the Gauss circle counts, written out because they have no closed form
        // and so cannot be restated from the code under test. A `<` where the rule says `<=` gives
        // 0, 1, 9, 25, 45 — a board that is still perfectly round, is EMPTY at radius 0, and is the
        // wrong size everywhere else.
        let counts: Vec<usize> = (0..5)
            .map(|r| FullGrid::disc(r, Adjacency::Four).len())
            .collect();
        assert_eq!(counts, vec![1, 5, 13, 29, 49]);

        let g = FullGrid::disc(5, Adjacency::Four);
        assert!(
            g.contains(Sq::new(3, 4)),
            "3-4-5: on the rim, and the rim is in"
        );
        assert!(
            !g.contains(Sq::new(4, 4)),
            "inside the box, outside the circle"
        );
    }

    #[test]
    fn a_disc_measures_the_diagonal_the_way_its_adjacency_moves() {
        // The constructor has two arms, and only one wrong pairing announces itself: eight
        // directions with Manhattan distance panics inside `FullGrid::new`. Four directions with
        // Chebyshev does not — it builds a four-way board that calls the diagonal one step away, so
        // a unit reports the enemy beside it as in range and then cannot walk to it.
        for (adj, dirs, diagonal) in [(Adjacency::Four, 4, 2), (Adjacency::Eight, 8, 1)] {
            let g = FullGrid::disc(3, adj);
            let centre = g.at(Sq::new(0, 0));
            let corner = g.at(Sq::new(1, 1));

            assert_eq!(g.neighbors(centre).count(), dirs, "{adj:?}");
            assert_eq!(g.distance(centre, corner), diagonal, "{adj:?}");
        }
    }

    #[test]
    fn a_hexagon_of_radius_r_has_the_centred_hexagonal_number_of_cells() {
        // 1, 7, 19, 37: 3r(r+1) + 1.
        for r in 0..4 {
            assert_eq!(
                FullGrid::hexagon(r).len() as i32,
                3 * r * (r + 1) + 1,
                "radius {r}"
            );
        }
    }

    #[test]
    fn a_hex_cell_has_six_neighbours_unless_it_is_on_the_rim() {
        let g = FullGrid::hexagon(2);
        let centre = g.at(Hex::new(0, 0));
        assert_eq!(g.neighbors(centre).count(), 6);

        let rim = g.at(Hex::new(2, 0));
        assert_eq!(g.neighbors(rim).count(), 3);
    }

    /// The four staggering conventions, which a hex rectangle must handle alike.
    const OFFSETS: [Offset; 4] = [Offset::OddR, Offset::EvenR, Offset::OddQ, Offset::EvenQ];

    #[test]
    fn a_hex_rectangle_holds_exactly_width_times_height_cells() {
        // No convention may drop a cell by staggering two rows onto each other, nor gain one.
        for o in OFFSETS {
            for (w, h) in [(1, 1), (1, 7), (7, 1), (5, 4), (9, 8)] {
                assert_eq!(
                    FullGrid::hex_rect(w, h, o).len() as i32,
                    w * h,
                    "{o:?} {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn every_cell_of_a_hex_rectangle_is_the_tilemap_cell_it_was_built_from() {
        // The interop claim: a map file's (col, row) names one cell here, and that cell names the
        // same (col, row) back. Without it, loading an authored map puts the terrain in the wrong
        // places and nothing complains.
        for o in OFFSETS {
            let g = FullGrid::hex_rect(6, 5, o);
            for row in 0..5 {
                for col in 0..6 {
                    let i = g.index_of(o.to_hex(col, row));
                    let i = i.unwrap_or_else(|| panic!("{o:?} has no ({col}, {row})"));
                    assert_eq!(o.from_hex(g.coord(i)), (col, row), "{o:?}");
                }
            }
        }
    }

    #[test]
    fn filtered_drops_cells_and_the_steps_into_them() {
        let full = FullGrid::square(3, 3, Adjacency::Four);
        let holed = full.filtered(|c| c != Sq::new(1, 1));

        assert_eq!(holed.len(), 8);
        assert!(!holed.contains(Sq::new(1, 1)));

        // The cell above the hole can no longer step down into it.
        let above = holed.at(Sq::new(1, 0));
        assert!(holed.step(above, Dir8::S).is_none());
    }

    #[test]
    fn duplicate_cells_are_dropped_keeping_the_first() {
        let g = FullGrid::new(
            [Sq::new(0, 0), Sq::new(1, 0), Sq::new(0, 0)],
            &Dir8::ORTHO,
            Metric::MANHATTAN,
        );
        assert_eq!(g.len(), 2);
        assert_eq!(
            g.index_of(Sq::new(0, 0)),
            g.indices().next(),
            "the first occurrence kept its place, so it is still cell zero"
        );
    }
}
