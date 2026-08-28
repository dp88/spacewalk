//! A generic basis for grids: square, hex, and whatever else your game needs.
//!
//! Board games and tactics games have the same skeleton — a set of cells, a notion of which cells
//! touch which, a distance, and a way to find a route. Only the shape changes. `spacewalk` is
//! that skeleton, with the shape left open: chess and draughts on squares, a clone-and-jump
//! capture game on hexes, a tactical battle on either, a three-layer chess board on something you
//! define yourself.
//!
//! # The grid holds no game state
//!
//! A grid is cells, their indices, and which cell each direction leads to. It knows nothing of
//! terrain, pieces, or players — you keep those, in whatever shape suits you, and hand the grid a
//! closure when you want a path:
//!
//! ```
//! use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
//!
//! let g = FullGrid::square(8, 8, Adjacency::Four);
//! let mud: Vec<Sq> = vec![Sq::new(3, 3), Sq::new(3, 4)];      // your data, your types
//!
//! let walk = Movement::scan(&g, |s| {
//!     Some(if mud.contains(&g.coord(s.to)) { 30 } else { 10 })
//! });
//!
//! let from = g.at(Sq::new(0, 0));
//! let to   = g.at(Sq::new(7, 7));
//! assert_eq!(g.path(from, to, &walk).unwrap().len(), 14);
//! ```
//!
//! That keeps the grid immutable and shareable, and keeps it out of your borrow checker's way:
//! `&grid` and `&mut your_state` are different objects, so an AI search can read the board while
//! it mutates the position, and cloning a position never copies the board.
//!
//! When "whatever shape suits you" is simply one value per cell, [`CellMap`] is that `Vec`, sized
//! from the board and subscripted by an [`Idx`] rather than by a hand-written cast. It holds no
//! grid, so the two objects above stay two objects.
//!
//! Indices go in; coordinates come back out with [`Grid::coords_of`], which is what you draw and
//! what you save. [`Grid::at`] is the way in for a cell you already know is on the board, and
//! [`Grid::index_of`] the way in when being off it is an answer rather than a mistake.
//!
//! # Cost is what it costs to enter a cell, from a direction
//!
//! Not the cost *of* the cell. That difference is what lets a grid hold a river: entering it
//! downstream is cheap, upstream is dear. A conveyor belt, and a ledge you can drop off but not
//! climb back up, are the same shape. See [`path`] — and mind that it makes the graph **directed**,
//! so a [`Path`] cannot simply be reversed.
//!
//! # Four shapes ship; the rest is yours
//!
//! [`FullGrid::square`], [`FullGrid::disc`], [`FullGrid::hexagon`] and [`FullGrid::hex_rect`] are
//! the common cases — a disc is a round board of square cells centred on the origin, and a hex
//! rectangle is the shape a tilemap editor authors. Beyond them, [`FullGrid::new`] takes any set of
//! cells at all — a checkers board is the dark squares of a square grid ([`FullGrid::filtered`]),
//! and a game with a genuinely different geometry implements [`Coord`]. That is a few dozen lines;
//! `tests/chess3d.rs` builds a three-layer chess board without touching this crate.
//!
//! # A plain rectangle stores nothing
//!
//! A [`FullGrid`] holds any set of cells, so it must store the cells, an index over them, and the
//! step table both ways. On a full `w × h` rectangle each of those is arithmetic, and
//! [`RectGrid`] is that arithmetic: same answers, same indices, three fields whatever the size of
//! the board. It is what a large outdoor map wants. `FullGrid` is what everything else wants.
//!
//! # A region of a board is a board
//!
//! [`Grid`] is the vocabulary — every question above, asked of anything that is a board. Three
//! things answer it. [`FullGrid`] is one you built and [`RectGrid`] one it computes. [`SubGrid`] is
//! *part* of either, and it is a board in its own right: it has its own edges, its own components,
//! its own paths.
//!
//! That is what a highlighted range is. Ask for one, and you get the thing you draw **and** the
//! thing you then reason over:
//!
//! ```
//! use spacewalk::{Adjacency, FullGrid, Grid, Sq};
//!
//! let g = FullGrid::square(16, 16, Adjacency::Eight);
//! let eye = g.at(Sq::new(8, 8));
//! let wall = |i| g.coord(i).x == 10;
//!
//! let seen = g.visible_from(eye, 5, wall);       // a board of what this unit can see
//! assert!(seen.contains(Sq::new(9, 8)));
//! assert!(!seen.contains(Sq::new(12, 8)), "the wall is in the way");
//!
//! // And it is a board: a route inside a field of view cannot leave it.
//! for i in seen.indices() {
//!     assert!(g.coord(seen.to_root(i)).x <= 10);
//! }
//! ```
//!
//! [`Grid::within`], [`Grid::ring`], [`Grid::component`] and [`Grid::visible_from`] all hand one
//! back; [`Grid::subset`] makes one from any cells you name. A `SubGrid` borrows rather than copies,
//! so asking for one costs a sort, not a board.
//!
//! # Height is game state, like everything else
//!
//! A cell's ground level is not geometry, so the grid does not hold it. It goes in a [`CellMap`]
//! beside your terrain, and two gates read it: [`height_gate`] for what a hill hides,
//! [`climb_gate`] for what a ledge refuses.
//!
//! ```
//! use spacewalk::prelude::*;
//!
//! let g = FullGrid::square(9, 3, Adjacency::Eight);
//! let mut ground = CellMap::new(&g, 0i32);
//! ground[g.at(Sq::new(4, 1))] = 4;                     // a ridge across the middle
//!
//! let eye = g.at(Sq::new(0, 1));
//! let sight = height_gate(&g, |i| ground[i], |i| ground[i] + 2);
//!
//! let seen = g.visible_from_by(eye, 8, &sight);
//! assert!(seen.contains(Sq::new(4, 1)), "the ridge is in plain view");
//! assert!(!seen.contains(Sq::new(8, 1)), "and the dead ground behind it is not");
//! ```
//!
//! Sight needed one thing a plain blocker could not give it. A hill hides what is *lower* than the
//! line passing over it, so whether a cell blocks depends on the target as much as on the cell.
//! [`Grid::los_by`] and [`Grid::visible_from_by`] hand the predicate a [`Sight`] — the whole
//! question, the way a cost function is handed a whole [`Step`]. [`Grid::los`] and
//! [`Grid::visible_from`] remain as compatibility wrappers that throw the target away.
//!
//! Movement needed nothing new. A climb is priced by the cell entered and the direction of arrival,
//! which is the river and the one-way ledge above; [`climb_gate`] only adds the limit past which a
//! step is refused outright.
//!
//! # Drawing it, and clicking on it
//!
//! The lattice does not know what a pixel is, and does not need to — until you want to *show* it.
//! [`layout`] is that, and only that: where a cell lands on screen, and which cell the mouse is
//! over. It is the one part of the crate that speaks `f32`, and it is a one-way street — no float
//! ever reaches a [`Cost`] or a [`Metric`], so pathfinding stays integer and stays reproducible.
//!
//! ```
//! use spacewalk::{FullGrid, Grid, HexLayout, Pt};
//!
//! let g = FullGrid::hexagon(4);
//! let layout = HexLayout::pointy(Pt::new(32.0, 32.0)).at(Pt::new(400.0, 300.0));
//!
//! // Which cell did they click? Off the board is `None`, with no special case.
//! let hovered = g.index_of(layout.hex_at(Pt::new(430.0, 310.0)));
//! assert!(hovered.is_some());
//! ```
//!
//! [`Offset`] converts to the `(col, row)` a tilemap editor stores, which is what you need the
//! moment you load a map someone else authored; [`FullGrid::hex_rect`] builds the board that file
//! describes, in the same convention.
//!
//! # Two things to know before you build one
//!
//! **Indices are not addresses.** [`Idx`] is a dense index, valid only within the board that issued
//! it. Two boards with the same cells may number them differently; [`FullGrid::filtered`] renumbers,
//! and a [`SubGrid`] numbers its own cells from zero. *Serialize coordinates, never indices.*
//!
//! A subset makes this sharper than a filter does, because both numberings stay **live**: the
//! region's index 0 and the board's index 0 are each valid and each a different cell. No bounds
//! check can separate them — every index is in range for one of them. [`Grid::to_root`] and
//! [`Grid::of_root`] are the bridge, and the only correct one. See [`SubGrid`] for the two ways it
//! bites.
//!
//! So an index carries the board that issued it, and a **debug build checks it**:
//!
//! ```
//! use spacewalk::{Adjacency, FullGrid, Grid, Sq};
//!
//! let board = FullGrid::square(8, 8, Adjacency::Four);
//! let dark = board.filtered(|c| (c.x + c.y) % 2 == 0);
//! let light = board.filtered(|c| (c.x + c.y) % 2 == 1);
//! assert_eq!(dark.len(), light.len(), "so no bound can tell them apart");
//!
//! let cell = dark.at(Sq::new(2, 2));
//! assert_eq!(dark.coord(cell), Sq::new(2, 2));
//!
//! // `light.coord(cell)` is index 9 either way — a real cell of both boards, and a different one.
//! // In a debug build it panics: "issued by a different grid". Shipped, it quietly answers
//! // Sq::new(3, 2), which is why the rule below is still the rule.
//! ```
//!
//! That is a [`Tag`], and in release it is zero-sized: the checks vanish and an `Idx` is a bare
//! `u32` again. Equality, ordering, and hashing compare the number alone in both profiles, so
//! nothing you observe changes with the build — only whether the mistake is reported. Treat it as
//! it is meant: a development aid, not a runtime guarantee. The rule is still *serialize
//! coordinates*.
//!
//! Two boards that number the same cells the same way share a tag and are interchangeable, which
//! is what makes rebuilding a board from [`Grid::cells`] restore the *same indices* rather than
//! merely an equivalent board.
//!
//! **The metric must agree with the adjacency.** An eight-way board measured with Manhattan
//! distance is the classic tactics bug: a unit can *step* to the enemy diagonally beside it, but
//! measures it as two cells away and so cannot *attack* it. [`Adjacency`] picks both together, so
//! there is no second knob to get wrong; a hand-built grid must uphold it (see [`FullGrid::new`]).
//!
//! # Feature flags
//!
//! - `serde` (off by default): `Serialize`/`Deserialize` on every plain-data type — [`Sq`],
//!   [`Hex`], [`Dir8`], [`Dir6`], [`Adjacency`], [`Pt`], [`Orientation`], [`HexLayout`],
//!   [`SqLayout`], [`Offset`], and [`CellMap<T>`](CellMap) whenever `T` is. [`FullGrid`] and
//!   [`Metric`] are deliberately not serializable: a grid is rebuilt from its cells, and a metric
//!   holds function pointers. `tests/save.rs` shows what to persist instead — coordinates, never
//!   indices, and a `CellMap` only alongside the cells that fix its order.
//!
//! # It does not need `std`
//!
//! The crate is `#![no_std]`, unconditionally, and builds for a bare-metal target. Nothing in it
//! wants an operating system — no files, no threads, no clock — so `alloc` is the whole of what it
//! asks for, and that is present wherever `std` is. There is no feature to turn on and nothing an
//! ordinary user does differently.
//!
//! Two things follow that are worth knowing. `core` has no `f64::round`, `floor`, `cos`, or `sin`,
//! so [`layout`] carries its own — the hexagon corner angles were always twelve fixed numbers, and
//! the rounding is integer arithmetic once a value is known to be finite. Each one is checked
//! against `std`'s over a few million values, because hand-written floating point deserves the
//! suspicion.
//!
//! And the dependency list is one crate: `hashbrown`, which is the table `std`'s own `HashMap` is
//! built from. The searches behind [`Grid::path`] and [`Grid::reachable`] are the crate's own — a
//! general graph library must key its bookkeeping on whatever a node happens to be, and a board's
//! cells are already numbered `0..len`, which turns that map into two vectors read by subscript.
//!
//! # Where to start
//!
//! [`prelude`] imports the names above in one line. [`Grid`] is where the vocabulary lives, and its
//! own documentation says which questions hand back an iterator, which a board, and which a `Vec`.

