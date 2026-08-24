//! Where a cell sits on screen, and which cell the mouse is over.
//!
//! Everything else in this crate is integer lattice geometry: cells, neighbours, distances, routes.
//! None of that needs to know what a pixel is. But a game does, twice — it must **draw** the board,
//! and it must work out **which cell the player just clicked**. That is what this module is, and it
//! is all it is.
//!
//! ```
//! use spacewalk::{FullGrid, Grid, Hex, HexLayout, Pt};
//!
//! let g = FullGrid::hexagon(4);
//! let layout = HexLayout::pointy(Pt::new(32.0, 32.0)).at(Pt::new(400.0, 300.0));
//!
//! // Drawing: where does this cell go?
//! assert_eq!(layout.center(Hex::new(0, 0)), Pt::new(400.0, 300.0));
//!
//! // Picking: which cell is under the cursor? Off the board is `None`, for free.
//! assert!(g.index_of(layout.hex_at(Pt::new(420.0, 290.0))).is_some());
//! assert_eq!(g.index_of(layout.hex_at(Pt::new(9000.0, 9000.0))), None);
//! ```
//!
//! # It cannot corrupt a search
//!
//! This module is the only floating-point in the crate's public API, and it is a **one-way street**.
//! No `f32` here ever reaches a [`Cost`](crate::Cost), a [`Metric`](crate::Metric), or the step
//! table. Pathfinding stays integer, and therefore stays reproducible: a replay, a lockstep
//! multiplayer peer, and a unit test all still get the same answer. Layout is presentation, and
//! presentation is downstream of everything.
//!
//! # `f32` at the edges, `f64` in the middle
//!
//! The API speaks `f32`, as common graphics APIs do, and a cast at every call site of every frame
//! is friction for nothing. The arithmetic inside is `f64`, because picking a cell from a pixel is
//! a *rounding* problem and rounding is where precision is spent. A board is capped at
//! [`MAX_CELLS`](crate::MAX_CELLS), so no legal board reaches a pixel coordinate an `f32` cannot
//! hold exactly — but a stray click can, and `f64` keeps that honest.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::coord::{clamp_i32, hex_round};
use crate::float;
use crate::{Hex, Sq};

/// `√3`, which is the whole of hex trigonometry.
const SQRT3: f64 = 1.732_050_807_568_877_2;

/// A point on the screen, in whatever units you draw in.
///
/// Deliberately not `glam::Vec2` or `mint::Point2`. This crate has one dependency and no opinion
/// about your maths library; converting two floats at the boundary is cheaper for you than a
/// version conflict is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Pt {
    /// Rightward.
    pub x: f32,
    /// Downward, as screen coordinates go.
    pub y: f32,
}

impl Pt {
    /// A point at `(x, y)`.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<(f32, f32)> for Pt {
    fn from((x, y): (f32, f32)) -> Self {
        Pt::new(x, y)
    }
}

/// Which way up the hexes are drawn.
///
/// # The compass names only tell the truth for one of these
///
/// [`Dir6`](crate::Dir6)'s names — `E`, `Ne`, `Nw`, … — describe a **pointy-top** board, because a
/// pointy-top hex is the one with a neighbour due east. A **flat-top** hex has no due-east
/// neighbour at all; it has one due *north* and one due *south*. That is a fact about hexagons, not
/// a choice this crate made, and no naming could paper over it.
///
/// So under [`Flat`](Orientation::Flat) the lattice is unchanged and every route, distance and
/// neighbour is exactly what it was — but the *names* become labels for lattice axes rather than
/// promises about pixels. Each renders 30° around from where it reads:
///
/// | [`Dir6`](crate::Dir6) | under `Pointy` | under `Flat` |
/// |---|---|---|
/// | `E`  | due east  | east-south-east |
/// | `Nw` | north-west | **due north** |
/// | `Se` | south-east | **due south** |
///
/// Pick the orientation you want to *look* at. It changes nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Orientation {
    /// A vertex at the top. Neighbours due east and west; rows stagger.
    Pointy,
    /// A flat edge at the top. Neighbours due north and south; columns stagger.
    Flat,
}

/// Where the hexes of a board land on screen.
///
/// `size` is the **circumradius** — centre to vertex — so a hex is `2 · size` from vertex to
/// opposite vertex, and adjacent centres are `√3 · size` apart. (For squares, [`SqLayout`]'s `size`
/// is the whole cell instead, because that is what everyone means by a 32-pixel tile. Each field is
/// the obvious thing for its own shape; they are not the same thing.)
///
/// Give `size` two different components to squash the board into an isometric-looking ellipse.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HexLayout {
    /// Which way up the hexes are drawn.
    pub orientation: Orientation,
    /// Centre-to-vertex, per axis.
    pub size: Pt,
    /// Where [`Hex::new(0, 0)`] lands.
    pub origin: Pt,
}

