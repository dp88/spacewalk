# Changelog

All notable changes to this project are documented in this file.

## Unreleased

## 0.3.0 — 2026-09-17

- **Breaking:** `FullGrid` no longer stores a step table or a reverse table. It
  keeps the coordinates and the index over them, and computes each step from
  the coordinate. A stored board drops from about 134 bytes a cell to about 34
  (eight-way) and builds about 1.4 times faster. Every query returns the same
  answer. The price is search speed: `path`, `reachable`, and `reaching` run
  40% to 60% slower than in 0.2.1 on every board measured, from 16×16 to
  512×512. Sight queries are unchanged.
- **Breaking:** `Coord::step` must move every cell of a board by one offset, as
  the coordinate's own `Sub` measures it. The grid finds who can step into a
  cell by undoing the step, so it can no longer hold a step that leads two
  cells into one, or a portal. `FullGrid::new` checks every edge and refuses
  such a board with the new `GridError::StepNotInvertible`. A step that clamps
  at the edge of the board, and a world whose `Add` and `Sub` wrap, still work.
- **Breaking:** `GridError::TooManyEdges` is gone. With no table of edges there
  is no edge limit to pass.
- **Breaking:** `RectGrid` is gone. It existed so that a large rectangle could
  avoid the tables that `FullGrid` no longer has. Use
  `FullGrid::square(w, h, adjacency)`: it gives the same cells and the same
  indices. It holds about 34 bytes a cell, where `RectGrid` held nothing.
- **Breaking:** the `Grid::Root` associated type is gone, because every root
  is now a `FullGrid`. `Grid::root` returns `&FullGrid<Self::Cell>`, and a
  region is `SubGrid<'_, C>` over its coordinate type, where it was
  `SubGrid<'_, FullGrid<C>>`.
- **Breaking:** the `Grid` trait is sealed. No outside crate could implement
  it over storage of its own in any case, because only this crate mints an
  `Idx`; the seal states that in the type system, and frees the trait to grow.
  `Grid::tag` moves to the private supertrait, and `Tag` is no longer public.
- **Breaking:** the `Lerp` alias is gone. `Metric::with_lerp` takes the same
  function pointer, written out: `fn(a: C, b: C, t: u32, n: u32) -> C`.
- **Breaking:** the crate root no longer re-exports the screen layout types or
  the rule gates. Import `HexLayout`, `SqLayout`, `Pt`, and `Orientation` from
  `spacewalk::layout`; `corner_gate` and `CornerRule` from `spacewalk::square`;
  `height_gate` and `climb_gate` from `spacewalk::height`. Those paths have
  always worked, so a consumer can move to them before it upgrades. `Offset`
  stays at the root, because `FullGrid::hex_rect` takes one.
- **Breaking:** the seven coordinate twins are gone from `Grid`: `step_from`,
  `within_cell`, `visible_from_cell`, `component_from`, `path_between`,
  `reachable_from`, and `reaching_cell`. Each was `index_of`, one call, and a
  map back. Indices go in through `at` and `index_of`; coordinates come out
  through `coord`, `cells`, and `coords_of`. So `g.path_between(a, b, &m)`
  becomes `g.path(g.at(a), g.at(b), &m)`.
- **Breaking:** `SubGrid::indices_in_root` is gone. It was a second name for
  `SubGrid::root_indices`.
- **Breaking:** the `prelude` module is gone. The crate root is now the short
  list that the prelude was, so import from it by name. Remember `Grid`: it is
  a trait, and it must be in scope before a method of a board can be called.

## 0.2.1 — 2026-09-17

- `Graph<K>` and `GraphError`: pathfinding with no grid. A graph is built from
  a plain list of `(from, to, cost)` edges over nodes named by a type of your
  own. It answers `path`, `reachable`, and `reaching` with the same search
  engine, the same `Idx` and `Path` types, and the same tie-breaking as a
  `Grid`. It needs no `Coord`, no directions, and no metric.
- The `Grid` trait documentation no longer says that an outside crate can
  implement it over storage of its own. Only this crate mints an `Idx`, so the
  trait is a bound to write code against, and `Coord` is the extension point.
  The stale method counts in the same documentation are gone.
- The `GridError::MetricDisagrees` message no longer holds a stray `+` and a
  run of spaces, which a broken line continuation had left in it.

## 0.2.0 — 2026-08-27

- `Sight`, and the `Grid::los_by` and `Grid::visible_from_by` queries that take
  a predicate over it. A blocker is now told who is looking and what they are
  looking at, so a rule that depends on the target can be expressed at all.
  `los` and `visible_from` keep their signatures as compatibility wrappers.
- `height` module: `height_gate` for what a hill hides, `climb_gate` for what a
  ledge refuses. Heights stay in a `CellMap` the application owns. Both gates
  are integer throughout, and the sight comparison is computed in `i128` so a
  hostile height field cannot wrap it.
- When its metric supports lines, `Grid::line` now always contains its own
  endpoints. A coordinate past the lattice limit cannot be rounded back to, so
  an endpoint was dropped from its own line: `los` then skipped a real blocker
  in the eye's place and became one-sided, and on a dense board it returned an
  empty line, which blocks nothing at all. Cells beyond the limit are still
  missed in between, which `line` now documents.

## 0.1.0 — 2026-08-24

Initial release.

- `Grid` trait vocabulary over three board types: `FullGrid` for any stored
  cell set, `RectGrid` for arithmetic rectangles, `SubGrid` for borrowed
  regions with their own numbering.
- Square, disc, hexagon, and tile-map hex-rectangle constructors; custom
  boards through the `Coord` trait.
- Directed movement costs per entered cell and arrival direction; A*,
  budget-bounded Dijkstra, reverse reachability, and `path_toward` with
  deterministic tie-breaking.
- Metrics with range, ring, line, line-of-sight, and bounded field-of-view
  queries; regions return as boards.
- `CellMap<T>` per-cell storage keyed by guarded dense indices; debug builds
  check board identity.
- `HexLayout`, `SqLayout`, `Offset`, and `Pt` for drawing and picking;
  floating point confined to the layout boundary.
- `#![no_std]` with `alloc`; optional `serde` feature for plain-data types
  and `CellMap`.
- Safety caps: `MAX_CELLS`, `MAX_SIGHT`, and `try_` constructors for
  recoverable failures.
