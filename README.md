# spacewalk

[![CI](https://github.com/dp88/spacewalk/actions/workflows/ci.yml/badge.svg)](https://github.com/dp88/spacewalk/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

A generic basis for grids: square, hex, and whatever else your game needs.

Board games and tactics games have the same skeleton — a set of cells, a notion of which cells
touch which, a distance, and a way to find a route. Only the shape changes. This is that skeleton,
with the shape left open.

```toml
[dependencies]
spacewalk = "0.1"
```

```rust
use spacewalk::prelude::*;
```

## The grid holds no game state

A grid is cells, their indices, and which cell each direction leads to. It knows nothing of
terrain, pieces, or players — you keep those, in whatever shape suits you, and hand the grid a
closure when you want a path.

```rust
use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};

let g = FullGrid::square(8, 8, Adjacency::Four);
let mud = vec![Sq::new(3, 3), Sq::new(3, 4)];          // your data, your types

let walk = Movement::scan(&g, |s| {
    Some(if mud.contains(&g.coord(s.to)) { 30 } else { 10 })
});

let from = g.at(Sq::new(0, 0));
let to   = g.at(Sq::new(7, 7));
let path = g.path(from, to, &walk).unwrap();

assert_eq!(path.len(), 14);                            // fourteen orthogonal steps
```

That keeps the grid immutable and shareable, and keeps it out of your borrow checker's way:
`&grid` and `&mut your_state` are different objects, so an AI search can read the board while it
mutates the position, and cloning a position never copies the board.

When your state is simply one value per cell, `CellMap<T>` is that vector — sized from the board,
and subscripted by an `Idx` rather than by a cast you wrote out by hand:

```rust
# use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Sq};
let g = FullGrid::square(8, 8, Adjacency::Four);
let mut mud = CellMap::new(&g, false);

mud[g.at(Sq::new(3, 3))] = true;
assert_eq!(mud.iter().filter(|&(_, &m)| m).count(), 1);
```

It borrows the grid to measure itself and does not hold one, so those are still two objects. It
goes stale exactly as an `Idx` does: build a fresh one after `filtered`.

## A region of a board is a board

`Grid` is the vocabulary — every question below, asked of anything that is a board. `FullGrid` is
one you built; `RectGrid` is a plain rectangle it computes instead of stores; `SubGrid` is *part*
of either, and it is a board in its own right, with its own edges, its own components, and its own
paths.

That is what a highlighted range is: ask for one, and you get the thing you draw **and** the thing
you then reason over.

```rust
# use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};
let g = FullGrid::square(16, 16, Adjacency::Eight);
let unit = g.at(Sq::new(4, 4));
let walk = Movement::scan(&g, |_| Some(10));

// Where can it go this turn? The cells, with what reaching each one costs.
let budget: Vec<_> = g.reachable(unit, 30, &walk);

// As a board: now a route inside the highlight cannot wander outside it.
let range = g.subset(budget.iter().map(|&(i, _)| i));
let far = range.at(Sq::new(4, 7));

assert!(range.contains(Sq::new(4, 7)));
assert!(range.path(range.at(Sq::new(4, 4)), far, &walk).is_some());
```

`within`, `ring`, `component` and `visible_from` hand one back directly; `subset` makes one from
any cells you name. A `SubGrid` borrows rather than copies, so asking for one costs a sort, not a
board — which is why a range query can return one at all.

## Cost is what it costs to *enter* a cell, from a *direction*

Not the cost *of* the cell. That difference is what lets a board hold a river: entering it
downstream is cheap, upstream is dear.

```rust
# use spacewalk::{Adjacency, Dir8, FullGrid, Grid, Movement, Sq};
# let g = FullGrid::square(8, 8, Adjacency::Four);
let wall  = g.at(Sq::new(4, 4));
let river = g.at(Sq::new(2, 2));

let walk = Movement::scan(&g, |s| match s.to {
    t if t == wall  => None,                                          // impassable
    t if t == river => Some(if s.dir == Dir8::S { 1 } else { 50 }),   // the current runs south
    _ => Some(10),                                                    // open ground
});

let above = g.at(Sq::new(2, 1));
let below = g.at(Sq::new(2, 3));

assert_eq!(g.path(above, below, &walk).unwrap().cost(), 11);   // downstream, through the river
assert_eq!(g.path(below, above, &walk).unwrap().cost(), 40);   // upstream, cheaper to go around
```

A conveyor belt, and a ledge you can drop off but not climb back up, are the same shape. One
`Option<Cost>` closure covers terrain cost, impassable cells, cells blocked by other pieces, and
one-way movement — four separate mechanisms in most engines.

The price is that the graph is **directed**: a path cannot be reversed, and reaching somewhere does
not mean you can get back.

## Four shapes ship; the rest is yours

`FullGrid::square`, `FullGrid::disc`, `FullGrid::hexagon` and `FullGrid::hex_rect` are the common
cases. A disc is a round board of square cells centred on the origin, which is what an arena wants
and what `square` cannot give you — it only ever emits `0..w` by `0..h`, so it has no middle to
name. A hex rectangle is what a tactics map usually is and what a tilemap editor authors. Beyond
them, `FullGrid::new` takes any set of cells at all, and `FullGrid::filtered` drops the ones you
don't want — a draughts board is the dark squares of a square grid, a holed hex board is a hexagon
less its gaps.

A game with a genuinely different geometry implements `Coord`, which is three items: an associated
direction type, the list of directions, and where one step lands. `tests/chess3d.rs` builds a
three-layer chess board that way, in the test file, with no change to this crate.

## Drawing it, and clicking on it

The lattice does not know what a pixel is, and does not need to — until you want to *show* it.

```rust
use spacewalk::{FullGrid, Grid, HexLayout, Pt};

let g = FullGrid::hexagon(4);
let layout = HexLayout::pointy(Pt::new(32.0, 32.0)).at(Pt::new(400.0, 300.0));

for i in g.indices() {
    let c = layout.center(g.coord(i));      // where to draw it
    let outline = layout.corners(g.coord(i));  // and its six vertices
    # let _ = (c, outline);
}

// Which cell is under the mouse? Off the board is `None`, with no special case,
// because `index_of` already returns an Option.
let hovered = g.index_of(layout.hex_at(Pt::new(430.0, 310.0)));
assert!(hovered.is_some());
assert_eq!(g.index_of(layout.hex_at(Pt::new(9000.0, 9000.0))), None);
```

Pointy-top and flat-top; `Offset` converts to the `(col, row)` a tilemap editor stores, which is
what you need the moment you load a map someone else authored. `FullGrid::hex_rect` builds the board
that file describes, in the same convention. This is the only floating point in the crate, and it
is a one-way street: no float reaches a cost, so pathfinding stays integer and stays reproducible.

## What it does

| | |
|---|---|
| `at(c)`, `index_of(c)` | a coordinate to an index — panicking, and `Option`, for when off the board is an answer |
| `coord(i)`, `coords_of(is)` | and back again, which is what you draw and what you save |
| `step(i, dir)` | one cell along, respecting holes — the primitive everything is built from |
| `neighbors(i)` | every neighbour, **with the direction that reaches it** |
| `in_neighbors(j)` | every cell that can step *into* `j` — the graph is directed, so this differs |
| `ray(i, dir)` | slide until the board runs out — rooks, bishops |
| `run(i, dir, same)` | the unbroken line **both ways** through a cell — line-of-N, wall segments |
| `offset(i, delta)` | a lattice hop that ignores what it flies over — knights, capture-by-jump |
| `distance`, `within`, `ring` | attack range, blast radius — priced by the radius, board scan only when that's cheaper |
| `line`, `los`, `visible_from` | field of view: walls actually block sight |
| `component`, `is_connected` | islands: did the generated map split, does this wall seal the room |
| `subset(cells)` | any cells you name, as a board — the highlight you draw and search |
| `to_root`, `of_root` | a region's index, as the board that owns the cells numbers it |
| `root_indices()` | a region as plain cells you can keep — a `SubGrid` borrows, so it cannot be stored |
| `path`, `reachable`, `path_toward` | A\* and budget-bounded Dijkstra, over your cost closure |
| `reaching(goal, …)` | who can reach *here* — a threat map, in one backward search |
| `center`, `hex_at`, `corners` | where a cell is on screen, and which cell the mouse is over |

Two distinctions carry their weight. `step` respects holes and `offset` does not — a rook cannot
slide through a gap in the board, but a knight leaps one, and so does a capture-by-jump. And
`neighbors` keeps the direction, which is what lets a draughts man move forward only.

## Nothing here is sized by a number you passed in

Every allocation and every loop in this crate is bounded by the size of the board, never by a cost,
a radius, or a coordinate you hand it. That is not decoration — it was learned the hard way.

Costs are summed into a total that **saturates** rather than wraps. Rust does not check integer
overflow in release, and a wrapped total makes a longer path look cheaper than a short one, which
destroys the invariant Dijkstra rests on: cells re-open forever, the heap grows without bound, and
the process eats memory until the machine dies. In release only — the build a game ships.

The same rule, everywhere: distances are computed in `i64` so they cannot wrap; `Coord::step`
saturates, so a cell at `i32::MAX` cannot wrap onto a real cell on the far side of the world and
forge an edge; `ray` is bounded by the board, because a wrapping coordinate makes the step table
cyclic; a range query counts its offsets before building them and scans the board instead when that
is cheaper; sight is capped, because raycasting is O(r³) and a big enough radius is a hang made of
time rather than memory. Grids are capped at `MAX_CELLS`.

`tests/robust.rs` is one test per bug, and it runs in release, because debug's overflow checks mask
exactly the bugs that only exist without them.

## An index knows which board issued it

`Idx` is a dense index, valid only within the board that issued it. `filtered` renumbers, a
`SubGrid` numbers its own cells from zero, and a stale index would otherwise quietly address a
*different cell*. No bounds check can find that: two boards of the same size number every cell in
range for both.

So an index carries its board, and a **debug build checks it**:

```rust
# use spacewalk::{Adjacency, FullGrid, Grid, Sq};
let board = FullGrid::square(8, 8, Adjacency::Four);
let dark = board.filtered(|c| (c.x + c.y) % 2 == 0);
let light = board.filtered(|c| (c.x + c.y) % 2 == 1);
assert_eq!(dark.len(), light.len());        // so no bound can tell them apart

let cell = dark.at(Sq::new(2, 2));
assert_eq!(dark.coord(cell), Sq::new(2, 2));

// `light.coord(cell)` panics in debug: "issued by a different grid".
// Shipped, it quietly answers Sq::new(3, 2).
```

In release the tag is zero-sized, the checks vanish, and an `Idx` is a bare `u32` again. Equality,
ordering, and hashing compare the number alone in **both** profiles, so nothing you observe changes
with the build — only whether you are told. It is a development aid, not a runtime guarantee:
*serialize coordinates, never indices.*

A subset makes this sharper than a filter does, because both numberings stay **live**: the region's
index 0 and the board's index 0 are each valid and each a different cell. `to_root` and `of_root`
are the bridge, and the only correct one.

Two boards that number the same cells the same way share a tag and are interchangeable — which is
what makes rebuilding from `cells()` restore the *same indices*, not merely an equivalent board.
`Idx::get` reads the number out for a structure of your own; there is deliberately no way back in.

## The metric must agree with the adjacency

An eight-way board measured with Manhattan distance is the classic tactics bug: a unit can *step* to
the enemy diagonally beside it, but measures it as two cells away and so cannot *attack* it.
`Adjacency` picks both together, so there is no second knob to get wrong — and `FullGrid::new`
checks the same thing per edge for a board you build yourself, so it panics rather than misbehaves.

## The tests are the examples

There is no `examples/` directory, on purpose: the acceptance tests **are** real games, built from
nothing but the public API. They are the code you actually want to read before you write any — each
one is a working thing, not a snippet. If a change to the core would make one of them impossible to
write, the change is wrong.

- [`square_tactics`](tests/square_tactics.rs) — turn-based grid tactics: four- and eight-way
  movement, corner rules, terrain
- [`checkers`](tests/checkers.rs) — dark squares only, forward-only men, jumps and chains
- [`hex_capture`](tests/hex_capture.rs) — clone-and-jump capture on a holed hex board
- [`hex_field`](tests/hex_field.rs) — a rectangular hex battlefield loaded from an authored map:
  terrain in a `CellMap`, an island check, and five in a row along three axes
- [`chess3d`](tests/chess3d.rs) — a coordinate this crate has never heard of, defined **in the test
  file**, with no change to the crate. Start here if your board is an odd shape.
- [`screen`](tests/screen.rs) — drawing the board and picking the cell under the mouse
- [`fov`](tests/fov.rs) — walls cast shadows, and sight is symmetric
- [`threat`](tests/threat.rs) — who can reach *this* cell, on a directed graph
- [`directional`](tests/directional.rs) — rivers, conveyors, one-way ledges
- [`admissible`](tests/admissible.rs) — A\* returns the *cheapest* path, not merely a path
- [`determinism`](tests/determinism.rs) — the same question, asked twice, gets the same answer
- [`save`](tests/save.rs) — what to serialize, and the one way to get a wrong answer out of this
  crate without hearing about it
- [`range_cost`](tests/range_cost.rs) — range queries priced by the radius, not the board, and the
  memory bomb the fix must not ship
- [`highlight`](tests/highlight.rs) — a turn's movement range and an attack, as boards you can
  draw and then search
- [`identity`](tests/identity.rs) — an index belongs to one board, and says so when it is handed
  to another
- [`robust`](tests/robust.rs) — one test per bug that could take a machine down

```sh
cargo test
cargo test --release          # robust's bugs hide behind debug's overflow checks; range_cost times the build you ship
cargo test --all-features     # adds serde
```

Run both profiles. The release build is where the index check is gone, and every answer must be the
same without it — only whether a mistake is reported may differ.

## Requirements

- Rust 1.88 or later. The crate uses edition 2024 and let-chains.
- The `serde` feature adds derives for the coordinate and grid types. It is off by default.

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE)
