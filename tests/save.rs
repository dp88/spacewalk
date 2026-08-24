//! Saving a game: **serialize coordinates, never indices.**
//!
//! The crate says this in three places and, until now, proved it nowhere. It is the one way left to
//! get a wrong answer out of `spacewalk` without hearing about it, so it deserves a test rather
//! than a paragraph.
//!
//! An [`Idx`] is a dense `u32` handed out by one particular grid. It is not an address. Two grids
//! over the same cells may number them differently, and [`FullGrid::filtered`] renumbers outright — so a
//! saved index does not merely go stale, it may quietly come back pointing at a **different cell**.
//! No panic, no error: your archer is simply standing somewhere else when the save loads.
//!
//! A coordinate means the same thing forever. Save those.
//!
//! Run with `cargo test --features serde`.

#![cfg(feature = "serde")]

use spacewalk::{Adjacency, Dir6, Dir8, FullGrid, Grid, Hex, Sq};

#[test]
fn coordinates_round_trip() {
    let cells = [Sq::new(0, 0), Sq::new(-7, 12), Sq::new(i32::MIN, i32::MAX)];
    for c in cells {
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Sq>(&json).unwrap(), c, "{json}");
    }

    for h in [
        Hex::new(0, 0),
        Hex::new(3, -5),
        Hex::new(i32::MAX, i32::MIN),
    ] {
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Hex>(&json).unwrap(), h, "{json}");
    }
}

#[test]
fn directions_round_trip() {
    // Worth saving: a piece's facing, or a river's current.
    for d in Dir8::ALL {
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<Dir8>(&json).unwrap(), d, "{json}");
    }
    for d in Dir6::ALL {
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<Dir6>(&json).unwrap(), d, "{json}");
    }
}

#[test]
fn a_saved_index_from_before_a_filter_is_caught() {
    // THE hazard, made executable — and no longer silent.
    let full = FullGrid::square(3, 3, Adjacency::Four);

    // Our hero stands in the middle. We save his index, like a fool.
    let hero = full.at(Sq::new(1, 1));
    assert_eq!(hero.get(), 4);

    // The map changes — a cell is destroyed, flooded, whatever — and the grid is rebuilt.
    let after = full.filtered(|c| c != Sq::new(0, 0));

    // Index 4 is still a number in range, so no bound can object. What objects is the index
    // itself: it remembers which board numbered it, and `after` is not that board.
    if cfg!(debug_assertions) {
        let boom = std::panic::catch_unwind(|| after.coord(hero));
        let msg = *boom.unwrap_err().downcast::<String>().unwrap();
        assert!(msg.contains("different grid"), "{msg}");
    } else {
        // Shipped, the tag is gone and index 4 is simply index 4 — our hero has quietly moved one
        // square east, and the first anyone knows is a player saying the save file is haunted.
        // This is why the rule is *serialize coordinates*: the check is a development aid, not a
        // guarantee to lean on at runtime.
        assert_eq!(after.coord(hero), Sq::new(2, 1));
    }
}

#[test]
fn an_out_of_range_stale_index_at_least_gets_caught() {
    // The other half, and the only half the crate *can* catch. If the board shrank past your stale
    // index, it is no longer a number in range, and `assert_cell` says so by name.
    //
    // This is the half that is caught in **every** profile, tag or no tag: the board shrank past
    // the stale index, so it is no longer a number in range. The test above covers the other half.
    let full = FullGrid::square(3, 3, Adjacency::Four);
    let corner = full.at(Sq::new(2, 2)); // 8, the last cell
    let after = full.filtered(|c| c != Sq::new(0, 0)); // now only 8 cells: 0..=7

    let boom = std::panic::catch_unwind(|| after.coord(corner));
    let msg = *boom.unwrap_err().downcast::<String>().unwrap();
    if cfg!(debug_assertions) {
        assert!(msg.contains("different grid"), "{msg}");
        return;
    }
    assert!(msg.contains("not on this grid"), "{msg}");
    assert!(
        msg.contains("filtered"),
        "and it names the likely culprit: {msg}"
    );
}

#[test]
fn a_saved_coordinate_survives_the_same_change() {
    // The right way, and it is barely more work: save what the cell *is*, not where it sat in a
    // list. Look it up again on load.
    let full = FullGrid::square(3, 3, Adjacency::Four);

    let hero = full.at(Sq::new(2, 2));
    let saved = serde_json::to_string(&full.coord(hero)).unwrap();

    let after = full.filtered(|c| c != Sq::new(0, 0));

    let loaded: Sq = serde_json::from_str(&saved).unwrap();
    let rehomed = after.index_of(loaded).expect("the cell still exists");

    assert_eq!(
        after.coord(rehomed),
        Sq::new(2, 2),
        "still exactly where we left him"
    );
    assert_ne!(
        rehomed, hero,
        "under a different index, which is the whole point"
    );
}

#[test]
fn a_coordinate_that_no_longer_exists_says_so() {
    // And the case the index form cannot even detect: the cell we saved is *gone*. A coordinate
    // lookup returns None and the game can decide what to do. A stale index would have silently
    // handed back whichever cell inherited that slot.
    let full = FullGrid::square(3, 3, Adjacency::Four);
    let doomed = full.coord(full.at(Sq::new(0, 0)));

    let after = full.filtered(|c| c != Sq::new(0, 0));

    let saved = serde_json::to_string(&doomed).unwrap();
    let loaded: Sq = serde_json::from_str(&saved).unwrap();

    assert_eq!(
        after.index_of(loaded),
        None,
        "the cell is gone, and we are told so"
    );
}

