//! Cell addresses, and the directions you can leave one by.
//!
//! A [`Coord`] is a cell address *and* a vector — it adds and subtracts. That is what lets
//! [`Grid::offset`](crate::Grid::offset) express a knight's leap or a checkers jump without the
//! grid knowing what a knight is.
//!
//! Two implementations ship: [`Sq`] (square, eight directions) and [`Hex`] (axial, six). A game
//! with an exotic board implements the trait itself; see `tests/chess3d.rs`, which builds a
//! three-layer chess board in a few dozen lines without touching this crate.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Sub};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Squeeze a widened distance back into the `u32` a metric returns.
///
/// A distance only reaches the clamp between coordinates billions apart, which no board has. It
/// exists so the arithmetic above can be done in `i64` — where it cannot wrap — without the cast
/// home reintroducing the very wrap we widened to avoid.
fn clamp_u32(d: i64) -> u32 {
    d.clamp(0, i64::from(u32::MAX)) as u32
}

/// How a grid measures distance — and, if it can, how to enumerate a neighbourhood without
/// walking the whole board.
///
/// `distance` is the metric proper. The other two exist so [`Grid::within`](crate::Grid::within)
/// can answer "every cell inside radius `r`" in time proportional to **the radius**, rather than to
/// the board. On a 200 × 200 map that is the difference between 0.2µs and 88µs, on the single query
/// a tactics game makes most: attack range, vision, blast radius, every unit, every turn.
///
/// # Why `count` exists, and why it must not allocate
///
/// The obvious implementation — build the offset list, then use it — is a memory bomb. The list is
/// sized by the *caller's* radius, and `within(i, 0, u32::MAX)` would ask for about 7×10¹⁹ entries.
/// So `count` reports how big the list *would* be, without building it, and the grid only builds it
/// when it is smaller than the board. Otherwise it falls back to scanning, which is bounded by the
/// board and is the code that was always there.
///
/// Both branches give the same answer. The choice is only ever about which is cheaper.
///
/// # The parts arrive together
///
/// The fields are private, and there are exactly two ways in: [`Metric::scanning`], which asks for
/// `distance` alone, and [`Metric::tabulated`], which asks for all three at once. That is not
/// bookkeeping — the invariants below relate the three, so a metric assembled a field at a time is
/// a metric that can be half right. A wrong `count` does not fail loudly; it silently picks the
/// slower branch, or asks for an offset table sized by a radius.
///
/// # Invariants
///
/// These are checked where they can be and stated where they cannot. Read them on the constructor
/// you use.
///
/// - `distance` must never **over**estimate the number of steps between two cells, or A\* stops
///   being admissible and quietly returns non-optimal paths. [`FullGrid::new`](crate::FullGrid::new)
///   does check the other side of this — that no single step covers more than one unit — and panics.
/// - `deltas(r)` must agree with `distance`: it must yield exactly the offsets `d` for which
///   `distance(c, c + d) <= r`, each paired with that distance. This means the metric must be
///   **translation invariant** — the same everywhere on the board. If yours is not, use
///   [`Metric::scanning`], which makes no such assumption.
/// - `count(r)` must equal `deltas(r).len()`, and must saturate rather than overflow.
#[derive(Clone, Copy)]
pub struct Metric<C: Coord> {
    /// The number of steps between two cells. Never an overestimate.
    distance: fn(C, C) -> u32,
    /// How many offsets lie within `r`. **Must not allocate** — that is the whole point of it.
    /// Saturate at [`u64::MAX`], which reads as "more than any board" and forces the scan.
    count: fn(u32) -> u64,
    /// Every offset within `r` of the origin, each with its distance. In a fixed order.
    ///
    /// Only ever called when `count(r)` is no larger than the board, so it cannot run away.
    deltas: fn(u32) -> Vec<(C, u32)>,

    /// The cell `t/n` of the way along the straight line from `a` to `b`, rounded to the lattice.
    /// `None` if this lattice has no notion of a straight line.
    ///
    /// This drives [`Grid::los`](crate::Grid::los). Note the shape: it returns **one cell**, and the
    /// grid drives the loop. The obvious alternative — `fn(a, b) -> Vec<C>` — hands the allocation
    /// to *your* code, sized by *your* coordinates, where the crate can no longer bound it. Two
    /// cells a billion apart would ask for a billion-element vector to describe a line touching two
    /// cells. Returning one cell at a time makes that impossible rather than merely discouraged.
    lerp: Option<Lerp<C>>,
}

