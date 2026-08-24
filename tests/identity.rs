//! An index belongs to the board that issued it, and says so.
//!
//! This is the failure the crate used to call "the one way to get a wrong answer out of this crate
//! without hearing about it". A bounds check cannot find it: take two boards of the same size and
//! every index is in range for both, so nothing is out of place — the number is simply about a
//! different cell than the caller thinks.
//!
//! Each test below is one of the three shapes that bug takes, and each one now panics in a debug
//! build. The last two are the other side of the same coin: two boards that number the same cells
//! the same way *are* interchangeable, and must stay so, because saving and reloading a board
//! depends on it.
//!
//! The check is a `debug_assert`, so it is gone in release. That is the trade — a shipped game pays
//! nothing, and the mistake is caught while you are making it. The tests run in both profiles and
//! say what each one promises.

use spacewalk::{Adjacency, CellMap, FullGrid, Grid, Idx, Movement, RectGrid, Sq};

/// The panic message, or `None` if the call did not panic.
fn caught(f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = std::panic::catch_unwind(f);
    std::panic::set_hook(quiet);

    out.err().map(|e| match e.downcast::<String>() {
        Ok(s) => *s,
        Err(e) => (*e.downcast::<&str>().unwrap()).to_string(),
    })
}

/// The two halves of a chequerboard: same cell count, different cells, different numbering.
fn dark_and_light() -> (FullGrid<Sq>, FullGrid<Sq>) {
    let board = FullGrid::square(8, 8, Adjacency::Four);
    (
        board.filtered(|c| (c.x + c.y) % 2 == 0),
        board.filtered(|c| (c.x + c.y) % 2 == 1),
    )
}

#[test]
fn an_index_from_a_sibling_board_is_refused() {
    let (dark, light) = dark_and_light();
    assert_eq!(dark.len(), light.len(), "so no bound can tell them apart");

    let cell = dark.at(Sq::new(2, 2));
    let boom = caught(|| {
        light.coord(cell);
    });

    if cfg!(debug_assertions) {
        let msg = boom.expect("a foreign index must be refused");
        assert!(msg.contains("different grid"), "{msg}");
    } else {
        // What the check buys, stated as the bug it replaces: shipped, this is a different cell
        // and nobody is told.
        assert_ne!(light.coord(cell), Sq::new(2, 2));
    }
}

#[test]
fn a_cell_map_built_from_another_board_is_refused() {
    let (dark, light) = dark_and_light();

    let height = CellMap::from_fn(&light, |c: Sq| c.x);
    let cell = dark.at(Sq::new(2, 2));

    let boom = caught(|| {
        let _ = height[cell];
    });

    if cfg!(debug_assertions) {
        let msg = boom.expect("a map from the wrong board must be refused");
        assert!(msg.contains("different grid"), "{msg}");
    } else {
        assert_eq!(height[cell], light.coord(cell).x, "in range, and wrong");
    }
}

#[test]
fn a_regions_own_index_is_refused_by_its_root() {
    // The sharpest case, because both numberings are live at once: the region's cell 0 and the
    // board's cell 0 are each perfectly valid and each a different cell.
    let g = FullGrid::square(8, 8, Adjacency::Four);
    let region = g.within(g.at(Sq::new(4, 4)), 0, 2);

    let local = region.at(Sq::new(4, 4));
    let boom = caught(|| {
        g.coord(local);
    });

    if cfg!(debug_assertions) {
        let msg = boom.expect("a region's index must not address its root");
        assert!(msg.contains("different grid"), "{msg}");
    } else {
        assert_ne!(g.coord(local), Sq::new(4, 4));
    }

    // `to_root` is the bridge, and it is the only correct one.
    assert_eq!(g.coord(region.to_root(local)), Sq::new(4, 4));
}

#[test]
fn the_bridge_between_a_region_and_its_root_still_works_both_ways() {
    let g = FullGrid::square(8, 8, Adjacency::Four);
    let region = g.within(g.at(Sq::new(4, 4)), 0, 2);

    for i in region.indices() {
        let up = region.to_root(i);
        assert_eq!(region.of_root(up), Some(i), "and back down again");
        assert_eq!(g.coord(up), region.coord(i), "naming the same cell");
    }

    assert_eq!(
        g.of_root(g.at(Sq::new(0, 0))),
        Some(g.at(Sq::new(0, 0))),
        "a whole board is its own root",
    );
}

#[test]
fn a_rebuilt_board_keeps_its_predecessors_numbering() {
    // The property `tests/save.rs` rests on, and the reason a board's identity is derived from its
    // cells rather than from a counter. Two boards built from the same cells in the same order are
    // the same numbering, so an index travels between them — as it must, or reloading a save would
    // have to look every cell up again.
    let original = FullGrid::square(6, 6, Adjacency::Eight)
        .filtered(|c| (c.x + c.y) % 3 != 0)
        .filtered(|c| c != Sq::new(5, 5));

    let restored = FullGrid::new(
        original.cells().collect::<Vec<_>>(),
        original.dirs(),
        original.metric(),
    );

    for i in original.indices() {
        assert_eq!(restored.coord(i), original.coord(i), "index {i} survived");
    }
}

#[test]
fn a_rectangle_and_the_stored_board_of_the_same_shape_agree() {
    // The crate promises these two give "same answers, same indices". That is worth having only if
    // an index really does travel between them, so it is checked rather than asserted in prose.
    let stored = FullGrid::square(9, 7, Adjacency::Eight);
    let rect = RectGrid::new(9, 7, Adjacency::Eight);

    for i in stored.indices() {
        assert_eq!(rect.coord(i), stored.coord(i), "index {i} means one cell");
    }

    let from = stored.at(Sq::new(0, 0));
    let to = rect.at(Sq::new(8, 6));
    let walk = Movement::uniform(&rect, 1);
    assert_eq!(
        rect.path(from, to, &walk).unwrap().steps(),
        stored.path(from, to, &walk).unwrap().steps(),
        "including one taken from each",
    );
}

#[test]
fn an_index_can_be_read_but_not_forged() {
    // `Idx::get` is one-way on purpose. You may key your own structure with the number; you may not
    // hand a number back to a board and call it a cell. That asymmetry is the whole guarantee.
    let g = FullGrid::square(4, 4, Adjacency::Four);
    let i: Idx = g.at(Sq::new(2, 1));

    assert_eq!(i.get(), 6);
    assert_eq!(format!("{i}"), "6", "and it prints as the number it is");

    let mine = vec![0u8; g.len()];
    assert_eq!(mine[i.get() as usize], 0);
}

#[test]
fn equality_ignores_the_board_so_release_and_debug_agree() {
    // The tag must never reach `==`, `<`, or a hash. If it did, a `Vec<Idx>` would compare
    // differently between profiles — which is a far worse bug than the one the tag catches.
    let (dark, light) = dark_and_light();

    let a = dark.at(Sq::new(2, 2));
    let b = light.indices().nth(a.get() as usize).unwrap();

    assert_eq!(a, b, "same number, whatever board issued it");
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);

    let mut sorted: Vec<Idx> = dark.indices().collect();
    sorted.reverse();
    sorted.sort_unstable();
    assert_eq!(sorted, dark.indices().collect::<Vec<_>>());
}