impl HexLayout {
    /// Pointy-top hexes of the given circumradius, with the origin cell at `(0, 0)`.
    #[must_use]
    pub const fn pointy(size: Pt) -> Self {
        Self {
            orientation: Orientation::Pointy,
            size,
            origin: Pt::new(0.0, 0.0),
        }
    }

    /// Flat-top hexes of the given circumradius, with the origin cell at `(0, 0)`.
    ///
    /// Mind [`Orientation`]: the lattice is the same, but `Dir6`'s compass names stop matching the
    /// screen.
    #[must_use]
    pub const fn flat(size: Pt) -> Self {
        Self {
            orientation: Orientation::Flat,
            ..Self::pointy(size)
        }
    }

    /// Move the board so [`Hex::new(0, 0)`] lands here — the centre of your viewport, usually.
    #[must_use]
    pub const fn at(mut self, origin: Pt) -> Self {
        self.origin = origin;
        self
    }

    /// The centre of a cell, in pixels. This is where you draw it.
    #[must_use]
    pub fn center(&self, h: Hex) -> Pt {
        let (q, r) = (f64::from(h.q), f64::from(h.r));

        // Both matrices are re-derived for THIS crate, and the difference is not cosmetic. Every
        // published hex formula assumes axial `r` grows south. Here `r` grows north-EAST (`Dir6::Ne`
        // is `r + 1`) while screen `y` grows down, so the standard matrices are quietly wrong on
        // this lattice. `tests/screen.rs` pins the derivation the only way that means anything: it
        // checks each `Dir6` lands where its name says, and that all six neighbours come out the
        // same distance from the centre — which they only do if a hex is really a hexagon.
        //
        // Flat is Pointy turned 30°. A rotation preserves distance, so equidistance survives it for
        // free, and one verified matrix beats two hand-derived ones.
        let (u, v) = match self.orientation {
            Orientation::Pointy => (SQRT3 * q + SQRT3 / 2.0 * r, -1.5 * r),
            Orientation::Flat => (1.5 * (q + r), SQRT3 / 2.0 * (q - r)),
        };

        place(self.size, u, v, self.origin)
    }

    /// The cell containing a pixel — the cell under the mouse.
    ///
    /// This answers for the *whole plane*, so it always names some cell, whether or not your board
    /// has one there. That composes: [`Grid::index_of`](crate::Grid::index_of) already returns
    /// `Option`, so a click outside the board is `None` without a special case.
    ///
    /// ```
    /// # use spacewalk::{FullGrid, Grid, HexLayout, Pt};
    /// let g = FullGrid::hexagon(3);
    /// let layout = HexLayout::pointy(Pt::new(20.0, 20.0));
    ///
    /// let hovered = g.index_of(layout.hex_at(Pt::new(140.0, 0.0)));
    /// assert_eq!(hovered, None, "past the edge of the board");
    /// ```
    ///
    /// # Panics
    ///
    /// If either component of `size` is zero, which would divide by it.
    #[must_use]
    pub fn hex_at(&self, p: Pt) -> Hex {
        let (u, v) = local("HexLayout", self.size, self.origin, p);

        // The inverse of `center`, and it must stay the inverse of `center`. `tests/screen.rs`
        // brute-forces the property that actually matters — that this returns the NEAREST cell
        // centre — over a dense field of pixels, which catches any sign error in either direction.
        let (q, r) = match self.orientation {
            Orientation::Pointy => (u / SQRT3 + v / 3.0, -2.0 / 3.0 * v),
            Orientation::Flat => (u / 3.0 + v / SQRT3, u / 3.0 - v / SQRT3),
        };

        // `hex_round` is shared with line-drawing: "which cell is this fractional point in" is one
        // question, and it deserves one answer. It clamps, so a click at 1e30 lands off any board
        // rather than wrapping onto a real cell.
        hex_round(q, r)
    }

