![spacewalk banner](art/banner.webp)

# spacewalk

[![CI](https://github.com/dp88/spacewalk/actions/workflows/ci.yml/badge.svg)](https://github.com/dp88/spacewalk/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/spacewalk.svg)](https://crates.io/crates/spacewalk)
[![docs.rs](https://img.shields.io/docsrs/spacewalk)](https://docs.rs/spacewalk)
![MSRV](https://img.shields.io/badge/rust-1.88%2B-blue)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

The geometry instrument for operations conducted on a lattice, such as board
games, tactics games, and tile maps: cells, directions, distance, sight,
regions, and routes. Pieces, terrain, players, and the game clock remain under
the command of your application.

## 🚀 Quick start

```toml
[dependencies]
spacewalk = "0.3"
```

```rust
use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};

let grid = FullGrid::square(8, 8, Adjacency::Four);
let mud = [Sq::new(3, 3), Sq::new(3, 4)];

let movement = Movement::cell_cost(&grid, |cell| {
    Some(if mud.contains(&cell) { 30 } else { 10 })
});

let (from, to) = (grid.at(Sq::new(0, 0)), grid.at(Sq::new(7, 7)));
let route = grid
    .path(from, to, &movement)
    .expect("the route should be open");

assert_eq!(route.len(), 14);
assert_eq!(route.cells(&grid).count(), 15); // the start is included
```

## 👨‍🚀 Mission Objectives

- Code written against the `Grid` trait runs on a board you built
  (`FullGrid`) or on a borrowed region of one (`SubGrid`) without a change of
  course.
- Square, hex, disc, and tile-map hex rectangles ship as constructors. A
  custom `Coord` implementation opens anything else, including
  three-dimensional boards.
- A step's cost belongs to the cell entered and the direction of arrival, so
  a river, a conveyor, or a one-way ledge needs no special case. A*,
  budget-bounded Dijkstra, and reverse reachability search the graph with
  deterministic tie-breaking. A `Graph` runs the same searches over nodes you
  name yourself, from a plain list of edges, with no grid at all.
- Range, ring, component, and field-of-view queries return a `SubGrid`, so
  the thing you highlight is also the thing you path over.
- Elevation lives in a `CellMap` beside your terrain. `height::height_gate`
  says what a hill hides and `height::climb_gate` what a ledge refuses, and
  both compose with the closures you already pass.
- Pathfinding is integer arithmetic, so a replay reproduces. Floats exist
  only in the screen-layout layer, and no float ever reaches a cost, a
  metric, or a step.

## 👨‍🚀 Requirements and features

- Rust 1.88 or newer, edition 2024.
- `#![no_std]` with `alloc`; one runtime dependency (`hashbrown`).
- `serde` feature: serialization for the plain-data types and `CellMap`.
  Grids rebuild from their cells; serialize coordinates, never indices.
- Safety caps bound untrusted map files: `MAX_CELLS` is 2²⁴ cells and
  `MAX_SIGHT` is 64. Panicking constructors have `try_` counterparts.

## 🧑‍🚀 More examples and documentation

- [API documentation](https://docs.rs/spacewalk): rustdoc is the manual, with
  the `Grid` vocabulary, cost models, metrics, index identity, and layouts.
- [`tests/`](tests/): public-API missions, from square tactics, checkers, hex
  capture, and three-dimensional chess to field of view, directed threats,
  save and load, and robustness at hostile numeric limits.
- [CHANGELOG](CHANGELOG.md)
- [Issue tracker](https://github.com/dp88/spacewalk/issues)

## 👩‍⚖️ License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

*Banner image: “Astronaut Walks in Space,” credited to the U.S. Information
Agency; [source via Artvee](https://artvee.com/dl/astronaut-walks-in-space).*
