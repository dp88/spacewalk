# Changelog

All notable changes to this project are documented in this file.

## Unreleased

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
