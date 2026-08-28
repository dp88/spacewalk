//! What every board can answer, whatever it is made of.
//!
//! [`Grid`] is the vocabulary: cells, the steps between them, distance, sight, and reach. Three
//! things speak it, and code written against it does not care which it was handed —
//! [`FullGrid`], a board you built; [`RectGrid`](crate::RectGrid), a rectangle it computes; and
//! [`SubGrid`], a region of either.
//!
//! The trait asks for eleven methods and gives back twenty. Everything from `ray` to `path` is
//! written in terms of the geometry primitives below and needs no storage of its own, which is why
//! a region of a board can be a board without copying one.

use crate::coord::{Coord, Idx, Metric, Tag};
// The trait no longer names a `FullGrid`, but the documentation below links to one throughout.
#[allow(unused_imports)]
use crate::full::FullGrid;
use crate::path::{Cost, Movement, Path, Step};
use crate::sub::SubGrid;
use alloc::vec;
use alloc::vec::Vec;

/// The largest sight radius [`Grid::visible_from`] will attempt: 64.
///
/// Raycasting draws a line to every candidate cell, so its cost grows as the **cube** of the radius.
/// At 64 that is a few milliseconds; at 1000 it is the better part of a minute of solid compute,
/// which inside a game loop is a hang that happens to be made of time rather than memory. Same bug
/// class, different resource.
///
/// A 129 × 129 field of view is far more than any roguelike shows. If you truly need more, you want
/// shadowcasting — a different algorithm, not this one with the brakes off.
pub const MAX_SIGHT: u32 = 64;

/// The direction type of a board's coordinate: [`Dir8`](crate::Dir8) for squares,
/// [`Dir6`](crate::Dir6) for hexes, whatever your own [`Coord`] declares.
///
/// A spelling convenience. `Dir<B>` is `<<B as Grid>::Cell as Coord>::Dir`, which is the sort of
/// thing that belongs behind a name.
pub type Dir<B> = <<B as Grid>::Cell as Coord>::Dir;

/// One cell on a sight line: who is looking, what they are looking at, and the cell in between
/// being tested.
///
/// This is to sight what [`Step`] is to movement. A cost function is handed the whole step rather
/// than left to derive its own direction, and for the same reason a blocker is handed the whole
/// question. Whether a cell stops sight is not always a property of that cell alone — a hill blocks
/// only what is lower than it, a window looks one way, a crate hides a crouching unit and not a
/// standing one. Each of those needs to know who is looking, and at what.
///
/// [`Grid::los_by`] and [`Grid::visible_from_by`] take a predicate over this.
/// [`Grid::los`] and [`Grid::visible_from`] are the same thing with `target` thrown away.
///
/// # These are the receiver's indices
///
/// All three are numbered by the board you asked, not by its root. A field of view taken on a
/// [`SubGrid`] hands the predicate `SubGrid` indices, so read your own tables through
/// [`Grid::to_root`] if they are keyed against the whole board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sight {
    /// Where the looker stands. Never itself tested.
    pub eye: Idx,
    /// The cell being tested. This is the one that may stop sight.
    pub at: Idx,
    /// What the looker is trying to see. Never itself tested — you can see the wall you look at.
    pub target: Idx,
}

/// Check that `i` belongs to this board, and turn it into a slot in the board's own tables.
///
/// Two failures, and the second is the one that used to get through. The bounds check catches an
/// index past the end. The tag check catches an index that is *in* range but was issued by a
/// different board — two boards of the same size, where no bound can tell them apart. That case
/// was silent before, and `full.rs` called it the one way to get a wrong answer out of this crate
/// without hearing about it.
///
/// The tag check is a `debug_assert`, so a shipped build pays nothing. See [`Tag`].
#[track_caller]
pub(crate) fn slot(len: usize, tag: Tag, i: Idx) -> usize {
    debug_assert!(
        i.tag().agrees(tag),
        "cell {i} was issued by a different grid than the one being asked \
         (indices are per-grid, and this one is in range for both, so nothing else can catch it). \
         Look the cell up again with `Grid::index_of` or `Grid::at` on the grid you mean.",
    );
    assert!(
        (i.raw() as usize) < len,
        "cell {i} is not on this grid, which has {len} cells (indices are per-grid; \
         a stale one from before `filtered` or a subset renumbered will not do)",
    );
    i.raw() as usize
}

/// Check only that `i` came from this board, for the places where being off it is an answer.
///
/// [`Grid::of_root`] asks whether a cell is in a region and says `None` when it is not, so a bound
/// is not a failure there. Coming from the wrong board still is.
#[track_caller]
pub(crate) fn same_grid(tag: Tag, i: Idx) {
    debug_assert!(
        i.tag().agrees(tag),
        "cell {i} was issued by a different grid than the one being asked \
         (indices are per-grid, and this one may well be in range for both). \
         Look the cell up again with `Grid::index_of` or `Grid::at` on the grid you mean.",
    );
}

/// The most a single step may cost on a board of `len` cells before a path could overflow [`Cost`].
///
/// No simple path visits a cell twice, so it takes at most `len - 1` steps. Keep every step under
/// this and no total can overflow — which matters more than it sounds, because an overflowing total
/// does not merely give a wrong answer; it hangs.
pub(crate) fn cost_ceiling(len: usize) -> Cost {
    Cost::MAX / (len.saturating_sub(1).max(1) as Cost)
}

/// A board: cells, the steps between them, and the questions you may ask about both.
///
/// Implement this only if you are storing cells in a way this crate does not — it is the interface,
/// not the extension point. To add a *shape* or a *geometry*, implement [`Coord`] and hand your
/// cells to [`FullGrid::new`]; that is a few dozen lines and needs no change here.
///
/// # The eleven you write, the twenty you get
///
/// Everything above the divider in the source is required and small: the cell count, the coordinate
/// at an index and back, the direction alphabet, one step, the neighbours out and in, the metric,
/// and the three that say where this board sits relative to the one that owns the cells. Everything
/// else — rays, runs, ranges, lines, sight, components, and all of pathfinding — is written in
/// terms of those, once, here.
///
/// # Indices are per-board
///
/// An [`Idx`] means something only to the board that issued it. A [`SubGrid`] numbers its own cells
/// from zero, so its indices and its root's are **both valid and mutually wrong**. [`to_root`] and
/// [`of_root`] are the bridge, and they are the only correct one.
///
/// A debug build catches the mistake for you: an index carries a [`Tag`] naming its board, and
/// every method here checks it. In release the tag is zero-sized and the checks are gone.
///
/// # What each question hands back
///
/// One rule, applied throughout. An **iterator** when the walk is lazy and the caller may stop
/// early ([`indices`](Grid::indices), [`cells`](Grid::cells), [`ray`](Grid::ray)). A [`SubGrid`]
/// when the answer *is* a board and you will go on to ask it things ([`within`](Grid::within),
/// [`ring`](Grid::ring), [`component`](Grid::component), [`visible_from`](Grid::visible_from)). A
/// `Vec` when the walk must finish before any of it is correct — [`run`](Grid::run) reads both ways
/// from its anchor, [`line`](Grid::line) is built from both ends, and the searches must settle
/// before a cost is final.
///
/// # This trait is not dyn compatible
///
/// [`neighbors`](Grid::neighbors) returns `impl Iterator`, so there is no `Box<dyn Grid>`. That is
/// deliberate: a vtable here would put an allocation on the hottest loop in the crate, and every
/// search walks neighbours.
///
/// Write generic code instead — `fn f<B: Grid>(g: &B, c: B::Cell)` reads no worse and costs
/// nothing, and it takes a [`FullGrid`], a [`RectGrid`](crate::RectGrid), and a [`SubGrid`] alike.
/// If you genuinely must choose a board shape at runtime, an `enum` over the two or three you
/// actually ship is the answer, and it stays static.
///
/// [`to_root`]: Grid::to_root
/// [`of_root`]: Grid::of_root
pub trait Grid {
    /// The coordinate this board is laid out on: [`Sq`](crate::Sq), [`Hex`](crate::Hex), or yours.
    type Cell: Coord;

