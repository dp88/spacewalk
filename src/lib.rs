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
//! let from = g.index_of(Sq::new(0, 0)).unwrap();
//! let to   = g.index_of(Sq::new(7, 7)).unwrap();
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
//! let eye = g.index_of(Sq::new(8, 8)).unwrap();
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
//! **Indices are not addresses.** [`Idx`] is a dense `u32`, valid only within the board that issued
//! it. Two boards with the same cells may number them differently; [`FullGrid::filtered`] renumbers,
//! and a [`SubGrid`] numbers its own cells from zero. *Serialize coordinates, never indices.*
//!
//! A subset makes this sharper than a filter does, because both numberings stay **live**: the
//! region's index 0 and the board's index 0 are each valid and each a different cell. Nothing can
//! catch that at runtime — every index is in range for one of them. [`Grid::to_root`] and
//! [`Grid::of_root`] are the bridge, and the only correct one. See [`SubGrid`] for the two ways it
//! bites.
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

#![deny(missing_docs)]

/// The README is compiled as part of the test suite, so its examples cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod cells;
pub mod coord;
pub mod full;
pub mod grid;
pub mod layout;
pub mod path;
pub mod rect;
pub mod square;
pub mod sub;

pub use cells::CellMap;
pub use coord::{Coord, Dir6, Dir8, Hex, Idx, Lerp, Metric, Sq};
pub use full::{Adjacency, FullGrid, MAX_CELLS};
pub use grid::{Dir, Grid, MAX_SIGHT};
pub use layout::{HexLayout, Offset, Orientation, Pt, SqLayout};
pub use path::{Cost, Movement, Path, Step};
pub use rect::RectGrid;
pub use sub::SubGrid;
