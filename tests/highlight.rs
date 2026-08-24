//! Acceptance test: the ranges a tactics game paints on the board, and then reasons about.
//!
//! A player selects a unit. The game shades where it can walk. They hover an enemy, and the game
//! shades the blast. Both are sets of cells — but a set of cells is not what the game needs next,
//! because the very next question is always another board question: *route me through the shaded
//! area*, *is the shaded area split by that wall*, *what does the unit see from there*.
//!
//! So a range comes back as a [`SubGrid`], which is a board. The claim this file exists to prove is
//! that it is a board in the strong sense: it has its own edges, so a search inside a highlight
//! cannot leave it, and its own components, so an area split by a wall reports as split.
//!
//! The second claim is the one that bites in practice. The game's terrain lives in the game, keyed
//! by the *root* board's numbering. A region numbers its cells afresh. Every read of game data from
//! inside a region therefore goes through [`Grid::to_root`], and this file does it the way a game
//! would.

use spacewalk::{Adjacency, CellMap, Cost, FullGrid, Grid, Idx, Movement, Sq, Step, SubGrid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terrain {
    Open,
    Rough,
    Wall,
}

/// A 12 × 10 field with a wall down column 6, and a door in it at row 5.
///
/// The board is pure geometry, so the wall is not a hole in it — it is a value in the game's own
/// terrain map, exactly as a real game holds one. The grid never learns what a wall is.
fn field() -> (FullGrid<Sq>, CellMap<Terrain>) {
    let g = FullGrid::square(12, 10, Adjacency::Four);
    let terrain = CellMap::from_fn(&g, |c: Sq| match c {
        Sq { x: 6, y: 5 } => Terrain::Open, // the door
        Sq { x: 6, .. } => Terrain::Wall,
        Sq { y: 2..=3, .. } => Terrain::Rough,
        _ => Terrain::Open,
    });
    (g, terrain)
}

/// What it costs to walk on this board: rough ground is dear, a wall is impassable.
///
/// Scanned against `b`, never against something else. A `Movement` prices cells by index, and a
/// region numbers its cells differently — so one scanned on the root and used on a region would
/// price the wrong cells, in silence. Taking the board as a parameter is what makes that hard to
/// get wrong.
fn march<'a, B: Grid<Cell = Sq>>(
    b: &'a B,
    terrain: &'a CellMap<Terrain>,
) -> Movement<impl Fn(Step<Sq>) -> Option<Cost> + 'a> {
    Movement::scan(b, move |s: Step<Sq>| match terrain[b.to_root(s.to)] {
        Terrain::Open => Some(10),
        Terrain::Rough => Some(30),
        Terrain::Wall => None,
    })
}

#[test]
fn a_movement_range_is_a_board_a_route_cannot_leave() {
    // The player's turn: shade where the unit can go, then route it to a cell in the shading. The
    // route must stay in the shading — a path that leaves the highlight and comes back is a path
    // the player was never shown and cannot afford.
    let (g, terrain) = field();
    let unit = g.index_of(Sq::new(2, 5)).unwrap();

    let budget = 40;
    let reach = g.reachable(unit, budget, &march(&g, &terrain));
    let range: SubGrid<FullGrid<Sq>> = g.subset(reach.iter().map(|&(i, _)| i));

    assert!(
        range.len() > 1 && range.len() < g.len(),
        "some of the board"
    );

    // Inside the highlight, the unit is at index whatever-this-board-says. Ask again by coordinate.
    let from = range.index_of(Sq::new(2, 5)).unwrap();
    let far = range
        .indices()
        .max_by_key(|&i| range.distance(from, i))
        .unwrap();

    let route = range.path(from, far, &march(&range, &terrain)).unwrap();
    assert!(
        route.cost <= budget,
        "the shading promised this was affordable"
    );
    for &i in &route.steps {
        assert!(
            reach.iter().any(|&(r, _)| r == range.to_root(i)),
            "{:?} was walked through but never shaded",
            range.coord(i),
        );
    }
}