    /// The whole board these cells belong to: `Self` for one you built, and the board it was taken
    /// from for a [`SubGrid`].
    ///
    /// A region is numbered against its root, so this is what [`to_root`](Grid::to_root) and every
    /// range query speak in terms of. It is an associated type rather than [`FullGrid`] because a
    /// whole board need not store its cells — [`RectGrid`](crate::RectGrid) computes them.
    type Root: Grid<Cell = Self::Cell>;

    // -- required: the geometry primitives ---------------------------------------------------

    /// This board's numbering, for checking the indices handed to it. See [`Tag`].
    ///
    /// Derive it from the cells in index order, with [`Tag::of`]. Two boards that number the same
    /// cells the same way must agree, or an index that *should* travel between them will trip the
    /// check.
    fn tag(&self) -> Tag;

    /// How many cells the board has.
    fn len(&self) -> usize;

    /// The coordinate at an index.
    ///
    /// # Panics
    /// If `i` is not a cell of this board.
    fn coord(&self, i: Idx) -> Self::Cell;

    /// The index of a coordinate, or `None` if it is not on the board.
    ///
    /// [`Grid::at`] is the same question when the cell is one you already know is there.
    fn index_of(&self, c: Self::Cell) -> Option<Idx>;

    /// The direction alphabet this board was built with.
    fn dirs(&self) -> &[Dir<Self>];

    /// The cell one step from `i` in direction `d`, or `None` at the board's edge.
    ///
    /// This is the primitive the rest of the crate is built from, and it *keeps the direction*,
    /// which is what makes forward-only pieces and directional costs expressible.
    ///
    /// It respects holes — you cannot step into a cell that is not there. For the hole-ignoring
    /// version, which is what a knight's leap or a capture-by-jump needs, see [`Grid::offset`].
    ///
    /// # Panics
    /// If `i` is not a cell of this board.
    fn step(&self, i: Idx, d: Dir<Self>) -> Option<Idx>;

    /// Every neighbour of `i`, with the direction that reaches it.
    ///
    /// # Panics
    /// If `i` is not a cell of this board.
    fn neighbors(&self, i: Idx) -> impl Iterator<Item = (Dir<Self>, Idx)>;

    /// The cells that can step **into** `j`, and the direction each would travel to do it.
    ///
    /// The mirror of [`Grid::neighbors`], and not the same set — the graph is directed, so a cell
    /// you can leave towards is not necessarily one you can arrive from. This is what makes a threat
    /// map possible; see [`Grid::reaching`].
    ///
    /// ```
    /// use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(3, 3, Adjacency::Four);
    /// let centre = board.at(Sq::new(1, 1));
    /// let north = board.at(Sq::new(1, 0));
    ///
    /// // The direction is the one the arriving piece travels: from the north, heading south.
    /// assert!(board.in_neighbors(centre).any(|(d, from)| from == north && d == Dir8::S));
    /// ```
    ///
    /// # Panics
    /// If `j` is not a cell of this board.
    fn in_neighbors(&self, j: Idx) -> impl Iterator<Item = (Dir<Self>, Idx)>;

    /// How distance is measured and range queries are answered. See [`Metric`].
    fn metric(&self) -> Metric<Self::Cell>;

    /// The board that owns these cells. A whole board is its own root.
    fn root(&self) -> &Self::Root;

    /// This board's index for a cell, as the [`root`](Grid::root) numbers it.
    ///
    /// The bridge to everything you keep beside the board — a [`CellMap`](crate::CellMap), a
    /// `Vec<Terrain>`, your own array. Those are keyed by the root's numbering, and a
    /// [`SubGrid`]'s are not.
    ///
    /// # Panics
    /// If `i` is not a cell of this board.
    fn to_root(&self, i: Idx) -> Idx;

    /// Where a cell of the [`root`](Grid::root) sits on this board, or `None` if it is not on it.
    ///
    /// The inverse of [`to_root`](Grid::to_root), and the way to ask whether a region holds a cell
    /// you already have an index for.
    fn of_root(&self, i: Idx) -> Option<Idx>;

    // -- provided: everything else ------------------------------------------------------------

    /// Whether the board has no cells at all.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The index of a coordinate you already know is on the board.
    ///
    /// The same question as [`index_of`](Grid::index_of), for the common case where a `None` would
    /// only ever be unwrapped. Most cells a game asks about came from the board in the first place —
    /// a unit's position, a tile the map file named, the cell under the mouse *after* it was
    /// checked — and `index_of(c).unwrap()` says nothing about which one went wrong when it does.
    ///
    /// Use [`index_of`](Grid::index_of) when off-board is an answer rather than a mistake.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// assert_eq!(g.at(Sq::new(3, 3)), g.at(Sq::new(3, 3)));
    /// ```
    ///
    /// # Panics
    /// If `c` is not on this board, naming the coordinate.
    #[track_caller]
    fn at(&self, c: Self::Cell) -> Idx {
        match self.index_of(c) {
            Some(i) => i,
            None => panic!(
                "{c:?} is not a cell of this grid, which has {} (use `index_of` if being off the \
                 board is an answer rather than a mistake)",
                self.len(),
            ),
        }
    }

    /// The cell one step from a coordinate, or `None` if either the coordinate or the destination
    /// is not on this board.
    #[must_use]
    fn step_from(&self, c: Self::Cell, d: Dir<Self>) -> Option<Self::Cell> {
        let i = self.index_of(c)?;
        self.step(i, d).map(|j| self.coord(j))
    }

    /// Every cell index, in order. The usual way to sweep a board.
    fn indices(&self) -> impl Iterator<Item = Idx> {
        let tag = self.tag();
        #[allow(clippy::cast_possible_truncation)]
        (0..self.len() as u32).map(move |i| Idx::new(tag, i))
    }