    /// The six vertices, in order, for drawing the outline.
    ///
    /// ```
    /// # use spacewalk::{FullGrid, Grid, Hex, HexLayout, Pt};
    /// let layout = HexLayout::pointy(Pt::new(10.0, 10.0));
    /// let c = layout.corners(Hex::new(0, 0));
    ///
    /// // A pointy-top hex has a vertex straight up. (Screen y grows downward.)
    /// assert!(c.iter().any(|p| p.x.abs() < 1e-4 && (p.y + 10.0).abs() < 1e-4));
    /// ```
    #[must_use]
    pub fn corners(&self, h: Hex) -> [Pt; 6] {
        let c = self.center(h);
        // Pointy has a vertex at 12 o'clock, flat has one at 3 o'clock: the same ring, 30° apart.
        // Six fixed angles per orientation, so twelve numbers rather than twelve calls to `cos`.
        let ring = match self.orientation {
            Orientation::Pointy => &float::POINTY_CORNERS,
            Orientation::Flat => &float::FLAT_CORNERS,
        };
        core::array::from_fn(|i| place(self.size, ring[i].0, ring[i].1, c))
    }
}

/// Where the cells of a square board land on screen.
///
/// `size` is the **whole cell** — a 32-pixel tile is `Pt::new(32.0, 32.0)` — because that is what
/// everybody means by a tile size. ([`HexLayout`]'s `size` is a circumradius instead; a hexagon has
/// no one obvious width.)
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SqLayout {
    /// The full width and height of one cell.
    pub size: Pt,
    /// The top-left corner of [`Sq::new(0, 0)`].
    pub origin: Pt,
}

impl SqLayout {
    /// Cells of the given size, with the top-left corner of `(0, 0)` at the origin.
    #[must_use]
    pub const fn new(size: Pt) -> Self {
        Self {
            size,
            origin: Pt::new(0.0, 0.0),
        }
    }

    /// Move the board so the top-left corner of [`Sq::new(0, 0)`] lands here.
    #[must_use]
    pub const fn at(mut self, origin: Pt) -> Self {
        self.origin = origin;
        self
    }

    /// The **centre** of a cell — not its corner. Sprites are usually drawn from the middle, and it
    /// keeps this the exact counterpart of [`HexLayout::center`].
    #[must_use]
    pub fn center(&self, s: Sq) -> Pt {
        let (u, v) = (f64::from(s.x) + 0.5, f64::from(s.y) + 0.5);
        place(self.size, u, v, self.origin)
    }

    /// The cell containing a pixel — the cell under the mouse.
    ///
    /// ```
    /// # use spacewalk::{Adjacency, FullGrid, Grid, Pt, Sq, SqLayout};
    /// let g = FullGrid::square(8, 8, Adjacency::Four);
    /// let layout = SqLayout::new(Pt::new(32.0, 32.0));
    ///
    /// assert_eq!(layout.sq_at(Pt::new(40.0, 40.0)), Sq::new(1, 1));
    ///
    /// let hovered = g.index_of(layout.sq_at(Pt::new(-5.0, 40.0)));
    /// assert_eq!(hovered, None, "left of the board");
    /// ```
    ///
    /// # Panics
    ///
    /// If either component of `size` is zero.
    #[must_use]
    pub fn sq_at(&self, p: Pt) -> Sq {
        let (u, v) = local("SqLayout", self.size, self.origin, p);
        // Floor, not truncate: truncation folds -0.5 and +0.5 onto the same cell, putting a seam
        // down the middle of the board that only shows up in the negative quadrant.
        Sq::new(clamp_i32(float::floor(u)), clamp_i32(float::floor(v)))
    }

    /// The four corners, clockwise from the top-left.
    #[must_use]
    pub fn corners(&self, s: Sq) -> [Pt; 4] {
        let c = self.center(s);
        let (hw, hh) = (self.size.x / 2.0, self.size.y / 2.0);
        [
            Pt::new(c.x - hw, c.y - hh),
            Pt::new(c.x + hw, c.y - hh),
            Pt::new(c.x + hw, c.y + hh),
            Pt::new(c.x - hw, c.y + hh),
        ]
    }
}