/// The cell `t/n` of the way from `a` to `b`, rounded to the lattice. See [`Metric::lerp`].
pub type Lerp<C> = fn(a: C, b: C, t: u32, n: u32) -> C;

impl<C: Coord> fmt::Debug for Metric<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metric").finish_non_exhaustive()
    }
}

impl<C: Coord> Metric<C> {
    /// A metric with no offset table: range queries scan the board.
    ///
    /// The honest choice for a coordinate of your own. It is O(cells) rather than O(radius), which
    /// on a small board is nothing at all — and it holds for metrics that are *not* translation
    /// invariant, where an offset table would silently give wrong answers.
    ///
    /// It is also the right answer for a board with many dimensions. Offsets grow as `(2r+1)ᵈ`, so
    /// on a three-dimensional board `r = 1000` is eight billion offsets — vastly worse than
    /// scanning. `tests/chess3d.rs` uses this.
    ///
    /// `distance` must not **over**estimate the number of steps between two cells. That is the one
    /// promise this constructor cannot check, and breaking it makes A\* return non-optimal paths
    /// without saying so. When in doubt, underestimate: a metric that always returns 0 is always
    /// admissible, and turns A\* into Dijkstra — slower, still correct.
    #[must_use]
    pub const fn scanning(distance: fn(C, C) -> u32) -> Self {
        Self {
            distance,
            count: |_| u64::MAX,
            deltas: |_| Vec::new(),
            lerp: None,
        }
    }

    /// A metric with an offset table, so a range query is priced by the radius, not by the board.
    ///
    /// The three arguments are one thing in three parts, which is why they arrive together. See the
    /// invariants on [`Metric`]: `deltas` must yield exactly the offsets within `r` under
    /// `distance`, and `count` must report how many of those there would be **without building
    /// them**. A `count` that allocates defeats the whole mechanism, and one that disagrees with
    /// `deltas` turns a range query into either a needless board scan or a memory bomb.
    ///
    /// Only worth it for a translation-invariant metric on a lattice of one or two dimensions.
    /// Anything else wants [`Metric::scanning`].
    #[must_use]
    pub const fn tabulated(
        distance: fn(C, C) -> u32,
        count: fn(u32) -> u64,
        deltas: fn(u32) -> Vec<(C, u32)>,
    ) -> Self {
        Self {
            distance,
            count,
            deltas,
            lerp: None,
        }
    }

    /// Give this metric a straight line, which is what [`Grid::los`](crate::Grid::los) needs.
    ///
    /// Without one, sight falls back to nothing: a lattice that cannot say which cells lie between
    /// two others cannot say what blocks a view. Not every lattice has a sensible answer — a
    /// three-layer chess board does not — and saying so is better than guessing.
    #[must_use]
    pub const fn with_lerp(mut self, lerp: Lerp<C>) -> Self {
        self.lerp = Some(lerp);
        self
    }

    /// The number of steps between two cells, as this metric counts them.
    #[must_use]
    pub fn distance(&self, a: C, b: C) -> u32 {
        (self.distance)(a, b)
    }

    /// How many offsets lie within `r`, without building them. See [`Metric::tabulated`].
    #[must_use]
    pub fn count(&self, r: u32) -> u64 {
        (self.count)(r)
    }

    /// Every offset within `r` of the origin, each with its distance.
    ///
    /// Ask [`count`](Metric::count) first. This is only bounded when the answer is.
    #[must_use]
    pub fn deltas(&self, r: u32) -> Vec<(C, u32)> {
        (self.deltas)(r)
    }

    /// The cell `t/n` of the way from `a` to `b`, or `None` if this lattice has no straight line.
    #[must_use]
    pub fn lerp(&self, a: C, b: C, t: u32, n: u32) -> Option<C> {
        self.lerp.map(|f| f(a, b, t, n))
    }

    /// Whether this lattice has a notion of a straight line at all.
    ///
    /// [`Grid::los`](crate::Grid::los) and [`Grid::line`](crate::Grid::line) need one, and answer
    /// with nothing when there is none. A three-layer chess board is the honest case: no two cells
    /// on different layers have cells "between" them in any sense worth blocking sight with.
    #[must_use]
    pub fn has_lerp(&self) -> bool {
        self.lerp.is_some()
    }
}