#![no_std]

extern crate alloc;

// Unit tests inside the crate use `std` freely — that is how the hand-written floating point in
// `float` is checked against the real thing, and how `cells` catches a panic.
#[cfg(test)]
extern crate std;

/// The README is compiled as part of the test suite, so its examples cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod cells;
pub mod coord;
/// Floating point `core` does not carry. Private: it is an implementation detail of
/// [`layout`] and [`coord`].
mod float;
pub mod full;
pub mod grid;
pub mod height;
pub mod layout;
pub mod path;
pub mod rect;
/// A\* and Dijkstra over a board's dense indices. Private: [`Grid`] is the way in.
mod search;
pub mod square;
pub mod sub;

pub use cells::CellMap;
pub use coord::{Coord, Dir6, Dir8, Hex, Idx, Lerp, Metric, Sq, Tag};
pub use full::{Adjacency, FullGrid, GridError, MAX_CELLS};
pub use grid::{Dir, Grid, MAX_SIGHT, Sight};
pub use height::{climb_gate, height_gate};
pub use layout::{HexLayout, Offset, Orientation, Pt, SqLayout};
pub use path::{Cost, Movement, MovementError, Path, Step};
pub use rect::RectGrid;
pub use square::{CornerRule, corner_gate};
pub use sub::SubGrid;

/// Everything you need to build a board and ask it something.
///
/// [`Grid`] is a trait, so it must be in scope for any of its thirty-odd methods to be callable —
/// which is the one import nobody guesses. This is that, plus the handful of names that come with
/// it.
///
/// ```
/// use spacewalk::prelude::*;
///
/// let g = FullGrid::square(8, 8, Adjacency::Four);
/// let walk = Movement::uniform(&g, 1);
/// assert_eq!(g.path(g.at(Sq::new(0, 0)), g.at(Sq::new(7, 7)), &walk).unwrap().len(), 14);
/// ```
pub mod prelude {
    pub use crate::{
        Adjacency, CellMap, Coord, CornerRule, Cost, Dir, Dir6, Dir8, FullGrid, Grid, Hex,
        HexLayout, Idx, Lerp, MAX_CELLS, MAX_SIGHT, Metric, Movement, MovementError, Offset,
        Orientation, Path, Pt, RectGrid, Sight, Sq, SqLayout, Step, SubGrid, Tag, climb_gate,
        corner_gate, height_gate,
    };
}
