![spacewalk banner](art/banner.webp)

# spacewalk

[![CI](https://github.com/dp88/spacewalk/actions/workflows/ci.yml/badge.svg)](https://github.com/dp88/spacewalk/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/spacewalk.svg)](https://crates.io/crates/spacewalk)
[![docs.rs](https://img.shields.io/docsrs/spacewalk)](https://docs.rs/spacewalk)
![MSRV](https://img.shields.io/badge/rust-1.88%2B-blue)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

The geometry instrument for operations conducted on a lattice — board games,
tactics games, and tile maps: cells, directions, distance, sight, regions,
and routes. Pieces, terrain, players, and the game clock remain under the
command of your application.

## Quick start

```toml
[dependencies]
spacewalk = "0.1"
```

```rust
use spacewalk::{Adjacency, FullGrid, Grid, Movement, Sq};

let grid = FullGrid::square(8, 8, Adjacency::Four);
let mud = [Sq::new(3, 3), Sq::new(3, 4)];

let movement = Movement::cell_cost(&grid, |cell| {
    Some(if mud.contains(&cell) { 30 } else { 10 })
});

let route = grid
    .path_between(Sq::new(0, 0), Sq::new(7, 7), &movement)
    .expect("the route should be open");

assert_eq!(route.len(), 14);
assert_eq!(route.cells(&grid).count(), 15); // the start is included
```

## Why

- **One vocabulary, three boards.** Code written against the `Grid` trait
  runs on a stored cell set (`FullGrid`), an arithmetic rectangle
  (`RectGrid`), or a borrowed region (`SubGrid`) without a change of course.
- **Any shape.** Square, hex, disc, and tile-map hex rectangles ship as
  constructors; a custom `Coord` implementation opens anything else,
  including three-dimensional boards.
- **Directed movement costs.** A step's cost belongs to the cell entered and
  the direction of arrival — rivers, conveyors, and ledges just work. A*,
  budget-bounded Dijkstra, and reverse reachability search the graph with
  deterministic tie-breaking.
- **Sight and regions come back as boards.** Range, ring, component, and
  field-of-view queries return a `SubGrid`, so the thing you highlight is
  also the thing you path over.
- **Height is yours, and it still works.** Elevation belongs beside your
  terrain, in a `CellMap`, not in the grid. `height_gate` says what a hill
  hides and `climb_gate` what a ledge refuses, both composing with the
  closures you already pass.
- **Integer pathfinding, reproducible replays.** Floats exist only in the
  screen-layout layer; no float ever reaches a cost, metric, or step table.

## Requirements and features

- Rust 1.88 or newer, edition 2024.
- `#![no_std]` with `alloc`; one runtime dependency (`hashbrown`).
- `serde` feature: serialization for the plain-data types and `CellMap`.
  Grids rebuild from their cells; serialize coordinates, never indices.
- Safety caps bound untrusted map files: `MAX_CELLS` is 2²⁴ cells and
  `MAX_SIGHT` is 64. Panicking constructors have `try_` counterparts.

## More examples and documentation

- [API documentation](https://docs.rs/spacewalk) — rustdoc is the manual:
  the `Grid` vocabulary, cost models, metrics, index identity, and layouts.
- [`tests/`](tests/) — public-API missions: square tactics, checkers, hex
  capture, three-dimensional chess, field of view, directed threats,
  save/load, and robustness at hostile numeric limits.
- [CHANGELOG](CHANGELOG.md)
- [Issue tracker](https://github.com/dp88/spacewalk/issues)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

*Banner image: “Astronaut Walks in Space,” credited to the U.S. Information
Agency; [source via Artvee](https://artvee.com/dl/astronaut-walks-in-space).*