/// Bring a `f64` home to an `i32`, clamped so an absurd value cannot wrap on the way.
///
/// The one door from float space back into the lattice, and both users of it — a line's endpoints
/// and a pixel under the mouse — are handing over a number this crate did not choose. NaN is not a
/// coordinate, and `NaN as i32` is `0`, which would silently name the origin cell: the mouse lands
/// on your capital. So NaN is sent to the far edge instead, where no board can hold it and
/// [`Grid::index_of`](crate::Grid::index_of) says `None`.
pub(crate) fn clamp_i32(v: f64) -> i32 {
    if v.is_nan() {
        LATTICE_LIMIT
    } else {
        v.clamp(f64::from(-LATTICE_LIMIT), f64::from(LATTICE_LIMIT)) as i32
    }
}

/// How far from the origin a rounded coordinate may land: `2³⁰ - 1`.
///
/// Not `i32::MAX`, and the difference is a bug. A [`Hex`] is really three cube axes summing to zero,
/// and the third is *derived*: [`Hex::s`] is `-q - r`. Clamp `q` and `r` to `i32::MAX` independently
/// and you can mint a "hex" whose `s` needs 33 bits — so `s()` wraps in release and the cell is not
/// on the lattice at all. Half the range, and `|s| ≤ 2³¹ - 2` holds by construction.
///
/// It never binds on a real board. The largest hexagon [`MAX_CELLS`](crate::MAX_CELLS) permits has a
/// radius of 2364; this is four hundred thousand times further out.
const LATTICE_LIMIT: i32 = (1 << 30) - 1;

/// Round one axis of a lerp, clamped so an absurd coordinate cannot wrap on the way back to `i32`.
fn lerp_axis(a: i32, b: i32, t: f64) -> i32 {
    let (a, b) = (f64::from(a), f64::from(b));
    clamp_i32((a + (b - a) * t).round())
}

/// `f64`, never `f32`. An `f32` has a 24-bit mantissa, so it cannot represent integers above
/// 16,777,216 exactly — a line drawn between coordinates in that range would snap to even cells,
/// skipping some and repeating others, and sight would pass clean through a wall. An `f64`'s 53-bit
/// mantissa represents every `i32` and every difference of two exactly.
fn sq_lerp(a: Sq, b: Sq, t: u32, n: u32) -> Sq {
    let f = f64::from(t) / f64::from(n);
    Sq::new(lerp_axis(a.x, b.x, f), lerp_axis(a.y, b.y, f))
}

/// The cell nearest a *fractional* hex: round all three cube axes, then repair whichever drifted
/// furthest so they still sum to zero. Rounding the axes independently is what would break the
/// `q + r + s = 0` invariant, so exactly one of them is recomputed from the other two — the one
/// that was rounded least honestly.
///
/// Shared by [`hex_lerp`] and by [`HexLayout::hex_at`](crate::HexLayout::hex_at): drawing a line
/// and picking the hex under the mouse are the same question — *which cell is this fractional
/// point in?* — so they must not be two pieces of code that can disagree.
///
/// Clamped, not wrapped: a fractional coordinate arriving from pixel space is a number this crate
/// did not choose, and `as i32` on a value past `i32::MAX` must not come out the far side.
pub(crate) fn hex_round(q: f64, r: f64) -> Hex {
    let s = -q - r;

    let (mut rq, mut rr, rs) = (q.round(), r.round(), s.round());
    let (dq, dr, ds) = ((rq - q).abs(), (rr - r).abs(), (rs - s).abs());
    if dq > dr && dq > ds {
        rq = -rr - rs;
    } else if dr > ds {
        rr = -rq - rs;
    }

    Hex::new(clamp_i32(rq), clamp_i32(rr))
}

/// A straight line on a hex lattice, one cell at a time.
///
/// The epsilon nudge breaks ties that would otherwise land exactly on a cell boundary — the
/// standard trick, and without it a line can flicker between two equally-close cells. Nudging `q`
/// and `r` by `+1e-6` leaves `s = -q - r` short by `2e-6`, which is precisely the skew wanted: no
/// axis stays on a boundary, and the three still sum to zero.
fn hex_lerp(a: Hex, b: Hex, t: u32, n: u32) -> Hex {
    let f = f64::from(t) / f64::from(n);
    let lerp = |x: i64, y: i64| x as f64 + (y as f64 - x as f64) * f;

    let (aq, ar) = (i64::from(a.q), i64::from(a.r));
    let (bq, br) = (i64::from(b.q), i64::from(b.r));

    hex_round(lerp(aq, bq) + 1e-6, lerp(ar, br) + 1e-6)
}