    /// Every cell's coordinate, in index order.
    ///
    /// This is what you save. Hand it back to [`FullGrid::new`] with the same directions and metric
    /// and you get the same board, with the same indices — see `tests/save.rs`.
    fn cells(&self) -> impl Iterator<Item = Self::Cell> {
        self.indices().map(|i| self.coord(i))
    }

    /// The coordinates of some indices, in the order given.
    ///
    /// Every answer this crate gives is in indices, and everything outside it — drawing, saving,
    /// your own tables keyed by coordinate — wants cells. This is that step, for a path's steps, a
    /// range query's results, or anything else you are holding.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let walk = Movement::uniform(&g, 1);
    /// let path = g.path(g.at(Sq::new(0, 0)), g.at(Sq::new(2, 0)), &walk).unwrap();
    ///
    /// let route: Vec<Sq> = g.coords_of(path.steps().iter().copied()).collect();
    /// assert_eq!(route, [Sq::new(0, 0), Sq::new(1, 0), Sq::new(2, 0)]);
    /// ```
    ///
    /// # Panics
    /// If any index is not a cell of this board.
    fn coords_of(&self, of: impl IntoIterator<Item = Idx>) -> impl Iterator<Item = Self::Cell> {
        of.into_iter().map(|i| self.coord(i))
    }

    /// Whether a coordinate is on the board.
    fn contains(&self, c: Self::Cell) -> bool {
        self.index_of(c).is_some()
    }

