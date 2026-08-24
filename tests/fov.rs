//! Acceptance test: field of view — a wall you cannot see through.
//!
//! Before this, `ray()` was the only thing resembling sight, and it walks a *single direction*. On a
//! square grid there are eight of those, so it reached 40 of the 120 cells within sight radius 5:
//! eight spokes, and everything between them invisible. That is not a field of view, and no
//! roguelike could have used it.
//!
//! Real sight needs a straight line in *coordinate* space, which is lattice-specific — so it lives
//! on the [`Metric`], and the grid drives the walk one cell at a time.

use spacewalk::{Adjacency, FullGrid, Grid, Hex, Idx, MAX_SIGHT, Sq};

mod common;

/// Nothing blocks.
fn clear(_: Idx) -> bool {
    false
}

#[test]
fn a_line_starts_where_you_are_and_ends_where_you_look() {
    let g = FullGrid::square(8, 8, Adjacency::Eight);
    let a = g.at(Sq::new(0, 0));
    let b = g.at(Sq::new(5, 0));

    let cells: Vec<Sq> = g.line(a, b).iter().map(|&i| g.coord(i)).collect();
    assert_eq!(cells.first(), Some(&Sq::new(0, 0)));
    assert_eq!(cells.last(), Some(&Sq::new(5, 0)));
    assert_eq!(cells.len(), 6, "a straight run along the rank");
}

#[test]
fn a_line_to_yourself_is_just_you() {
    // The degenerate case, and a nasty one: the interpolation divides by the distance, so a line
    // from a cell to itself divided by zero, rounded the resulting NaN to 0, and returned the
    // ORIGIN of the coordinate space — a cell on the far side of the board.
    let g = FullGrid::square(8, 8, Adjacency::Eight);
    let me = g.at(Sq::new(6, 6));

    assert_eq!(g.line(me, me), vec![me]);
    assert!(g.los(me, me, clear), "you can see yourself");
}

#[test]
fn sight_is_symmetric() {
    // Rounding a tie has to break *somewhere*, and if it breaks by direction of travel you get a
    // board where A can see B but B cannot see A. Players notice, and they are right to.
    let g = FullGrid::square(15, 15, Adjacency::Eight);
    let walls = [Sq::new(7, 7), Sq::new(4, 9)];
    let blocks = |i| walls.contains(&g.coord(i));

    for a in g.indices() {
        for b in g.indices() {
            assert_eq!(
                g.los(a, b, blocks),
                g.los(b, a, blocks),
                "{:?} <-> {:?}",
                g.coord(a),
                g.coord(b)
            );
        }
    }
}

#[test]
fn a_line_reversed_is_the_reverse_of_the_line() {
    let g = FullGrid::square(12, 12, Adjacency::Eight);
    let a = g.at(Sq::new(1, 2));
    let b = g.at(Sq::new(9, 7));

    let there = g.line(a, b);
    let mut back = g.line(b, a);
    back.reverse();

    assert_eq!(there, back);
}

#[test]
fn a_wall_casts_a_shadow() {
    // The whole point. A wall due east of you hides what is behind it.
    let g = FullGrid::square(11, 11, Adjacency::Eight);
    let eye = g.at(Sq::new(5, 5));
    let wall = g.at(Sq::new(6, 5));
    let behind = g.at(Sq::new(8, 5));

    let blocks = |i| i == wall;

    assert!(g.los(eye, wall, blocks), "you see the wall itself");
    assert!(!g.los(eye, behind, blocks), "but not what stands behind it");
    assert!(
        g.los(eye, g.at(Sq::new(5, 8)), blocks),
        "and the rest is clear"
    );
}

#[test]
fn you_cannot_see_round_a_corner() {
    // An L-shaped corridor. The far arm is hidden until you reach the bend.
    let g = FullGrid::square(7, 7, Adjacency::Eight);
    let wall: Vec<Sq> = (0..7).map(|y| Sq::new(3, y)).filter(|c| c.y != 6).collect();
    let blocks = |i| wall.contains(&g.coord(i));

    let eye = g.at(Sq::new(0, 0));
    let beyond = g.at(Sq::new(6, 0));

    assert!(!g.los(eye, beyond, blocks), "the wall is in the way");
    assert!(
        g.los(eye, g.at(Sq::new(2, 0)), blocks),
        "up to the wall, fine"
    );
}

#[test]
fn field_of_view_sees_far_more_than_eight_spokes() {
    // The bug this replaces, quantified. Eight rays reached 40 of the 120 cells in radius 5.
    let g = FullGrid::square(21, 21, Adjacency::Eight);
    let eye = g.at(Sq::new(10, 10));

    let seen = g.visible_from(eye, 5, clear);
    let in_range = g.within(eye, 0, 5);

    assert_eq!(
        seen.len(),
        in_range.len(),
        "with nothing in the way, you see everything in range"
    );
    assert!(
        seen.len() > 100,
        "not the 40 that eight rays managed: {}",
        seen.len()
    );
}

#[test]
fn a_pillar_hides_what_is_behind_it_and_nothing_else() {
    let g = FullGrid::square(21, 21, Adjacency::Eight);
    let eye = g.at(Sq::new(10, 10));
    let pillar = g.at(Sq::new(12, 10));

    let seen = g.visible_from(eye, 6, |i| i == pillar);
    let open = g.visible_from(eye, 6, clear);

    assert!(seen.len() < open.len(), "the pillar hides something");
    assert!(seen.contains(g.coord(pillar)), "but not itself");
    assert!(
        !seen.contains(Sq::new(14, 10)),
        "and what is directly behind it is gone"
    );
}

#[test]
fn hexes_have_sight_too() {
    let g = FullGrid::hexagon(5);
    let eye = g.at(Hex::new(0, 0));
    let wall = g.at(Hex::new(1, 0));
    let behind = g.at(Hex::new(3, 0));

    let blocks = |i| i == wall;
    assert!(g.los(eye, wall, blocks));
    assert!(
        !g.los(eye, behind, blocks),
        "the hex wall casts a shadow too"
    );
}

#[test]
fn an_absurd_sight_radius_is_refused_rather_than_attempted() {
    // Raycasting is O(r^3). At r = 1000 that is the better part of a minute of solid compute — a
    // hang made of time rather than memory, but a hang. Same bug class as the one that ate 32GB.
    let g = FullGrid::square(30, 30, Adjacency::Eight);
    let eye = g.at(Sq::new(15, 15));

    let boom = std::panic::catch_unwind(|| g.visible_from(eye, 100_000, clear));
    let msg = *boom.unwrap_err().downcast::<String>().unwrap();
    assert!(msg.contains("MAX_SIGHT"), "it names the limit: {msg}");

    // And the limit itself is fine.
    let _ = g.visible_from(eye, MAX_SIGHT, clear);
}

#[test]
fn a_custom_coord_without_a_lerp_simply_has_no_sight() {
    // `Metric::scanning` has no notion of a straight line, and says so rather than guessing. A 3D
    // chess board does not want line of sight, and is not made to pretend it does.
    use spacewalk::{Coord, Metric};

    common::coord_1d!(P, E, |x| P(x.0 + 1));

    let g = FullGrid::new(
        (0..5).map(P),
        P::DIRS,
        Metric::scanning(|a: P, b: P| (b.0 - a.0).unsigned_abs()),
    );

    assert!(
        g.line(g.at(P(0)), g.at(P(4))).is_empty(),
        "no lerp, no line"
    );
}