/// `kr² + kr + 1`, saturating — the centred polygonal numbers. `k = 2` is the count of a
/// Manhattan diamond, `k = 3` the count of a hexagon (the centred hexagonal numbers).
fn count_centered(k: u64, r: u32) -> u64 {
    let r = u64::from(r);
    k.saturating_mul(r)
        .saturating_mul(r)
        .saturating_add(k.saturating_mul(r))
        .saturating_add(1)
}

/// `(2r + 1)²`, saturating. The count of a Chebyshev square.
fn count_chebyshev(r: u32) -> u64 {
    let side = 2 * u64::from(r) + 1;
    side.saturating_mul(side)
}

/// Every square offset within `r`, paired with its distance under `metric`.
///
/// The bounding box is walked in `i64` and the radius clamped, so no arithmetic here can wrap even
/// if a caller asks for a preposterous radius — though the grid will have chosen to scan long before
/// that.
fn square_deltas(r: u32, metric: fn(Sq, Sq) -> u32) -> Vec<(Sq, u32)> {
    let reach = i64::from(r).min(i64::from(i32::MAX));
    let mut out = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let d = Sq::new(dx as i32, dy as i32);
            let dist = metric(Sq::new(0, 0), d);
            if dist <= r {
                out.push((d, dist));
            }
        }
    }
    out
}

impl Metric<Sq> {
    /// `|dx| + |dy|`. Four-way movement: range 1 is a plus sign, range 2 a diamond.
    pub const MANHATTAN: Self = Self::tabulated(
        |a, b| a.manhattan(b),
        |r| count_centered(2, r),
        |r| square_deltas(r, |a, b| a.manhattan(b)),
    )
    .with_lerp(sq_lerp);

    /// `max(|dx|, |dy|)`. Eight-way movement: range 1 is the eight surrounding cells.
    pub const CHEBYSHEV: Self = Self::tabulated(
        |a, b| a.chebyshev(b),
        count_chebyshev,
        |r| square_deltas(r, |a, b| a.chebyshev(b)),
    )
    .with_lerp(sq_lerp);
}

impl Metric<Hex> {
    /// Cube distance on a hex lattice.
    pub const HEX: Self = Self::tabulated(
        |a, b| a.distance(b),
        |r| count_centered(3, r),
        |r| {
            let reach = i64::from(r).min(i64::from(i32::MAX));
            let mut out = Vec::new();
            for dq in -reach..=reach {
                for dr in (-reach).max(-dq - reach)..=reach.min(-dq + reach) {
                    let d = Hex::new(dq as i32, dr as i32);
                    let dist = Hex::new(0, 0).distance(d);
                    if dist <= r {
                        out.push((d, dist));
                    }
                }
            }
            out
        },
    )
    .with_lerp(hex_lerp);
}

/// Which board's numbering an [`Idx`] belongs to.
///
/// A tag names a *numbering*, not an object. Two boards that number the same cells in the same
/// order share one, and their indices are interchangeable — which is the property
/// [`FullGrid::new`](crate::FullGrid::new) promises and `tests/save.rs` rests on. A tag derived
/// from a counter would break that, so this is derived from the cells.
///
/// In release builds this is a zero-sized type: every check below compiles to nothing, and an
/// [`Idx`] is a bare `u32` again.
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tag(u32);

/// Which board's numbering an [`Idx`] belongs to. Zero-sized in release; see the debug definition.
#[cfg(not(debug_assertions))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tag;

impl Tag {
    /// Derive a tag from a board's cells, in index order.
    ///
    /// The iterator is **never consumed in release**, so a caller may hand over one that would be
    /// expensive to walk — [`RectGrid`](crate::RectGrid) generates its cells for exactly this and
    /// pays nothing for them in a shipped build.
    #[cfg(debug_assertions)]
    pub fn of<H: Hash>(items: impl IntoIterator<Item = H>) -> Self {
        use std::hash::{DefaultHasher, Hasher};

        let mut h = DefaultHasher::new();
        let mut n: u64 = 0;
        for item in items {
            item.hash(&mut h);
            n += 1;
        }
        // Length is mixed in last so that a prefix of another board's cells cannot collide with it.
        h.write_u64(n);
        // Forced odd, which keeps zero free to mean [`Tag::ANY`].
        #[allow(clippy::cast_possible_truncation)]
        Self(h.finish() as u32 | 1)
    }