    /// A region of this board, as a board of its own.
    ///
    /// This is the answer to "let me work over here": the cells you name become a
    /// [`SubGrid`], which answers every question above about *itself*. A path inside a movement
    /// range stays inside it. A component of a room is a component of that room. Steps that would
    /// leave the region are the edge of the board, exactly as the outside of a map is.
    ///
    /// Cheap on purpose. A `SubGrid` borrows the cells rather than copying them, so this is a sort,
    /// not a board rebuild — which is what lets a range query hand one back.
    ///
    /// The order you list cells in does not reach the result: a subset numbers its cells in the
    /// root's order, and duplicates collapse.
    ///
    /// Subsets do not nest. Narrowing a region — the reachable cells that are also in sight, say —
    /// gives another region of the same [`root`](Grid::root), so [`to_root`](Grid::to_root) is one
    /// hop however many times you narrow.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let corner = g.subset(g.indices().filter(|&i| g.coord(i).x < 2 && g.coord(i).y < 2));
    ///
    /// assert_eq!(corner.len(), 4, "a 2x2 board in its own right");
    /// let a = corner.at(Sq::new(0, 0));
    /// assert_eq!(corner.neighbors(a).count(), 2, "and its own edges");
    /// ```
    fn subset(&self, cells: impl IntoIterator<Item = Idx>) -> SubGrid<'_, Self::Root> {
        SubGrid::of(self.root(), cells.into_iter().map(|i| self.to_root(i)))
    }

    /// The cells within a coordinate range, or `None` if the origin is not on this board.
    #[must_use]
    fn within_cell(&self, c: Self::Cell, min: u32, max: u32) -> Option<SubGrid<'_, Self::Root>> {
        self.index_of(c).map(|i| self.within(i, min, max))
    }

    // -- geometry ------------------------------------------------------------------------------

    /// Walk from `i` in a straight line until the board runs out.
    ///
    /// A rook, a bishop, a queen, a line of sight. The walk stops at the board's edge and at any
    /// hole; the caller stops it earlier by taking while a cell is empty.
    ///
    /// ```
    /// use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(8, 8, Adjacency::Eight);
    /// let a1 = board.at(Sq::new(0, 7));
    ///
    /// // A rook on a1 slides up the file: a2..a8, seven squares.
    /// assert_eq!(board.ray(a1, Dir8::N).count(), 7);
    ///
    /// // A bishop on a1 slides the long diagonal: b2..h8.
    /// let diagonal: Vec<Sq> = board.ray(a1, Dir8::Ne).map(|i| board.coord(i)).collect();
    /// assert_eq!(diagonal.first(), Some(&Sq::new(1, 6)));
    /// assert_eq!(diagonal.len(), 7);
    /// ```
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    fn ray(&self, i: Idx, d: Dir<Self>) -> impl Iterator<Item = Idx> {
        let _ = slot(self.len(), self.tag(), i);

        // Bounded by the board, deliberately. A straight line cannot visit more cells than exist,
        // so this changes no correct answer — but a `Coord` whose `step` wraps (a torus world is a
        // perfectly ordinary thing to want) makes the step table cyclic, and an unbounded walk down
        // it never returns. `.collect()` on that fills memory until the machine dies.
        core::iter::successors(self.step(i, d), move |&j| self.step(j, d)).take(self.len())
    }

    /// The unbroken line of cells through `i` along the `d` axis, in board order.
    ///
    /// Line-of-five, a flanking check, the length of a wall segment. [`Grid::ray`] slides one way
    /// and stops only at the board's edge; this walks **both** ways and stops at `same`.
    ///
    /// The answer reads along `d`: the cells behind `i` first, then `i`, then the cells ahead of
    /// it. So `run(i, E, …)` reads west to east, and asking along the opposite direction gives the
    /// same cells reversed.
    ///
    /// # `i` is the anchor, and is never tested
    ///
    /// `same` is asked about every cell except `i`, and `i` is always in the answer — so a run is
    /// never empty. That is what lets you ask *what if I played here* without writing to your
    /// board first:
    ///
    /// ```
    /// use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Sq};
    ///
    /// let g = FullGrid::square(9, 9, Adjacency::Eight);
    /// let at = |x, y| g.at(Sq::new(x, y));
    /// let mine = [Sq::new(2, 4), Sq::new(3, 4), Sq::new(5, 4), Sq::new(6, 4)];
    ///
    /// // Two pairs with a gap between them. Playing the gap would join them into five.
    /// let line = g.run(at(4, 4), Dir8::E, |i| mine.contains(&g.coord(i)));
    ///
    /// assert_eq!(line.len(), 5, "and the board was never touched");
    /// assert_eq!(g.coord(line[0]), Sq::new(2, 4), "the west end comes first, along E");
    /// ```
    ///
    /// # It walks the real edges, in both directions
    ///
    /// Backwards it follows [`Grid::in_neighbors`], not `d` turned around. The graph is directed,
    /// so those are not the same thing: on a board of one-way ledges the north step does not exist
    /// at all, yet the cell above still steps south into this one, and the line through them is
    /// real. It also asks nothing of `Dir` beyond `Eq`, so a coordinate of your own needs no
    /// notion of an opposite.
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    #[must_use]
    fn run(&self, i: Idx, d: Dir<Self>, same: impl Fn(Idx) -> bool) -> Vec<Idx> {
        let _ = slot(self.len(), self.tag(), i);

        // The reverse leg reads the in-edges rather than turning `d` around, which is what lets a
        // `Coord::Dir` get away with knowing nothing about its own opposite. Where several cells
        // step into one in the same direction — which a clamping `Coord::step` can produce — the
        // lowest-indexed wins, because `in_neighbors` yields them in index order.
        let behind = |j: Idx| {
            self.in_neighbors(j)
                .find(|&(dir, _)| dir == d)
                .map(|(_, f)| f)
        };

        // Bounded by the board on both legs, for the reason `ray` is: a wrapping `Coord::step`
        // makes the step table cyclic, and an unbounded walk down it never returns.
        let mut line: Vec<Idx> = core::iter::successors(behind(i), |&j| behind(j))
            .take(self.len())
            .take_while(|&j| same(j))
            .collect();

        line.reverse();
        line.push(i);
        line.extend(self.ray(i, d).take_while(|&j| same(j)));
        line
    }

    /// The cell at `coord(i) + delta`, or `None` if there is no such cell.
    ///
    /// A *lattice* hop, not a walk: it does not care what lies between, so it leaps over holes and
    /// over pieces. That is exactly what a knight does, and what a capture-by-jump does — a jump
    /// must be able to cross a gap in the board, so it cannot be a two-step graph walk.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(8, 8, Adjacency::Eight);
    /// let b1 = board.at(Sq::new(1, 7));
    ///
    /// // A knight on b1 leaps to a3 and c3, over its own back rank.
    /// let leaps: Vec<Sq> = [Sq::new(-1, -2), Sq::new(1, -2)]
    ///     .iter()
    ///     .filter_map(|&d| board.offset(b1, d))
    ///     .map(|i| board.coord(i))
    ///     .collect();
    /// assert_eq!(leaps, vec![Sq::new(0, 5), Sq::new(2, 5)]);
    /// ```
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    #[must_use]
    fn offset(&self, i: Idx, delta: Self::Cell) -> Option<Idx> {
        let _ = slot(self.len(), self.tag(), i);
        self.index_of(self.coord(i) + delta)
    }

    // -- metric --------------------------------------------------------------------------------

    /// The distance between two cells, under the metric this board was built with.
    ///
    /// The same metric drives attack range, vision, and the A\* heuristic — so they cannot
    /// disagree with each other, and none of them can disagree with the adjacency.
    ///
    /// A metric measures **coordinates**, so a [`SubGrid`] measures exactly as its root does. A
    /// region does not bring anything closer together.
    ///
    /// # Panics
    ///
    /// If `a` or `b` is not a cell of this board.
    #[must_use]
    fn distance(&self, a: Idx, b: Idx) -> u32 {
        let _ = slot(self.len(), self.tag(), a);
        let _ = slot(self.len(), self.tag(), b);
        self.metric().distance(self.coord(a), self.coord(b))
    }

    /// Every cell whose distance from `i` is in `min..=max`. Excludes `i` unless `min` is 0.
    ///
    /// Attack range, blast radius, vision — which is why it hands back a board rather than a list:
    /// the blast is the thing you highlight, and it is also the thing you then ask questions about.
    ///
    /// It measures in *coordinates*, not in steps, so walls and holes do not shorten it — a range-2
    /// archer shoots over a wall. If you want the shot blocked, use [`Grid::visible_from`].
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(8, 8, Adjacency::Four);
    /// let archer = board.at(Sq::new(3, 3));
    ///
    /// // A range 1-2 attack covers the diamond around the archer — their own cell not included.
    /// assert_eq!(board.within(archer, 1, 2).len(), 12);
    /// ```
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    #[must_use]
    fn within(&self, i: Idx, min: u32, max: u32) -> SubGrid<'_, Self::Root> {
        let _ = slot(self.len(), self.tag(), i);
        if min > max {
            return self.subset([]);
        }
        let (c, metric) = (self.coord(i), self.metric());

        // Two ways to answer this, and they agree. Walking the metric's offsets costs O(radius²)
        // — the disc, not its rim; scanning the board costs O(cells). Take the cheaper — and note
        // that `count` is what makes the choice *safe*, not merely fast: it reports how big the
        // offset list would be without building it, so a preposterous radius routes to the scan
        // instead of trying to allocate the universe. Both branches are bounded by the board.
        if metric.count(max) <= self.len() as u64 {
            let hit: Vec<Idx> = metric
                .deltas(max)
                .into_iter()
                .filter(|&(_, d)| d >= min)
                .filter_map(|(delta, _)| self.index_of(c + delta))
                .collect();
            self.subset(hit)
        } else {
            self.subset(
                self.indices()
                    .filter(|&j| (min..=max).contains(&metric.distance(c, self.coord(j)))),
            )
        }
    }

    /// Every cell at exactly distance `r`. `within(i, r, r)`.
    ///
    /// In a clone-and-jump capture game these are the two moves: `ring(i, 1)` is a clone, and
    /// `ring(i, 2)` is a jump — which must be able to cross a hole, so it cannot be a graph walk.
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    #[must_use]
    fn ring(&self, i: Idx, r: u32) -> SubGrid<'_, Self::Root> {
        self.within(i, r, r)
    }

    /// The cells on the straight line from `a` to `b`, `a` first and `b` last.
    ///
    /// A list, not a region, because the **order** is the answer: a line is walked, and what stops
    /// it is where along it the obstacle sits.
    ///
    /// Cells the line crosses that are not on the board are skipped, so a line over a hole simply
    /// has a gap in it. Empty if this board's metric has no [`lerp`](Metric::lerp).
    ///
    /// When the metric has a `lerp`, both endpoints are always present, first and last. That is not
    /// free either: interpolation rounds through a clamp at the lattice limit, so a cell further
    /// out than that cannot be rounded back to and would otherwise be missing from its own line.
    ///
    /// **Cells beyond that limit are still missed in between.** The same clamp sends every sampled
    /// position out there to the same coordinate, so a line joining two cells further than 2³⁰ from
    /// the origin reports only its endpoints, and sight along it stops at nothing. The standard
    /// shape constructors cannot reach that far; this bites only a board built from deliberately
    /// extreme coordinates.
    ///
    /// Symmetric: `line(a, b)` is `line(b, a)` reversed. That is not free — rounding a tie breaks
    /// one way or the other — so the line is always computed from the lower coordinate and flipped
    /// if needed. Without it you get a board where A can see B but B cannot see A, which players
    /// notice.
    ///
    /// # Panics
    ///
    /// If `a` or `b` is not a cell of this board.
    #[must_use]
    fn line(&self, a: Idx, b: Idx) -> Vec<Idx> {
        let _ = slot(self.len(), self.tag(), a);
        let _ = slot(self.len(), self.tag(), b);

        let metric = self.metric();
        if !metric.has_lerp() {
            return Vec::new();
        }
        if a == b {
            return vec![a]; // and never divide by zero, nor round a NaN into the origin
        }

        let ordered = self.coord(a) < self.coord(b);
        let (lo, hi) = if ordered { (a, b) } else { (b, a) };
        let (ca, cb) = (self.coord(lo), self.coord(hi));

        // Dense boards can interpolate one lattice step at a time. On a sparse board the
        // coordinate distance can hugely exceed the cell count — two cells a billion apart would
        // otherwise walk a billion steps to visit two. In that case, locate the line's sampled
        // positions by scanning the board instead. This keeps the answer exact without making work
        // proportional to a coordinate a caller supplied.
        let distance = self.distance(lo, hi);
        if distance > self.len() as u32 {
            let mut cells: Vec<(u32, Idx)> = self
                .indices()
                .filter_map(|j| {
                    let at = self.distance(lo, j);
                    // The endpoints are on their own line by definition, and are kept without
                    // asking the lerp. It rounds through a clamp at the lattice limit, so a
                    // coordinate past that never matches itself — an endpoint used to be dropped
                    // from its own line, and `los` then skipped a real blocker in its place.
                    (j == lo
                        || j == hi
                        || (at <= distance
                            && metric.lerp(ca, cb, at, distance) == Some(self.coord(j))))
                    .then_some((at, j))
                })
                .collect();
            cells.sort_unstable_by_key(|&(at, j)| (at, j));
            if lo != a {
                cells.reverse();
            }
            return cells.into_iter().map(|(_, j)| j).collect();
        }

        let n = distance.max(1);

        let mut cells: Vec<Idx> = (0..=n)
            .filter_map(|t| self.index_of(metric.lerp(ca, cb, t, n)?))
            .collect();
        cells.dedup();
        // Same guarantee as the sparse branch above. Past the lattice limit the interpolation
        // cannot round to either end, and this branch lost both — returning an empty line, which
        // blocks nothing, so sight ran clean through a wall standing between two neighbours.
        if cells.first() != Some(&lo) {
            cells.insert(0, lo);
        }
        if cells.last() != Some(&hi) {
            cells.push(hi);
        }
        if lo != a {
            cells.reverse();
        }
        cells
    }

    /// Can `a` see `b`? `blocks` says which cells stop sight.
    ///
    /// `b` itself may be a blocker and still be seen — you can see the wall you are looking at. The
    /// cell you are standing in is never consulted.
    ///
    /// When what blocks depends on *who is looking* — elevation, cover, a one-way window — see
    /// [`Grid::los_by`], which hands the predicate the whole question.
    ///
    /// # Panics
    ///
    /// If `a` or `b` is not a cell of this board.
    fn los(&self, a: Idx, b: Idx, blocks: impl Fn(Idx) -> bool) -> bool {
        self.los_by(a, b, |s| blocks(s.at))
    }

    /// Can `a` see `b`, when what blocks depends on the looker and the target?
    ///
    /// The same walk as [`Grid::los`], with a predicate over [`Sight`] rather than over a bare
    /// index. That is the whole difference, and it is what a height field needs: a hill hides only
    /// what is lower than the line over it, so a blocker cannot be decided from its own cell alone.
    /// See [`height_gate`](crate::height_gate).
    ///
    /// The predicate is never asked about `a` or `b` themselves.
    ///
    /// # A lattice with no straight line sees everything
    ///
    /// Sight is drawn along [`Grid::line`], and a [`Metric`] without a
    /// [`lerp`](Metric::with_lerp) has no line to draw — so there are no cells to block on, and
    /// this answers `true`. That is the long-standing behaviour of [`Grid::los`], preserved here.
    /// A board like `tests/chess3d.rs` should not ask.
    ///
    /// # Panics
    ///
    /// If `a` or `b` is not a cell of this board.
    fn los_by(&self, a: Idx, b: Idx, blocks: impl Fn(Sight) -> bool) -> bool {
        self.line(a, b).into_iter().all(|at| {
            at == a
                || at == b
                || !blocks(Sight {
                    eye: a,
                    at,
                    target: b,
                })
        })
    }

    /// Every cell within `r` of `i` that can actually be seen.
    ///
    /// The field of view, as a board — so the cells a unit can see are also the cells it can be
    /// asked questions about, and highlighting them is the same object as reasoning over them.
    ///
    /// Naive raycasting: it draws a line to each candidate. That is O(r³), which is why `r` is
    /// capped — see below. It is a fraction of the code of proper shadowcasting and honest about
    /// what it costs; if you need a sight radius bigger than [`MAX_SIGHT`], you want shadowcasting,
    /// not this with the brakes off.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(5, 5, Adjacency::Eight);
    /// let eye = board.at(Sq::new(0, 2));
    /// let pillar = board.at(Sq::new(2, 2));
    /// let behind = board.at(Sq::new(4, 2));
    ///
    /// let seen = board.visible_from(eye, 4, |i| i == pillar);
    /// assert!(seen.contains(board.coord(pillar)), "you can see the wall you are looking at");
    /// assert!(!seen.contains(board.coord(behind)), "but not through it");
    /// ```
    ///
    /// A blocker that depends on *who is looking* — a height field, cover, a one-way window —
    /// cannot be written as `Fn(Idx) -> bool`. See [`Grid::visible_from_by`].
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board, or `r` exceeds [`MAX_SIGHT`]. The cap is not fussiness:
    /// the work grows as the cube of the radius, so `r = 1000` is tens of seconds of solid compute —
    /// a hang, arriving by way of time rather than memory.
    #[must_use]
    fn visible_from(
        &self,
        i: Idx,
        r: u32,
        blocks: impl Fn(Idx) -> bool,
    ) -> SubGrid<'_, Self::Root> {
        self.visible_from_by(i, r, |s| blocks(s.at))
    }

    /// The field of view, when what blocks depends on the looker and the target.
    ///
    /// [`Grid::visible_from`] with a predicate over [`Sight`], and the reason this method exists:
    /// its predicate is asked once per candidate **per cell of the line to that candidate**, so it
    /// is the only one of the pair that can see which cell is being looked *at*. A height field
    /// cannot be expressed without that — a ridge hides a unit in the valley and not the one on the
    /// far peak, and those two questions differ only in the target.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// let board = FullGrid::square(5, 5, Adjacency::Eight);
    /// let eye = board.at(Sq::new(0, 2));
    /// let far = board.at(Sq::new(4, 2));
    ///
    /// // A blocker that reads the target: every cell on the way to `far` stops sight of `far`,
    /// // and stops nothing else. No predicate over the tested cell alone can say that.
    /// let seen = board.visible_from_by(eye, 4, |s| s.target == far);
    /// assert!(!seen.contains(Sq::new(4, 2)), "the one cell singled out");
    /// assert!(seen.contains(Sq::new(2, 2)), "though the line to it runs through the same cells");
    /// ```
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board, or `r` exceeds [`MAX_SIGHT`].
    #[must_use]
    fn visible_from_by(
        &self,
        i: Idx,
        r: u32,
        blocks: impl Fn(Sight) -> bool,
    ) -> SubGrid<'_, Self::Root> {
        assert!(
            r <= MAX_SIGHT,
            "a sight radius of {r} is beyond MAX_SIGHT ({MAX_SIGHT}); raycasting is O(r^3) and \
             this would take minutes. Use shadowcasting for a radius this large.",
        );

        let seen: Vec<Idx> = self
            .within(i, 0, r)
            .cells()
            .filter_map(|c| self.index_of(c))
            .filter(|&j| self.los_by(i, j, &blocks))
            .collect();
        self.subset(seen)
    }

    /// The cells visible from a coordinate, or `None` if the origin is not on this board.
    #[must_use]
    fn visible_from_cell(
        &self,
        c: Self::Cell,
        r: u32,
        blocks: impl Fn(Idx) -> bool,
    ) -> Option<SubGrid<'_, Self::Root>> {
        self.index_of(c).map(|i| self.visible_from(i, r, blocks))
    }

    // -- connectivity --------------------------------------------------------------------------

    /// Every cell **reachable from** `i` through cells that pass `passable`, `i` included.
    ///
    /// Did the generated map split into islands? Does that wall seal the room off? Those are
    /// unweighted questions, and this is the unweighted answer: a flood fill over
    /// [`neighbors`](Grid::neighbors), with no costs, no budget and no heap. It comes back as a
    /// board, because a room you found is a room you then want to path inside.
    ///
    /// # Out, not back
    ///
    /// It follows **forward** edges, so it means reachable-*from* `i`, exactly as
    /// [`reachable`](Grid::reachable) does. The graph is directed, and on a board with a one-way
    /// ledge on it "I can get there" and "we are in the same piece of map" are genuinely different
    /// claims. This is the first one. For who can get to *here*, see [`reaching`](Grid::reaching).
    ///
    /// An `i` that is not itself passable is in no component, and the answer is empty.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Sq};
    ///
    /// // A wall down the middle of a 5x3 room, with no gap in it.
    /// let g = FullGrid::square(5, 3, Adjacency::Four);
    /// let open = |i| g.coord(i).x != 2;
    ///
    /// let west = g.component(g.at(Sq::new(0, 0)), open);
    /// assert_eq!(west.len(), 6, "two columns, three rows — the wall seals it");
    /// assert!(!g.is_connected(open));
    /// ```
    ///
    /// # Panics
    ///
    /// If `i` is not a cell of this board.
    #[must_use]
    fn component(&self, i: Idx, passable: impl Fn(Idx) -> bool) -> SubGrid<'_, Self::Root> {
        let _ = slot(self.len(), self.tag(), i);
        if !passable(i) {
            return self.subset([]);
        }

        // A cell is marked before it is queued, so it is queued at most once and the frontier
        // cannot outgrow the board. Bounded by the board, like everything else here.
        let mut seen = vec![false; self.len()];
        seen[i.raw() as usize] = true;
        let mut frontier = vec![i];

        while let Some(at) = frontier.pop() {
            for (_, j) in self.neighbors(at) {
                if !seen[j.raw() as usize] && passable(j) {
                    seen[j.raw() as usize] = true;
                    frontier.push(j);
                }
            }
        }

        self.subset(self.indices().filter(|&j| seen[j.raw() as usize]))
    }

    /// The component containing a coordinate, or `None` if the coordinate is not on this board.
    #[must_use]
    fn component_from(
        &self,
        c: Self::Cell,
        passable: impl Fn(Idx) -> bool,
    ) -> Option<SubGrid<'_, Self::Root>> {
        self.index_of(c).map(|i| self.component(i, passable))
    }

    /// Whether the passable cells are one piece: every one of them reachable from the first.
    ///
    /// The island check. Read it precisely, because on a directed board the reading matters: this
    /// is one [`component`](Grid::component) taken from the **lowest-indexed** passable cell,
    /// compared against the passable count. So it asks whether everything can be reached *from*
    /// that cell, not whether every pair can reach each other.
    ///
    /// The difference is real. A board of one-way ledges running downhill is connected by this
    /// test when its top cell has the lowest index, and split when its bottom one does — because
    /// those are honestly different boards to walk. If you need mutual reachability, ask
    /// [`component`](Grid::component) from both ends.
    ///
    /// A board with no passable cells is connected, having nothing to be split into.
    #[must_use]
    fn is_connected(&self, passable: impl Fn(Idx) -> bool) -> bool {
        let Some(first) = self.indices().find(|&i| passable(i)) else {
            return true;
        };
        let total = self.indices().filter(|&i| passable(i)).count();
        self.component(first, &passable).len() == total
    }

    // -- paths ---------------------------------------------------------------------------------

    /// The cheapest path from `start` to `goal`, or `None` if there is no way through.
    ///
    /// A\*, with an admissible heuristic derived from the board's own metric and the movement's
    /// cheapest step. The path it finds is genuinely the cheapest, provided `min_step` is honest —
    /// which [`Movement::scan`] guarantees.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
    ///
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let walk = Movement::scan(&g, |_| Some(10));
    /// let a = g.at(Sq::new(0, 0));
    /// let b = g.at(Sq::new(3, 4));
    ///
    /// let p = g.path(a, b, &walk).unwrap();
    /// assert_eq!(p.len(), 7, "seven steps, four-way");
    /// assert_eq!(p.steps().first(), Some(&a), "and the start is included, so eight cells");
    /// ```
    ///
    /// # Panics
    ///
    /// If `start` or `goal` is not a cell of this board.
    fn path<F>(&self, start: Idx, goal: Idx, m: &Movement<F>) -> Option<Path>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        crate::search::find(self, start, goal, m)
    }

    /// The cheapest path between two coordinates, or `None` if either coordinate is off the board
    /// or no route exists.
    #[must_use]
    fn path_between<F>(&self, start: Self::Cell, goal: Self::Cell, m: &Movement<F>) -> Option<Path>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        let (Some(start), Some(goal)) = (self.index_of(start), self.index_of(goal)) else {
            return None;
        };
        self.path(start, goal, m)
    }

    /// Every cell reachable from `start` for no more than `budget`, and what reaching it costs.
    ///
    /// Cheapest first, and `start` itself comes back at cost 0. The search stops as soon as the
    /// frontier passes the budget — it does not explore the whole board and filter afterwards.
    ///
    /// A list of pairs rather than a board, because the **cost** is the answer — that is what a
    /// movement overlay shades and what an AI scores. When you want the region itself, promote it:
    /// `g.subset(moves.iter().map(|&(i, _)| i))`.
    ///
    /// This is *reach*, not *destinations*. A game where you may walk through an ally but not stop
    /// on one filters the result; the grid does not know what an ally is.
    ///
    /// ```
    /// use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
    ///
    /// let g = FullGrid::square(9, 9, Adjacency::Four);
    /// let walk = Movement::scan(&g, |_| Some(10));
    /// let centre = g.at(Sq::new(4, 4));
    ///
    /// // Three moves on open ground reaches a diamond of 25 cells, counting where you stand.
    /// assert_eq!(g.reachable(centre, 30, &walk).len(), 25);
    /// ```
    /// # Panics
    ///
    /// If `start` is not a cell of this board.
    fn reachable<F>(&self, start: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        crate::search::reachable(self, start, budget, m)
    }

    /// Every coordinate reachable from `start` for no more than `budget`, with its cost, or `None`
    /// if `start` is off the board.
    #[must_use]
    fn reachable_from<F>(
        &self,
        start: Self::Cell,
        budget: Cost,
        m: &Movement<F>,
    ) -> Option<Vec<(Self::Cell, Cost)>>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        let start = self.index_of(start)?;
        Some(
            self.reachable(start, budget, m)
                .into_iter()
                .map(|(i, cost)| (self.coord(i), cost))
                .collect(),
        )
    }

    /// The reachable cell that lands closest to `target`, and the path to it.
    ///
    /// What a pursuing unit wants: get as near as this turn's movement allows. If `target` is
    /// itself reachable this is simply the path to it.
    ///
    /// Ties break on distance, then cost, then index — a total order, so the answer is the same
    /// every run. A chasing enemy that dithered between two equally good cells would make a battle
    /// impossible to replay.
    ///
    /// # Panics
    ///
    /// If `start` or `target` is not a cell of this board.
    fn path_toward<F>(&self, start: Idx, target: Idx, budget: Cost, m: &Movement<F>) -> Option<Path>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        crate::search::toward(self, start, target, budget, m)
    }

    /// Every cell that can **reach** `goal` for no more than `budget`, and what it costs them.
    ///
    /// A threat map. [`Grid::reachable`] answers "where can I go"; this answers "who can get to
    /// *here*" — which is the question a tactics AI actually asks, and on a directed graph they are
    /// genuinely different questions. One backward Dijkstra, rather than one forward search per
    /// enemy on the board.
    ///
    /// The cost of arriving at `j` from `i` is `enter(Step { from: i, to: j, dir })` — the forward
    /// cost of that step, which is the right one, and which is only expressible because a [`Step`]
    /// carries `from` as well as `to`.
    ///
    /// ```
    /// use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Movement, Sq};
    ///
    /// // A one-way ledge: you may drop south off it, never climb north back up.
    /// let g = FullGrid::square(1, 4, Adjacency::Four);
    /// let m = Movement::scan(&g, |s| (s.dir == Dir8::S).then_some(10));
    ///
    /// let bottom = g.at(Sq::new(0, 3));
    ///
    /// // Everything above can reach the bottom...
    /// assert_eq!(g.reaching(bottom, 100, &m).len(), 4);
    /// // ...but from the bottom you can reach only yourself.
    /// assert_eq!(g.reachable(bottom, 100, &m).len(), 1);
    /// ```
    ///
    /// # Panics
    ///
    /// If `goal` is not a cell of this board.
    fn reaching<F>(&self, goal: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        crate::search::reaching(self, goal, budget, m)
    }

    /// Every coordinate that can reach `goal` for no more than `budget`, with its cost, or `None`
    /// if `goal` is off the board.
    #[must_use]
    fn reaching_cell<F>(
        &self,
        goal: Self::Cell,
        budget: Cost,
        m: &Movement<F>,
    ) -> Option<Vec<(Self::Cell, Cost)>>
    where
        F: Fn(Step<Self::Cell>) -> Option<Cost>,
    {
        let goal = self.index_of(goal)?;
        Some(
            self.reaching(goal, budget, m)
                .into_iter()
                .map(|(i, cost)| (self.coord(i), cost))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::coord::{Dir8, Hex, Idx, Metric, Sq};
    use crate::full::{Adjacency, FullGrid};
    use crate::grid::{Grid, Sight};
    use crate::path::Movement;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn a_ray_stops_at_the_edge() {
        let g = FullGrid::square(8, 8, Adjacency::Eight);
        let corner = g.at(Sq::new(0, 0));
        assert_eq!(g.ray(corner, Dir8::E).count(), 7);
        assert_eq!(g.ray(corner, Dir8::W).count(), 0);
    }

    #[test]
    fn a_ray_stops_at_a_hole_but_an_offset_leaps_it() {
        let g = FullGrid::square(5, 1, Adjacency::Four).filtered(|c| c.x != 2);
        let start = g.at(Sq::new(0, 0));

        // The ray walks x=1 and then dies at the gap: it cannot slide through.
        assert_eq!(g.ray(start, Dir8::E).count(), 1);

        // The offset hops clean over it. This is the difference that makes a jump possible.
        assert_eq!(g.offset(start, Sq::new(3, 0)), g.index_of(Sq::new(3, 0)));
    }

    #[test]
    fn within_measures_coordinates_so_a_hole_does_not_shorten_it() {
        let g = FullGrid::hexagon(2).filtered(|c| c != Hex::new(1, 0));
        let centre = g.at(Hex::new(0, 0));

        // (2,0) sits at distance 2 behind the hole at (1,0). It is still in range: an archer
        // shoots over a gap, and a jumping piece leaps one.
        let ring2 = g.ring(centre, 2);
        assert!(ring2.contains(Hex::new(2, 0)));
        assert!(!ring2.contains(Hex::new(0, 0)));
    }

    #[test]
    fn within_zero_includes_the_origin_and_within_one_does_not() {
        let g = FullGrid::square(5, 5, Adjacency::Four);
        let mid = g.at(Sq::new(2, 2));

        assert!(g.within(mid, 0, 1).contains(Sq::new(2, 2)));
        assert!(!g.within(mid, 1, 1).contains(Sq::new(2, 2)));

        // Manhattan range 1 is a plus sign: four cells.
        assert_eq!(g.within(mid, 1, 1).len(), 4);
    }

    #[test]
    fn a_run_walks_both_ways_and_reads_along_the_direction_it_was_asked_for() {
        let g = FullGrid::square(7, 1, Adjacency::Four);
        let at = |x| g.at(Sq::new(x, 0));
        let wall = |i| (1..=5).contains(&g.coord(i).x);

        let east = g.run(at(3), Dir8::E, wall);
        assert_eq!(east, vec![at(1), at(2), at(3), at(4), at(5)]);

        let west: Vec<_> = g.run(at(3), Dir8::W, wall).into_iter().rev().collect();
        assert_eq!(west, east, "the same line, read the other way");
    }

    #[test]
    fn a_run_always_holds_its_anchor_and_never_asks_about_it() {
        let g = FullGrid::square(5, 1, Adjacency::Four);
        let mid = g.at(Sq::new(2, 0));
        assert_eq!(g.run(mid, Dir8::E, |_| false), vec![mid]);
    }

    #[test]
    fn a_run_reaches_back_up_a_one_way_ledge() {
        // The reverse leg follows the in-edges, not the direction turned around. Here the north
        // step does not exist at all, and the line through the column is still whole.
        let g = ledges((0..4).map(|y| Sq::new(0, y)));
        let mid = g.at(Sq::new(0, 2));
        assert_eq!(g.run(mid, Dir8::S, |_| true).len(), 4);
    }

    #[test]
    fn a_wall_across_a_room_splits_it_into_two_components() {
        let g = FullGrid::square(5, 3, Adjacency::Four);
        let open = |i| g.coord(i).x != 2;

        let west = g.component(g.at(Sq::new(0, 0)), open);
        let east = g.component(g.at(Sq::new(4, 0)), open);

        assert_eq!(west.len(), 6);
        assert_eq!(east.len(), 6);
        assert!(west.cells().all(|c| !east.contains(c)), "disjoint");
        assert!(!g.is_connected(open));

        // One gap in the wall, and the two halves are one room again.
        let door = |i| g.coord(i) != Sq::new(2, 0) && g.coord(i) != Sq::new(2, 2);
        assert!(g.is_connected(door));
    }

    #[test]
    fn a_component_excludes_a_start_that_is_itself_impassable() {
        let g = FullGrid::square(3, 3, Adjacency::Four);
        let wall = g.at(Sq::new(1, 1));
        assert!(g.component(wall, |i| i != wall).is_empty());
    }

    /// A column of one-way ledges: every cell may drop south, none may climb north. `cells` fixes
    /// the indices, so handing it in reverse puts the bottom of the drop at index 0.
    fn ledges(cells: impl IntoIterator<Item = Sq>) -> FullGrid<Sq> {
        FullGrid::new(cells, &[Dir8::S], Metric::MANHATTAN)
    }

    #[test]
    fn a_component_follows_forward_edges_only() {
        // The directed case, and it is not a corner: a ledge you may drop off but not climb is an
        // ordinary tactics feature. From the top everything below is in reach; from the bottom
        // nothing is.
        let g = ledges((0..4).map(|y| Sq::new(0, y)));
        let top = g.at(Sq::new(0, 0));
        let bottom = g.at(Sq::new(0, 3));

        assert_eq!(g.component(top, |_| true).len(), 4);
        assert_eq!(g.component(bottom, |_| true).len(), 1);
    }

    #[test]
    fn is_connected_asks_from_the_lowest_index_and_the_answer_can_turn_on_it() {
        // The choice this method makes, pinned by a test rather than by prose. Both boards hold the
        // same four cells and the same four one-way drops. They differ only in which cell is
        // numbered 0 — and on a directed board that is a real difference, not an accident of
        // bookkeeping.
        assert!(
            ledges((0..4).map(|y| Sq::new(0, y))).is_connected(|_| true),
            "index 0 is the top of the drop, and everything is below it"
        );
        assert!(
            !ledges((0..4).rev().map(|y| Sq::new(0, y))).is_connected(|_| true),
            "index 0 is the bottom, and nothing can be reached from there"
        );
    }

    #[test]
    fn an_eight_way_grid_gives_an_archer_a_square_and_a_four_way_grid_a_diamond() {
        let mid = Sq::new(3, 3);

        let eight = FullGrid::square(7, 7, Adjacency::Eight);
        let i = eight.at(mid);
        assert_eq!(
            eight.within(i, 1, 2).len(),
            24,
            "a 5x5 square, less the centre"
        );

        let four = FullGrid::square(7, 7, Adjacency::Four);
        let j = four.at(mid);
        assert_eq!(four.within(j, 1, 2).len(), 12, "a diamond");
    }

    #[test]
    fn coordinate_facing_helpers_cover_the_common_queries() {
        let g = FullGrid::square(5, 5, Adjacency::Four);
        let movement = Movement::uniform(&g, 10);

        assert_eq!(g.step_from(Sq::new(1, 1), Dir8::E), Some(Sq::new(2, 1)));
        assert_eq!(g.step_from(Sq::new(-1, 1), Dir8::E), None);

        let path = g
            .path_between(Sq::new(0, 0), Sq::new(2, 0), &movement)
            .unwrap();
        assert_eq!(
            path.cells(&g).collect::<Vec<_>>(),
            vec![Sq::new(0, 0), Sq::new(1, 0), Sq::new(2, 0)]
        );

        assert_eq!(
            g.reachable_from(Sq::new(2, 2), 10, &movement)
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            g.reaching_cell(Sq::new(2, 2), 0, &movement).unwrap().len(),
            1
        );
        assert!(
            g.within_cell(Sq::new(2, 2), 0, 1)
                .unwrap()
                .contains(Sq::new(2, 2))
        );
        assert!(
            g.component_from(Sq::new(2, 2), |_| true)
                .unwrap()
                .contains(Sq::new(2, 2))
        );
        assert!(
            g.visible_from_cell(Sq::new(2, 2), 1, |_| false)
                .unwrap()
                .contains(Sq::new(2, 2))
        );
    }

    #[test]
    fn a_target_blind_gate_answers_exactly_as_the_plain_predicate_does() {
        // The wrapper's whole claim: `los` is `los_by` with the target thrown away. Checked over
        // every ordered pair on a board with a wall down the middle, so no single lucky line
        // carries it.
        let g = FullGrid::square(7, 7, Adjacency::Eight);
        let wall = |i: Idx| g.coord(i).x == 3 && g.coord(i).y != 3;

        for a in g.indices() {
            for b in g.indices() {
                assert_eq!(
                    g.los(a, b, wall),
                    g.los_by(a, b, |s| wall(s.at)),
                    "{:?} -> {:?}",
                    g.coord(a),
                    g.coord(b)
                );
            }
        }
    }

    #[test]
    fn a_gate_that_reads_the_target_says_what_no_bare_predicate_could() {
        // A one-way window: it stops sight INTO the east room and not out of it. The cell doing the
        // blocking is the same cell either way, so `Fn(Idx) -> bool` cannot express this at all.
        let g = FullGrid::square(7, 1, Adjacency::Eight);
        let window = g.at(Sq::new(3, 0));
        let west = g.at(Sq::new(0, 0));
        let east = g.at(Sq::new(6, 0));

        let one_way = |s: Sight| s.at == window && g.coord(s.target).x > 3;

        assert!(!g.los_by(west, east, one_way), "you cannot see in");
        assert!(g.los_by(east, west, one_way), "but you can see out");
    }

    #[test]
    fn a_field_of_view_gate_is_told_which_cell_is_being_looked_at() {
        // `visible_from` fixes the eye but not the target, which is the gap this pair closes.
        let g = FullGrid::square(5, 5, Adjacency::Eight);
        let eye = g.at(Sq::new(0, 2));
        let far = g.at(Sq::new(4, 2));

        let seen = g.visible_from_by(eye, 4, |s| s.target == far);
        assert!(!seen.contains(Sq::new(4, 2)), "singled out by target");
        assert!(
            seen.contains(Sq::new(3, 2)),
            "its neighbour is reached through the very same cells"
        );
    }

    #[test]
    fn the_eye_and_the_target_are_never_themselves_tested() {
        let g = FullGrid::square(5, 1, Adjacency::Eight);
        let (a, b) = (g.at(Sq::new(0, 0)), g.at(Sq::new(4, 0)));

        assert!(g.los_by(a, b, |s| {
            assert_ne!(s.at, s.eye, "the cell you stand in");
            assert_ne!(s.at, s.target, "the wall you are looking at");
            false
        }),);
    }
}