#[test]
fn a_region_reads_the_games_own_data_through_the_root() {
    // The bridge, doing the job it exists for. `terrain` is sized and keyed by the root board;
    // `blast` numbers its own three-by-three afresh. Subscripting one with the other's index is
    // the mistake, and `to_root` is the whole of the fix.
    let (g, terrain) = field();
    let centre = g.index_of(Sq::new(6, 4)).unwrap();
    let blast = g.within(centre, 0, 1);

    assert_eq!(
        terrain.len(),
        g.len(),
        "the map is sized by the whole board"
    );
    assert!(blast.len() < terrain.len());

    let walls = blast
        .indices()
        .filter(|&i| terrain[blast.to_root(i)] == Terrain::Wall)
        .count();
    assert_eq!(walls, 2, "the wall above the door, and the one below it");

    // And the other way: a cell the game already has a root index for, located in the region.
    assert_eq!(blast.of_root(centre), blast.index_of(Sq::new(6, 4)));
    assert_eq!(blast.of_root(g.index_of(Sq::new(0, 0)).unwrap()), None);
}

#[test]
fn an_area_of_effect_is_split_by_a_wall_that_runs_through_it() {
    // The question that needs the range to be a board rather than a list: a spell centred on the
    // wall covers cells on both sides, and the two sides are not one area. Answering it means
    // asking a *component* question about the highlight itself.
    let (g, terrain) = field();
    let centre = g.index_of(Sq::new(6, 1)).unwrap();

    let blast = g.within(centre, 0, 2);
    let open = |i: Idx| terrain[blast.to_root(i)] != Terrain::Wall;

    assert!(!blast.is_connected(open), "the wall runs through the blast");

    let west = blast.component(blast.index_of(Sq::new(4, 1)).unwrap(), open);
    let east = blast.component(blast.index_of(Sq::new(8, 1)).unwrap(), open);
    assert!(
        west.cells().all(|c| !east.contains(c)),
        "two separate areas"
    );
}

#[test]
fn a_region_of_a_region_still_maps_back_to_the_board_in_one_hop() {
    // Narrowing a highlight — the reachable cells that are also in sight, say — must not build a
    // chain of boards to walk back down. A subset of a subset is a subset of the board.
    let (g, terrain) = field();
    let eye = g.index_of(Sq::new(2, 5)).unwrap();

    let seen = g.visible_from(eye, 4, |i| terrain[i] == Terrain::Wall);
    let near = seen.subset(seen.indices().filter(|&i| seen.distance(0, i) <= 2));

    assert!(near.len() < seen.len());
    for i in near.indices() {
        // One hop, and it lands on the board — not on `seen`.
        assert_eq!(near.coord(i), g.coord(near.to_root(i)));
    }
}

#[test]
fn sight_stops_at_the_wall_but_range_does_not() {
    // The two range queries answer different questions and must not be confused. `within` measures
    // coordinates, so it reaches past a wall — that is what lets an archer shoot over one.
    // `visible_from` walks the line, so it does not.
    let (g, terrain) = field();
    let eye = g.index_of(Sq::new(4, 5)).unwrap();
    let blocked = |i: Idx| terrain[i] == Terrain::Wall;

    let range = g.within(eye, 0, 4);
    let sight = g.visible_from(eye, 4, blocked);

    assert!(range.contains(Sq::new(4, 1)), "four north, over the rough");
    assert!(sight.contains(Sq::new(4, 1)), "and nothing is in the way");

    assert!(range.contains(Sq::new(7, 4)), "past the wall, but in range");
    assert!(!sight.contains(Sq::new(7, 4)), "and the wall hides it");

    // Both are boards over the same cells, so the metric agrees with the root's.
    let (a, b) = (
        range.index_of(Sq::new(4, 5)).unwrap(),
        range.index_of(Sq::new(4, 1)).unwrap(),
    );
    assert_eq!(
        range.distance(a, b),
        g.distance(eye, g.index_of(Sq::new(4, 1)).unwrap())
    );
}