/// Axial ↔ **offset** coordinates: the `(col, row)` a tilemap editor speaks.
///
/// Axial `(q, r)` is what makes hex arithmetic bearable, so it is what this crate uses. It is not
/// what common tile map formats *store*. They address hexes as a plain rectangular `(col, row)`
/// grid with every other line nudged half a cell sideways, which is easy to author and horrible
/// to do maths on. This is the translation, and you need it the moment you load a map.
///
/// | Variant | Staggered axis | Staggered parity |
/// |---|---|---|
/// | [`OddR`](Offset::OddR) / [`EvenR`](Offset::EvenR) | **Y** | Odd / Even |
/// | [`OddQ`](Offset::OddQ) / [`EvenQ`](Offset::EvenQ) | **X** | Odd / Even |
///
/// The `R` pair staggers rows and goes with [`Orientation::Pointy`]; the `Q` pair staggers columns
/// and goes with [`Orientation::Flat`].
///
/// ```
/// use spacewalk::{FullGrid, Grid, Hex, Offset};
///
/// let cell = Offset::OddR.from_hex(Hex::new(2, -1));   // -> an offset (col, row)
/// assert_eq!(Offset::OddR.to_hex(cell.0, cell.1), Hex::new(2, -1));
/// ```
///
/// # Do not reach for the textbook here
///
/// Every published offset formula assumes axial `r` grows *south*. In this crate it grows
/// north-**east**, so going *down* a row goes down-*left* — the mirror of what those formulas
/// assume — and the shear they apply has the wrong sign. Copying it produces a board that
/// round-trips perfectly and still puts neighbours in the wrong places: no panic, no error, just a
/// map that is subtly wrong. (Measured, before it was fixed: 361 broken adjacencies on a board this
/// small.) `tests/screen.rs` asserts the property that catches it — every one of a cell's six real
/// neighbours must land within one column and one row of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Offset {
    /// Pointy-top, rows staggered, **odd** rows pushed right.
    OddR,
    /// Pointy-top, rows staggered, **even** rows pushed right.
    EvenR,
    /// Flat-top, columns staggered, **odd** columns pushed down.
    OddQ,
    /// Flat-top, columns staggered, **even** columns pushed down.
    EvenQ,
}

impl Offset {
    /// The `(col, row)` a tilemap would store this cell at.
    #[must_use]
    pub fn from_hex(self, h: Hex) -> (i32, i32) {
        let (q, r) = (i64::from(h.q), i64::from(h.r));
        let (col, row) = match self {
            // Rows run with screen y, and screen y runs opposite to `r`. Hence `row = -r`, and hence
            // the shear is subtracted where the textbook adds it.
            Self::OddR | Self::EvenR => {
                let row = -r;
                (q - shear(row, self == Self::EvenR), row)
            }
            // Columns run with screen x, which grows with `q + r`.
            Self::OddQ | Self::EvenQ => {
                let col = q + r;
                (col, q - shear(col, self == Self::EvenQ))
            }
        };
        (clamp_i64(col), clamp_i64(row))
    }

    /// The cell a tilemap's `(col, row)` means. The exact inverse of [`Offset::from_hex`].
    #[must_use]
    pub fn to_hex(self, col: i32, row: i32) -> Hex {
        let (col, row) = (i64::from(col), i64::from(row));
        let (q, r) = match self {
            Self::OddR | Self::EvenR => (col + shear(row, self == Self::EvenR), -row),
            Self::OddQ | Self::EvenQ => {
                let s = shear(col, self == Self::EvenQ);
                (row + s, col - row - s)
            }
        };
        Hex::new(clamp_i64(q), clamp_i64(r))
    }
}

/// Half a cell's worth of stagger for this row or column, rounded so the divide is always exact.
///
/// `line & 1` is 1 for odd numbers on both sides of zero (two's complement), and the numerator is
/// always even, so the division is exact and truncation-versus-floor never arises.
fn shear(line: i64, even: bool) -> i64 {
    (line + if even { -(line & 1) } else { line & 1 }) / 2
}

/// The one projection rule, cell units to pixels: scale and translate in `f64`, narrow only the
/// final answer. Both `center`s and the hex `corners` project through here; the square corners
/// are plain `f32` offsets from an already-projected centre.
fn place(size: Pt, u: f64, v: f64, at: Pt) -> Pt {
    // `mul_add` would be one rounding rather than two, but it is not in `core` and it is a fused
    // instruction this crate has no need of: the difference is one ulp, against a lattice whose
    // cells are pixels wide.
    #[allow(clippy::cast_possible_truncation)]
    Pt::new(
        (f64::from(size.x) * u + f64::from(at.x)) as f32,
        (f64::from(size.y) * v + f64::from(at.y)) as f32,
    )
}

/// The inverse of [`place`]: a pixel in cell-local units. Shared by [`HexLayout::hex_at`] and
/// [`SqLayout::sq_at`], which also share the reason this panics on a zero `size`.
fn local(kind: &str, size: Pt, origin: Pt, p: Pt) -> (f64, f64) {
    assert!(
        size.x != 0.0 && size.y != 0.0,
        "a {kind} with a zero size collapses every cell onto one point, so no pixel can name \
         a cell; size is {size:?}"
    );
    let u = (f64::from(p.x) - f64::from(origin.x)) / f64::from(size.x);
    let v = (f64::from(p.y) - f64::from(origin.y)) / f64::from(size.y);
    (u, v)
}

/// Coordinates arriving from pixel space are numbers this crate did not choose, so they get the
/// same treatment as every other such number: clamped, never wrapped.
fn clamp_i64(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