// ---------------------------------------------------------------------------------------------
// Restoring the grid itself
// ---------------------------------------------------------------------------------------------

#[test]
fn a_grid_is_rebuilt_from_its_definition_not_deserialized() {
    // There is no `Grid: Serialize`, on purpose. A grid holds function pointers (its Metric), which
    // have no honest serialized form — and everything else in it is *derived*. Storing the step
    // table would be megabytes of data you can regenerate in microseconds, and it would go stale the
    // moment the crate's internals changed.
    //
    // So you save the definition. For a shipped shape that is three numbers.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Board {
        w: i32,
        h: i32,
        adjacency: Adjacency,
    }

    let saved = serde_json::to_string(&Board {
        w: 8,
        h: 8,
        adjacency: Adjacency::Eight,
    })
    .unwrap();
    let b: Board = serde_json::from_str(&saved).unwrap();
    let g = FullGrid::square(b.w, b.h, b.adjacency);

    let original = FullGrid::square(8, 8, Adjacency::Eight);
    assert_eq!(g.len(), original.len());
    assert_eq!(
        g.cells().collect::<Vec<_>>(),
        original.cells().collect::<Vec<_>>(),
        "cell for cell, in the same order"
    );
}

#[test]
fn rebuilding_from_cells_restores_the_same_indices_not_merely_an_equal_board() {
    // The property that makes the whole approach work, and it is stronger than it looks.
    //
    // `FullGrid::new` numbers cells in the order it is handed them. So round-tripping `cells()` — which
    // comes back in index order — through `FullGrid::new` reproduces the SAME INDICES, not just an
    // equivalent board. A holed, irregular, hand-built board restores exactly.
    use spacewalk::{Dir8, Metric};

    let original = FullGrid::square(6, 6, Adjacency::Eight)
        .filtered(|c| (c.x + c.y) % 3 != 0) // something irregular, with holes
        .filtered(|c| c != Sq::new(5, 5));

    // The save file: just the cells. The directions and the metric are code, not data.
    let saved = serde_json::to_string(&original.cells().collect::<Vec<_>>()).unwrap();
    let cells: Vec<Sq> = serde_json::from_str(&saved).unwrap();

    let restored = FullGrid::new(cells, &Dir8::ALL, Metric::CHEBYSHEV);

    assert_eq!(restored.len(), original.len());
    for i in original.indices() {
        assert_eq!(
            restored.coord(i),
            original.coord(i),
            "index {i} means the same cell"
        );
    }

    // And the geometry that was derived from those cells came back identical too — the holes are
    // still holes, and the steps into them are still dead ends.
    for i in original.indices() {
        let a: Vec<_> = original.neighbors(i).collect();
        let b: Vec<_> = restored.neighbors(i).collect();
        assert_eq!(a, b, "the neighbourhood of {i} survived");
    }
}

#[test]
fn a_cell_map_survives_only_when_the_cells_that_fix_its_order_are_saved_with_it() {
    // A CellMap is the one thing here you may save that is keyed by index, and this is the rule
    // that makes it safe. The map is a list in index order; `cells()` is what fixes that order. Save
    // both and the pair restores exactly. Save the map alone, against a board you regenerate some
    // other way, and every value slides to a different cell — silently, as ever.
    use spacewalk::{CellMap, Dir8, Metric};

    let g = FullGrid::square(5, 5, Adjacency::Four).filtered(|c| c != Sq::new(2, 2));
    let height = CellMap::from_fn(&g, |c: Sq| c.x * 10 + c.y);

    let saved = serde_json::to_string(&(g.cells().collect::<Vec<_>>(), &height)).unwrap();
    let (cells, loaded): (Vec<Sq>, CellMap<i32>) = serde_json::from_str(&saved).unwrap();

    let restored = FullGrid::new(cells, &Dir8::ORTHO, Metric::MANHATTAN);
    for i in restored.indices() {
        let c = restored.coord(i);
        assert_eq!(loaded[i], c.x * 10 + c.y, "index {i} still means {c:?}");
    }
}

#[test]
fn a_whole_game_state_round_trips_through_coordinates() {
    // What a save file actually looks like: the grid is rebuilt from its own definition, and the
    // game's state is keyed by coordinate. The grid itself is never serialized — it is geometry,
    // and geometry is cheaper to rebuild than to store.
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Save {
        walls: Vec<Sq>,
        units: Vec<(Sq, u8)>,
    }

    let g = FullGrid::square(8, 8, Adjacency::Eight);
    let before = Save {
        walls: vec![Sq::new(3, 3), Sq::new(3, 4)],
        units: vec![(Sq::new(0, 0), 1), (Sq::new(7, 7), 2)],
    };

    let json = serde_json::to_string(&before).unwrap();
    let after: Save = serde_json::from_str(&json).unwrap();
    assert_eq!(before, after);

    // And it is usable against a freshly built grid, which is the actual test.
    use spacewalk::Movement;
    let walls = after.walls.clone();
    let m = Movement::scan(&g, |s| (!walls.contains(&g.coord(s.to))).then_some(10));

    let from = g.at(after.units[0].0);
    let to = g.at(after.units[1].0);
    assert!(
        g.path(from, to, &m).is_some(),
        "the loaded game still plays"
    );
}