    /// Derive a tag from a board's cells, in index order. Ignores its argument in release.
    #[cfg(not(debug_assertions))]
    pub fn of<H: Hash>(items: impl IntoIterator<Item = H>) -> Self {
        let _ = items;
        Self
    }

    /// Whether an index carrying `self` may be handed to a board carrying `other`.
    ///
    /// Equal tags, or [`Tag::ANY`] on either side. One definition serves both profiles: in release
    /// a `Tag` is zero-sized, so every arm is trivially true — and every caller is inside a
    /// `debug_assert`, which is gone by then anyway.
    pub(crate) fn agrees(self, other: Self) -> bool {
        self == other || self == Self::ANY || other == Self::ANY
    }
}

impl Tag {
    /// A tag that matches every board: what an index carries when nothing named its grid.
    ///
    /// One thing mints these — [`CellMap::iter`](crate::CellMap::iter) on a map that came back from
    /// serde, which has no grid to name. Dropping the check there is deliberate: what makes such a
    /// map line up is the cells saved beside it, and those are already the rule. See `tests/save.rs`.
    pub(crate) const ANY: Self = Self::any();

    #[cfg(debug_assertions)]
    const fn any() -> Self {
        Self(0)
    }

    #[cfg(not(debug_assertions))]
    const fn any() -> Self {
        Self
    }
}

/// A cell's dense index within one [`Grid`](crate::Grid).
///
/// Indices are assigned at construction and are stable for that grid's lifetime — but they mean
/// nothing to any *other* grid. **Serialize coordinates, never indices.**
///
/// # It carries the board it came from
///
/// In a debug build an `Idx` also holds a [`Tag`], and every method that takes one checks it. That
/// turns the crate's sharpest failure — an index from one board silently addressing a *different
/// cell* on another — into a panic that names the mistake. The check finds the case a bounds check
/// never could: two boards of the same size, where every index is in range for both.
///
/// In release the tag is zero-sized and the checks vanish, so this is a bare `u32` in a shipped
/// build. Equality, ordering, and hashing compare **the index alone** in both profiles, so no
/// behaviour depends on which one you built.
#[derive(Clone, Copy)]
pub struct Idx {
    i: u32,
    tag: Tag,
}

impl Idx {
    /// Mint an index for a board. Crate-internal: only a board may number its own cells.
    pub(crate) const fn new(tag: Tag, i: u32) -> Self {
        Self { i, tag }
    }

    /// The bare number, for keying a structure of your own.
    ///
    /// Reading an index out is safe; there is deliberately no way back in. Only a board mints an
    /// [`Idx`], so a number you took from one cannot be handed to a different board as though it
    /// belonged there — which is the whole mistake this type exists to stop.
    ///
    /// Reach for [`CellMap`](crate::CellMap) first when what you want is one value per cell: it is
    /// sized from the board, subscripted by an `Idx` directly, and it carries the same check.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.i
    }

    /// The bare number, for indexing the crate's own tables.
    pub(crate) const fn raw(self) -> u32 {
        self.i
    }

    /// The board this index was issued by.
    pub(crate) const fn tag(self) -> Tag {
        self.tag
    }
}

// Hand-written, and on `i` alone. Deriving these would compare the tag, and a value that compares
// differently in debug than in release is a far worse bug than the one the tag exists to catch.
impl PartialEq for Idx {
    fn eq(&self, o: &Self) -> bool {
        self.i == o.i
    }
}

impl Eq for Idx {}

impl PartialOrd for Idx {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Idx {
    fn cmp(&self, o: &Self) -> Ordering {
        self.i.cmp(&o.i)
    }
}

impl Hash for Idx {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.i.hash(h);
    }
}

// Transparent on purpose: an index reads as a number in a panic message and in a failed assertion.
impl fmt::Debug for Idx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.i)
    }
}

impl fmt::Display for Idx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.i)
    }
}

/// A cell address, and the directions leading out of it.
///
/// Implement this to put a board of your own shape on the crate's pathfinding and geometry.
/// The requirements are ordinary: a coordinate must be cheap to copy, usable as a hash key,
/// totally ordered (this is what makes tie-breaks deterministic), and addable as a vector.
pub trait Coord:
    Copy + Eq + Ord + Hash + fmt::Debug + Add<Output = Self> + Sub<Output = Self>
{
    /// The directions of travel out of a cell. Eight for a square, six for a hex.
    ///
    /// This rides along in [`Step`](crate::Step), which is what lets a cost function say
    /// "entering *this* cell heading north is expensive" — a river, a conveyor, a one-way ledge.
    type Dir: Copy + Eq + Hash + fmt::Debug + 'static;

    /// Every direction, in a fixed order.
    ///
    /// The order is load-bearing: it fixes the layout of the grid's step table, and therefore the
    /// order every downstream iteration and heap tie-break resolves in. Determinism starts here.
    const DIRS: &'static [Self::Dir];

    /// The cell one step from `self` in direction `d`.
    ///
    /// Pure arithmetic — it does not know whether the result is on any board. The grid decides
    /// that when it builds its step table.
    #[must_use]
    fn step(self, d: Self::Dir) -> Self;
}

