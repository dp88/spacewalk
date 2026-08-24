//! Acceptance test: a rectangular hex battlefield, loaded from an authored map.
//!
//! This is the shape a hex tactics game actually ships: not a hexagon, but a `w × h` field that
//! someone drew in a tile map editor and saved as `(col, row)`. The whole of it is built from the
//! public API — [`FullGrid::hex_rect`] for the board, [`CellMap`] for the terrain that hangs off it,
//! [`FullGrid::is_connected`] for the check that the map is playable, and [`FullGrid::run`] for a win
//! condition that reads a line across the lattice.
//!
//! Three claims are worth making here, and each one is a question a real game asks:
//!
//! - **The board is the map.** Cell `(col, row)` of the file is one cell here, and that cell says
//!   the same `(col, row)` back. Get this wrong and the terrain loads into the wrong places, with
//!   no error and no crash — the map is simply subtly not the one that was drawn.
//! - **Is the map playable?** A locked door can seal half a field off. That is a design mistake on
//!   a hand-drawn map and a routine accident on a generated one, and it is an *unweighted*
//!   question: no costs, no budget, no route.
//! - **Is that five in a row?** A line on a hex lattice runs along three axes, not four, and the
//!   check must walk both ways from the cell being considered — before anything is placed there.

use spacewalk::{
    CellMap, Coord, Cost, Dir6, FullGrid, Grid, Hex, Idx, Movement, Offset, Path, Step,
};

// ---------------------------------------------------------------------------------------------
// The map, as an editor would have saved it
// ---------------------------------------------------------------------------------------------

/// The staggering convention the editor wrote. Change this line and everything below still holds.
const CONVENTION: Offset = Offset::OddR;

/// `#` rock, `~` water, `+` a door, `.` open ground. Twelve columns, eight rows.
const MAP: [&str; 8] = [
    "............",
    "....~~......",
    "............",
    "#####+######",
    "............",
    "......~~~...",
    "............",
    "............",
];

const WIDTH: i32 = 12;
const HEIGHT: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tile {
    Floor,
    Water,
    Rock,
    Door,
}

impl Tile {
    /// What entering this tile costs, or `None` for a tile nothing may enter. Water is fordable and
    /// dear; the door is a tile whose answer depends on the state of the game rather than the map.
    fn cost(self, unlocked: bool) -> Option<Cost> {
        match self {
            Tile::Floor => Some(10),
            Tile::Water => Some(30),
            Tile::Rock => None,
            Tile::Door => unlocked.then_some(10),
        }
    }
}

/// The board, and the terrain that hangs off it. Two objects, deliberately: the grid is geometry
/// and holds no game state, and this is the game state.
fn field() -> (FullGrid<Hex>, CellMap<Tile>) {
    let g = FullGrid::hex_rect(WIDTH, HEIGHT, CONVENTION);
    let terrain = CellMap::from_fn(&g, |c: Hex| {
        let (col, row) = CONVENTION.from_hex(c);
        match MAP[row as usize].as_bytes()[col as usize] {
            b'#' => Tile::Rock,
            b'~' => Tile::Water,
            b'+' => Tile::Door,
            _ => Tile::Floor,
        }
    });
    (g, terrain)
}

/// The cell an authored `(col, row)` names.
fn at(g: &FullGrid<Hex>, col: i32, row: i32) -> Idx {
    g.index_of(CONVENTION.to_hex(col, row))
        .expect("the map's own coordinates are on the board")
}

// ---------------------------------------------------------------------------------------------
// The board is the map
// ---------------------------------------------------------------------------------------------

#[test]
fn every_cell_carries_the_terrain_the_editor_drew_there() {
    // The loader, checked against the file rather than against itself. A round-trip among our own
    // numbers passes happily when both directions are wrong the same way.
    let (g, terrain) = field();
    assert_eq!(g.len(), (WIDTH * HEIGHT) as usize);

    for (row, line) in MAP.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let i = at(&g, col as i32, row as i32);
            let drawn = match ch {
                b'#' => Tile::Rock,
                b'~' => Tile::Water,
                b'+' => Tile::Door,
                _ => Tile::Floor,
            };
            assert_eq!(terrain[i], drawn, "({col}, {row})");
        }
    }
}

#[test]
fn the_field_is_a_hex_lattice_and_not_a_square_one() {
    // Worth pinning: a rectangle of hexes is still hexes. An interior cell has six neighbours, and
    // the staggering is what puts them there.
    let (g, _) = field();
    assert_eq!(g.neighbors(at(&g, 5, 4)).count(), 6);
    assert_eq!(g.distance(at(&g, 5, 4), at(&g, 6, 4)), 1);
}

// ---------------------------------------------------------------------------------------------
// Is the map playable?
// ---------------------------------------------------------------------------------------------

/// Whether a unit may stand on a cell at all. The door is the only tile that moves.
fn passable<'a>(terrain: &'a CellMap<Tile>, unlocked: bool) -> impl Fn(Idx) -> bool + 'a {
    move |i| terrain[i].cost(unlocked).is_some()
}

