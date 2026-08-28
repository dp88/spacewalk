# Changelog

All notable changes to this project are documented in this file.

## Unreleased

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