// ---------------------------------------------------------------------------------------------
// Square
// ---------------------------------------------------------------------------------------------

/// A square-grid cell. `y` grows downward, as screen coordinates do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Sq {
    /// Column, growing east.
    pub x: i32,
    /// Row, growing south.
    pub y: i32,
}

impl Sq {
    /// A square cell at `(x, y)`.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// `|dx| + |dy|`. Range 1 is a plus sign, range 2 a diamond. Pairs with four-way movement.
    ///
    /// Computed in `i64` and clamped. Two `i32`s cannot overflow an `i64`, so the answer is exact
    /// for every pair of coordinates — including the extremes, where the obvious `self.x - o.x`
    /// wraps and reports two cells four billion apart as *adjacent*.
    ///
    /// ```
    /// use spacewalk::Sq;
    ///
    /// // A diagonal neighbour is two away, not one. Pair this metric with four-way movement.
    /// assert_eq!(Sq::new(3, 3).manhattan(Sq::new(4, 4)), 2);
    /// ```
    #[must_use]
    pub fn manhattan(self, o: Self) -> u32 {
        let (dx, dy) = (
            i64::from(self.x) - i64::from(o.x),
            i64::from(self.y) - i64::from(o.y),
        );
        clamp_u32(dx.abs() + dy.abs())
    }

    /// `max(|dx|, |dy|)`. Range 1 is the eight surrounding cells. Pairs with eight-way movement.
    ///
    /// Computed in `i64`; see [`Sq::manhattan`].
    #[must_use]
    pub fn chebyshev(self, o: Self) -> u32 {
        let (dx, dy) = (
            i64::from(self.x) - i64::from(o.x),
            i64::from(self.y) - i64::from(o.y),
        );
        clamp_u32(dx.abs().max(dy.abs()))
    }
}

/// The eight directions out of a square cell, clockwise from north.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Dir8 {
    /// North: `y - 1`.
    N,
    /// North-east.
    Ne,
    /// East: `x + 1`.
    E,
    /// South-east.
    Se,
    /// South: `y + 1`.
    S,
    /// South-west.
    Sw,
    /// West: `x - 1`.
    W,
    /// North-west.
    Nw,
}

impl Dir8 {
    /// All eight, clockwise from north.
    pub const ALL: [Dir8; 8] = [
        Dir8::N,
        Dir8::Ne,
        Dir8::E,
        Dir8::Se,
        Dir8::S,
        Dir8::Sw,
        Dir8::W,
        Dir8::Nw,
    ];

    /// The four orthogonals, clockwise from north.
    pub const ORTHO: [Dir8; 4] = [Dir8::N, Dir8::E, Dir8::S, Dir8::W];

    /// The four diagonals, clockwise from north-east.
    pub const DIAG: [Dir8; 4] = [Dir8::Ne, Dir8::Se, Dir8::Sw, Dir8::Nw];

    /// Whether this is a diagonal. A diagonal step covers √2 cells, not 1 — if your costs are
    /// scaled so an orthogonal step is 10, a diagonal one should be about 14.
    #[must_use]
    pub const fn is_diagonal(self) -> bool {
        matches!(self, Dir8::Ne | Dir8::Se | Dir8::Sw | Dir8::Nw)
    }

    /// The two orthogonals a diagonal squeezes between; `None` for an orthogonal.
    ///
    /// Moving from `A` to `D`, the flanks are `B` and `C`:
    ///
    /// ```text
    ///   [B][D]
    ///   [A][C]
    /// ```
    ///
    /// Corner rules are written in terms of these — see
    /// [`square::corner_gate`](crate::square::corner_gate).
    #[must_use]
    pub const fn flanks(self) -> Option<(Dir8, Dir8)> {
        match self {
            Dir8::Ne => Some((Dir8::N, Dir8::E)),
            Dir8::Se => Some((Dir8::S, Dir8::E)),
            Dir8::Sw => Some((Dir8::S, Dir8::W)),
            Dir8::Nw => Some((Dir8::N, Dir8::W)),
            _ => None,
        }
    }