#[test]
fn a_locked_door_leaves_the_two_halves_of_the_field_islands() {
    // The check a map wants before it is ever played: a wall of rock runs the width of the field,
    // and the only way through it is shut. This is unweighted — no route is computed, and no cost
    // is consulted.
    let (g, terrain) = field();
    let open = passable(&terrain, false);

    assert!(!g.is_connected(&open), "the south half is cut off");

    let north = g.component(at(&g, 0, 0), &open);
    let south = g.component(at(&g, 0, 7), &open);

    assert_eq!(north.len(), 36, "three rows of twelve");
    assert_eq!(south.len(), 48, "four rows of twelve");
    assert!(north.cells().all(|c| !south.contains(c)), "and disjoint");
}

#[test]
fn unlocking_the_door_makes_the_field_one_room() {
    let (g, terrain) = field();
    assert!(g.is_connected(passable(&terrain, true)));
}

#[test]
fn the_only_route_south_goes_through_the_door() {
    // And now the weighted question, which is a different one. The unweighted check above says a
    // route exists; this one says what it costs and where it runs.
    let (g, terrain) = field();
    let route = |unlocked| -> Option<Path> {
        let m = Movement::scan(&g, |s: Step<Hex>| terrain[s.to].cost(unlocked));
        g.path(at(&g, 0, 0), at(&g, 0, 7), &m)
    };

    assert!(route(false).is_none(), "no way through the rock");

    let through = route(true).expect("the door is open");
    assert!(
        through.steps.contains(&at(&g, 5, 3)),
        "every route south uses the one door"
    );
}

#[test]
fn a_ford_is_walked_round_when_it_is_dearer_than_the_detour() {
    // Terrain has to actually cost something, or the map is decoration. The water in row 5 is three
    // times open ground, so a unit crossing row 5 steps around it.
    let (g, terrain) = field();
    let m = Movement::scan(&g, |s: Step<Hex>| terrain[s.to].cost(true));

    let crossing = g.path(at(&g, 4, 5), at(&g, 10, 5), &m).expect("a way east");
    assert!(
        crossing.steps.iter().all(|&i| terrain[i] != Tile::Water),
        "it went round the ford"
    );
}

// ---------------------------------------------------------------------------------------------
// Is that five in a row?
// ---------------------------------------------------------------------------------------------

/// A hex lattice has three axes, not four. Each is one direction; `run` walks both ways along it.
const AXES: [Dir6; 3] = [Dir6::E, Dir6::Ne, Dir6::Nw];

/// Would placing a stone on `i` complete a line of five? The board is never written to — `run`
/// takes `i` as its anchor and does not ask the predicate about it.
fn wins(g: &FullGrid<Hex>, mine: &CellMap<bool>, i: Idx) -> bool {
    AXES.iter().any(|&d| g.run(i, d, |j| mine[j]).len() >= 5)
}

/// Stones on the cells named in offset coordinates.
fn stones(g: &FullGrid<Hex>, placed: &[(i32, i32)]) -> CellMap<bool> {
    let mut mine = CellMap::new(g, false);
    for &(col, row) in placed {
        mine[at(g, col, row)] = true;
    }
    mine
}

#[test]
fn filling_the_gap_in_a_row_of_four_wins() {
    let (g, _) = field();
    let mine = stones(&g, &[(2, 6), (3, 6), (5, 6), (6, 6)]);

    assert!(wins(&g, &mine, at(&g, 4, 6)), "two pairs joined into five");
    assert!(!wins(&g, &mine, at(&g, 7, 6)), "that only makes three");
    assert!(
        !mine[at(&g, 4, 6)],
        "and the winning move was never actually played"
    );
}

#[test]
fn a_line_runs_along_every_hex_axis_not_merely_along_the_rows() {
    // The row case above would pass on a square grid. This one would not: the other two axes cut
    // across the staggering, which is where an offset conversion of the wrong sign shows itself.
    let (g, _) = field();
    let centre = at(&g, 5, 4);

    for axis in AXES {
        let mut mine = CellMap::new(&g, false);
        let mut c = g.coord(centre);
        for _ in 0..2 {
            c = c.step(axis);
            mine[g.index_of(c).expect("still on the field")] = true;
        }
        let mut c = g.coord(centre);
        for _ in 0..2 {
            c = c.step(axis.opposite());
            mine[g.index_of(c).expect("still on the field")] = true;
        }

        assert!(wins(&g, &mine, centre), "{axis:?}");
        assert_eq!(g.run(centre, axis, |j| mine[j]).len(), 5, "{axis:?}");
    }
}

#[test]
fn a_line_stops_at_the_edge_of_the_field_rather_than_wrapping_onto_the_next_row() {
    // A staggered rectangle is stored row by row, and the trap is a line that walks off the east
    // edge and reappears in the west of the same row. Three stones wait at the west end for it to
    // do exactly that. It does not: `run` walks the lattice, and the lattice ends.
    let (g, _) = field();
    let mine = stones(&g, &[(0, 6), (1, 6), (2, 6), (9, 6), (10, 6)]);

    let line = g.run(at(&g, 11, 6), Dir6::E, |j| mine[j]);
    assert_eq!(line, vec![at(&g, 9, 6), at(&g, 10, 6), at(&g, 11, 6)]);
    assert!(!wins(&g, &mine, at(&g, 11, 6)), "three is not five");
}