    /// The direction facing the other way.
    #[must_use]
    pub const fn opposite(self) -> Dir8 {
        // A half turn: `ALL` lists the directions in declaration order, four apart from a U-turn.
        Dir8::ALL[(self as usize + 4) % 8]
    }
}

impl Coord for Sq {
    type Dir = Dir8;
    const DIRS: &'static [Dir8] = &Dir8::ALL;

    fn step(self, d: Dir8) -> Self {
        let (dx, dy) = match d {
            Dir8::N => (0, -1),
            Dir8::Ne => (1, -1),
            Dir8::E => (1, 0),
            Dir8::Se => (1, 1),
            Dir8::S => (0, 1),
            Dir8::Sw => (-1, 1),
            Dir8::W => (-1, 0),
            Dir8::Nw => (-1, -1),
        };
        Sq::new(self.x.saturating_add(dx), self.y.saturating_add(dy))
    }
}

// ---------------------------------------------------------------------------------------------
// Hex
// ---------------------------------------------------------------------------------------------

/// An axial hex cell `(q, r)`, with the third cube axis implied: `s = -q - r`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Hex {
    /// The first axial coordinate, growing east.
    pub q: i32,
    /// The second axial coordinate, growing north-east.
    pub r: i32,
}

impl Hex {
    /// A hex cell at axial `(q, r)`.
    #[must_use]
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// The third cube axis, always `-q - r`.
    ///
    /// At the extreme edge of the `i32` lattice that negation can overflow — a panic in debug, a
    /// wrap in release, like any Rust arithmetic. Every cell a real board holds is orders of
    /// magnitude inside the edge, and the grid's own math never calls this near it.
    #[must_use]
    pub const fn s(self) -> i32 {
        -self.q - self.r
    }

    /// Cube distance: the number of steps between two cells.
    ///
    /// Computed in `i64` and clamped. This one had three separate ways to overflow: the `i32`
    /// subtraction, the negation inside [`Hex::s`], and the sum of three `unsigned_abs` values,
    /// which can pass `u32::MAX` before the halving. In `i64` none of them can.
    #[must_use]
    pub fn distance(self, o: Self) -> u32 {
        let (dq, dr) = (
            i64::from(self.q) - i64::from(o.q),
            i64::from(self.r) - i64::from(o.r),
        );
        let ds = -dq - dr;
        clamp_u32((dq.abs() + dr.abs() + ds.abs()) / 2)
    }
}

/// The six directions out of a hex cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Dir6 {
    /// East: `q + 1`.
    E,
    /// North-east: `r + 1`.
    Ne,
    /// North-west.
    Nw,
    /// West: `q - 1`.
    W,
    /// South-west: `r - 1`.
    Sw,
    /// South-east.
    Se,
}

impl Dir6 {
    /// All six, anticlockwise from east.
    pub const ALL: [Dir6; 6] = [Dir6::E, Dir6::Ne, Dir6::Nw, Dir6::W, Dir6::Sw, Dir6::Se];

    /// The direction facing the other way.
    #[must_use]
    pub const fn opposite(self) -> Dir6 {
        // A half turn: `ALL` lists the directions in declaration order, three apart from a U-turn.
        Dir6::ALL[(self as usize + 3) % 6]
    }
}

impl Coord for Hex {
    type Dir = Dir6;
    const DIRS: &'static [Dir6] = &Dir6::ALL;

    fn step(self, d: Dir6) -> Self {
        let (dq, dr) = match d {
            Dir6::E => (1, 0),
            Dir6::Ne => (0, 1),
            Dir6::Nw => (-1, 1),
            Dir6::W => (-1, 0),
            Dir6::Sw => (0, -1),
            Dir6::Se => (1, -1),
        };
        Hex::new(self.q.saturating_add(dq), self.r.saturating_add(dr))
    }
}

// ---------------------------------------------------------------------------------------------
// Operators and formatting
// ---------------------------------------------------------------------------------------------

/// Vector addition and subtraction, **saturating** at the ends of `i32`.
///
/// Saturating rather than wrapping, and it matters. A cell at `i32::MAX` stepping east must not
/// come out at `i32::MIN` — because `i32::MIN` may well be *another real cell on the board*, and the
/// grid would then forge an edge between two cells four billion apart and let pieces walk it. That
/// is not a hypothetical: it is what plain `+` does in release.
///
/// Saturating instead makes such a step land back on the cell it came from, and
/// [`Grid::new`](crate::Grid::new) drops steps that go nowhere. So at the very edge of the
/// coordinate space the world simply ends, which is the answer you wanted anyway. Everywhere a real
/// board lives — anywhere inside ±2 billion — this is ordinary exact arithmetic.
macro_rules! vector_ops {
    ($t:ty, $($f:ident),+) => {
        impl Add for $t {
            type Output = Self;
            fn add(self, o: Self) -> Self { Self { $($f: self.$f.saturating_add(o.$f)),+ } }
        }
        impl Sub for $t {
            type Output = Self;
            fn sub(self, o: Self) -> Self { Self { $($f: self.$f.saturating_sub(o.$f)),+ } }
        }
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "({})", [$(self.$f.to_string()),+].join(", "))
            }
        }
    };
}

vector_ops!(Sq, x, y);
vector_ops!(Hex, q, r);

impl From<(i32, i32)> for Sq {
    fn from((x, y): (i32, i32)) -> Self {
        Sq::new(x, y)
    }
}

impl From<(i32, i32)> for Hex {
    fn from((q, r): (i32, i32)) -> Self {
        Hex::new(q, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manhattan_makes_a_diagonal_neighbour_two_away() {
        // The source of the classic bug: under four-way movement a diagonally adjacent enemy is
        // TWO steps away, so a melee unit standing next to it cannot reach it. That is correct,
        // and it is why the metric must agree with the adjacency.
        assert_eq!(Sq::new(0, 0).manhattan(Sq::new(1, 1)), 2);
        assert_eq!(Sq::new(0, 0).manhattan(Sq::new(3, 4)), 7);
    }

    #[test]
    fn chebyshev_makes_a_diagonal_neighbour_one_away() {
        assert_eq!(Sq::new(0, 0).chebyshev(Sq::new(1, 1)), 1);
        assert_eq!(Sq::new(0, 0).chebyshev(Sq::new(3, 4)), 4);
    }

    #[test]
    fn every_square_step_is_one_chebyshev_away() {
        // The A* heuristic leans on this: one step never changes the metric by more than one.
        for d in Dir8::ALL {
            assert_eq!(Sq::default().chebyshev(Sq::default().step(d)), 1, "{d:?}");
        }
    }

    #[test]
    fn every_hex_step_is_one_away() {
        for d in Dir6::ALL {
            assert_eq!(Hex::default().distance(Hex::default().step(d)), 1, "{d:?}");
        }
    }

    #[test]
    fn hex_cube_axes_sum_to_zero() {
        let h = Hex::new(3, -5);
        assert_eq!(h.q + h.r + h.s(), 0);
    }

    #[test]
    fn hex_distance_is_symmetric_and_additive_along_a_line() {
        let a = Hex::new(0, 0);
        let b = a.step(Dir6::E).step(Dir6::E).step(Dir6::E);
        assert_eq!(a.distance(b), 3);
        assert_eq!(b.distance(a), 3);
    }

    #[test]
    fn opposites_round_trip() {
        for d in Dir8::ALL {
            assert_eq!(Sq::default().step(d).step(d.opposite()), Sq::default());
        }
        for d in Dir6::ALL {
            assert_eq!(Hex::default().step(d).step(d.opposite()), Hex::default());
        }
    }

    #[test]
    fn flanks_are_the_two_orthogonals_a_diagonal_squeezes_between() {
        assert_eq!(Dir8::Ne.flanks(), Some((Dir8::N, Dir8::E)));
        assert_eq!(Dir8::N.flanks(), None);

        // A flank shares a cell with the diagonal it guards.
        for d in Dir8::DIAG {
            let (a, b) = d.flanks().unwrap();
            let corner = Sq::default().step(d);
            assert_eq!(corner.manhattan(Sq::default().step(a)), 1);
            assert_eq!(corner.manhattan(Sq::default().step(b)), 1);
        }
    }

    #[test]
    fn coords_are_vectors() {
        assert_eq!(Sq::new(1, 2) + Sq::new(3, 4), Sq::new(4, 6));
        assert_eq!(Sq::new(1, 2) - Sq::new(3, 4), Sq::new(-2, -2));
        assert_eq!(Hex::new(1, 2) + Hex::new(3, 4), Hex::new(4, 6));
    }
}
