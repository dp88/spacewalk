//! Acceptance test: **mouse picking**, and drawing the board it picks from.
//!
//! The other acceptance tests in this crate are games. This one is the thing every game does before
//! it is a game: put the board on the screen, and work out what the player clicked.
//!
//! Hex layout is the easiest place in this whole crate to be confidently, silently wrong. Every
//! published formula assumes axial `r` grows *south*; here it grows north-**east**. Code that copies
//! them round-trips perfectly, draws a plausible board, and puts the neighbours in the wrong places.
//! Nothing panics. So this file does not check the formulas — it checks the **properties a hexagon
//! actually has**, which is the only thing a wrong matrix cannot fake:
//!
//! - every cell picks itself back, from anywhere inside it
//! - all six neighbours are the same distance away (or it is not a hexagon)
//! - adjacent cells share exactly two corners (or it does not tile)
//! - the cell you pick is the cell whose centre is nearest (or it is not a Voronoi cell)
//! - offset coordinates land on the pixel a tilemap would draw them at (or interop is a lie)

use spacewalk::{
    Coord, Dir6, FullGrid, Grid, Hex, HexLayout, Offset, Orientation, Pt, Sq, SqLayout,
};

/// Pixels. Everything here is exact to a hundredth of one.
const EPS: f32 = 0.01;

fn close(a: Pt, b: Pt) -> bool {
    (a.x - b.x).abs() < EPS && (a.y - b.y).abs() < EPS
}

fn dist(a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (f64::from(a.x - b.x), f64::from(a.y - b.y));
    dx.hypot(dy)
}

/// Every hex within `r` of the origin.
fn hexes(r: i32) -> Vec<Hex> {
    (-r..=r)
        .flat_map(move |q| ((-r).max(-q - r)..=r.min(-q + r)).map(move |s| Hex::new(q, s)))
        .collect()
}

/// The layouts worth trying: both orientations, square and squashed cells, moved and not.
fn layouts() -> Vec<HexLayout> {
    vec![
        HexLayout::pointy(Pt::new(32.0, 32.0)),
        HexLayout::flat(Pt::new(32.0, 32.0)),
        HexLayout::pointy(Pt::new(16.0, 24.0)).at(Pt::new(-101.5, 42.25)),
        HexLayout::flat(Pt::new(24.0, 16.0)).at(Pt::new(640.0, 360.0)),
        // A negative height is a y-up camera, which is a legitimate thing to want.
        HexLayout::pointy(Pt::new(20.0, -20.0)),
    ]
}

// ---------------------------------------------------------------------------------------------
// The board is really a hexagon
// ---------------------------------------------------------------------------------------------

#[test]
fn all_six_neighbours_are_the_same_distance_away() {
    // If they are not, it is not a hexagon, whatever it round-trips like. This is the cheapest test
    // here and it is the one that catches a wrong constant in the forward matrix — a sign error can
    // survive a round-trip (the inverse simply undoes it) but it cannot survive this.
    for l in [
        HexLayout::pointy(Pt::new(32.0, 32.0)),
        HexLayout::flat(Pt::new(32.0, 32.0)),
    ] {
        let c = l.center(Hex::new(0, 0));
        let ds: Vec<f64> = Dir6::ALL
            .iter()
            .map(|&d| dist(c, l.center(Hex::new(0, 0).step(d))))
            .collect();

        for d in &ds {
            assert!(
                (d - ds[0]).abs() < 1e-6,
                "{:?}: neighbour distances differ: {ds:?}",
                l.orientation
            );
        }
        // Centre-to-centre is √3 · circumradius, which is what makes `size` the circumradius.
        assert!((ds[0] - 32.0 * 3f64.sqrt()).abs() < 1e-4, "{ds:?}");
    }
}

#[test]
fn the_compass_names_tell_the_truth_on_a_pointy_board() {
    // `Dir6`'s names are pointy-top names. Here is the board where they are literal.
    let l = HexLayout::pointy(Pt::new(10.0, 10.0));
    let o = l.center(Hex::new(0, 0));

    let at = |d: Dir6| {
        let p = l.center(Hex::new(0, 0).step(d));
        (p.x - o.x, p.y - o.y) // screen: +x east, +y SOUTH
    };

    let (ex, ey) = at(Dir6::E);
    assert!(
        ex > 1.0 && ey.abs() < EPS,
        "E must be due east, got ({ex}, {ey})"
    );
    let (wx, wy) = at(Dir6::W);
    assert!(wx < -1.0 && wy.abs() < EPS, "W must be due west");

    let (nex, ney) = at(Dir6::Ne);
    assert!(
        nex > 0.0 && ney < 0.0,
        "Ne must go up and right, got ({nex}, {ney})"
    );
    let (swx, swy) = at(Dir6::Sw);
    assert!(swx < 0.0 && swy > 0.0, "Sw must go down and left");
}

#[test]
fn a_flat_board_turns_the_compass_thirty_degrees_and_the_docs_say_so() {
    // The claim in `Orientation`'s docs, pinned. A flat-top hex has NO due-east neighbour — it has
    // a due-north and a due-south one — so under `Flat` the names are lattice labels, not compass
    // bearings. If this ever stops being true the docs have started lying.
    let l = HexLayout::flat(Pt::new(10.0, 10.0));
    let o = l.center(Hex::new(0, 0));
    let at = |d: Dir6| {
        let p = l.center(Hex::new(0, 0).step(d));
        (p.x - o.x, p.y - o.y)
    };

    let (nwx, nwy) = at(Dir6::Nw);
    assert!(
        nwx.abs() < EPS && nwy < 0.0,
        "under Flat it is Nw that points due NORTH: ({nwx}, {nwy})"
    );
    let (sex, sey) = at(Dir6::Se);
    assert!(
        sex.abs() < EPS && sey > 0.0,
        "and Se that points due SOUTH: ({sex}, {sey})"
    );

    // And `E`, which reads like a promise of east, is thirty degrees south of it.
    let (ex, ey) = at(Dir6::E);
    assert!(
        ex > 0.0 && ey > 0.0,
        "E renders east-SOUTH-east under Flat: ({ex}, {ey})"
    );
    assert!(
        (ey.atan2(ex).to_degrees() - 30.0).abs() < 0.01,
        "exactly 30°"
    );
}

#[test]
fn adjacent_cells_share_exactly_two_corners() {
    // The tiling proof, and the one people forget. Corners come from one formula and centres from
    // another; if they disagree by so much as a rotation the board still *looks* like hexes but the
    // edges do not meet, and you get seams. Two shared corners is what an edge IS.
    for l in layouts() {
        for h in hexes(3) {
            let mine = l.corners(h);
            for d in Dir6::ALL {
                let theirs = l.corners(h.step(d));
                let shared = mine
                    .iter()
                    .filter(|&&a| theirs.iter().any(|&b| close(a, b)))
                    .count();
                assert_eq!(
                    shared, 2,
                    "{:?} {h:?} vs {d:?}: shared {shared} corners",
                    l.orientation
                );
            }
        }
    }
}

#[test]
fn a_corner_is_one_circumradius_from_the_centre() {
    let l = HexLayout::pointy(Pt::new(32.0, 32.0));
    for h in hexes(2) {
        let c = l.center(h);
        for corner in l.corners(h) {
            assert!((dist(c, corner) - 32.0).abs() < 1e-3, "{h:?} {corner:?}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Picking
// ---------------------------------------------------------------------------------------------

#[test]
fn every_cell_picks_itself_back() {
    for l in layouts() {
        for h in hexes(20) {
            assert_eq!(l.hex_at(l.center(h)), h, "{:?} {h:?}", l.orientation);
        }
    }
}

#[test]
fn anywhere_inside_a_cell_picks_that_cell_not_merely_the_centre() {
    // Round-tripping centres is a weak test: a *transposed* inverse matrix can still send every
    // centre home on a symmetric lattice. Picking from off-centre cannot be faked. Walk most of the
    // way out towards each corner and each edge midpoint — still inside, so still this cell.
    for l in layouts() {
        for h in hexes(6) {
            let c = l.center(h);
            let corners = l.corners(h);

            for (i, &corner) in corners.iter().enumerate() {
                let edge = corners[(i + 1) % 6];
                let mid = Pt::new((corner.x + edge.x) / 2.0, (corner.y + edge.y) / 2.0);

                for t in [0.1f32, 0.5, 0.9] {
                    for target in [corner, mid] {
                        let p = Pt::new(
                            c.x + (target.x - c.x) * t * 0.97,
                            c.y + (target.y - c.y) * t * 0.97,
                        );
                        assert_eq!(
                            l.hex_at(p),
                            h,
                            "{:?} {h:?} at t={t} toward {target:?}",
                            l.orientation
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_cell_you_pick_is_the_cell_whose_centre_is_nearest() {
    // The definitive property, and the reason cube-rounding works at all: a hexagonal cell IS the
    // Voronoi cell of its centre. So picking is, by definition, nearest-centre — and here it is,
    // checked against brute force over a dense field of pixels. Any sign error in either matrix dies
    // here, in either direction, whether or not it round-trips.
    for l in [
        HexLayout::pointy(Pt::new(24.0, 24.0)),
        HexLayout::flat(Pt::new(24.0, 24.0)),
    ] {
        let board = hexes(10);

        let mut checked = 0;
        for i in -60i16..60 {
            for j in -60i16..60 {
                let p = Pt::new(f32::from(i) * 4.5, f32::from(j) * 4.5);

                let mut best = board[0];
                let mut best_d = f64::MAX;
                let mut runner_up = f64::MAX;
                for &h in &board {
                    let d = dist(p, l.center(h));
                    if d < best_d {
                        runner_up = best_d;
                        (best, best_d) = (h, d);
                    } else if d < runner_up {
                        runner_up = d;
                    }
                }

                // Skip points near a tie — the boundary between two cells, where either answer is
                // defensible and only the tie-break decides. Also skip the rim, where the true
                // nearest cell may be one we did not enumerate.
                if runner_up - best_d < 0.5 || best_d > 20.0 {
                    continue;
                }

                assert_eq!(l.hex_at(p), best, "{:?} at {p:?}", l.orientation);
                checked += 1;
            }
        }
        assert!(
            checked > 3000,
            "only {checked} points were unambiguous — the test proves little"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Offset coordinates — the interop contract
// ---------------------------------------------------------------------------------------------

/// The pixel a staggered tile map puts `(col, row)` at. This is the interop contract;
/// everything else here is checked against it.
fn tilemap_pixel(o: Offset, col: i32, row: i32) -> Pt {
    let s3 = 3f64.sqrt();
    let (c, r) = (f64::from(col), f64::from(row));
    let (odd_col, odd_row) = (f64::from(col & 1), f64::from(row & 1));
    match o {
        Offset::OddR => Pt::new((s3 * (c + 0.5 * odd_row)) as f32, (1.5 * r) as f32),
        Offset::EvenR => Pt::new((s3 * (c - 0.5 * odd_row)) as f32, (1.5 * r) as f32),
        Offset::OddQ => Pt::new((1.5 * c) as f32, (s3 * (r + 0.5 * odd_col)) as f32),
        Offset::EvenQ => Pt::new((1.5 * c) as f32, (s3 * (r - 0.5 * odd_col)) as f32),
    }
}

#[test]
fn an_offset_cell_lands_on_the_pixel_a_tilemap_would_draw_it_at() {
    // The headline. This is what "interop" means — not that our numbers round-trip among
    // themselves, but that they agree with the tool that authored the map. Do this test BEFORE the
    // round-trip: a round-trip passes happily when both directions are wrong the same way.
    for (o, orientation) in [
        (Offset::OddR, Orientation::Pointy),
        (Offset::EvenR, Orientation::Pointy),
        (Offset::OddQ, Orientation::Flat),
        (Offset::EvenQ, Orientation::Flat),
    ] {
        let l = HexLayout {
            orientation,
            size: Pt::new(1.0, 1.0),
            origin: Pt::new(0.0, 0.0),
        };

        for col in -20..=20 {
            for row in -20..=20 {
                let ours = l.center(o.to_hex(col, row));
                let theirs = tilemap_pixel(o, col, row);
                assert!(
                    close(ours, theirs),
                    "{o:?} ({col}, {row}): we say {ours:?}, a tilemap says {theirs:?}"
                );
            }
        }
    }
}

#[test]
fn offset_conversion_keeps_neighbours_neighbouring() {
    // The test that catches the bug the textbook formula would have shipped. Copy the standard shear
    // — which assumes `r` grows south, where ours grows north-east — and the conversion still
    // round-trips perfectly while quietly scattering each cell's neighbours across the map. Measured,
    // before the sign was fixed: 361 violations on a board this size. Silent, every one of them.
    for o in [Offset::OddR, Offset::EvenR, Offset::OddQ, Offset::EvenQ] {
        for h in hexes(10) {
            let (col, row) = o.from_hex(h);
            for d in Dir6::ALL {
                let (ncol, nrow) = o.from_hex(h.step(d));
                assert!(
                    (ncol - col).abs() <= 1 && (nrow - row).abs() <= 1,
                    "{o:?}: {h:?} is at ({col}, {row}) but its {d:?} neighbour is at ({ncol}, {nrow}) \
                     — that is not adjacent, and no tilemap would draw it there"
                );
            }
        }
    }
}

#[test]
fn offset_round_trips_both_ways_including_negatives() {
    // Negatives are where the truncating-divide bug lives: Rust's `/` rounds toward zero, so a
    // formula that wants a floor tears the board along row zero. Ours keeps the numerator even, so
    // the divide is exact and the question never arises — but that is a claim, so here is the check.
    for o in [Offset::OddR, Offset::EvenR, Offset::OddQ, Offset::EvenQ] {
        for h in hexes(30) {
            let (col, row) = o.from_hex(h);
            assert_eq!(o.to_hex(col, row), h, "{o:?} {h:?} -> ({col}, {row}) -> ?");
        }
        for col in -30..=30 {
            for row in -30..=30 {
                assert_eq!(
                    o.from_hex(o.to_hex(col, row)),
                    (col, row),
                    "{o:?} ({col}, {row})"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Squares
// ---------------------------------------------------------------------------------------------

#[test]
fn a_square_board_draws_and_picks() {
    let l = SqLayout::new(Pt::new(32.0, 32.0));

    assert_eq!(
        l.center(Sq::new(0, 0)),
        Pt::new(16.0, 16.0),
        "the CENTRE of the first cell"
    );
    assert_eq!(l.center(Sq::new(2, 1)), Pt::new(80.0, 48.0));

    for x in -50..50 {
        for y in -50..50 {
            let s = Sq::new(x, y);
            assert_eq!(l.sq_at(l.center(s)), s);
        }
    }

    // Anywhere inside the cell, not just its middle.
    assert_eq!(l.sq_at(Pt::new(0.1, 0.1)), Sq::new(0, 0));
    assert_eq!(l.sq_at(Pt::new(31.9, 31.9)), Sq::new(0, 0));
    assert_eq!(l.sq_at(Pt::new(32.1, 0.0)), Sq::new(1, 0));
}

#[test]
fn a_square_board_does_not_tear_along_zero() {
    // `sq_at` floors rather than truncates. Truncation folds -0.5 and +0.5 onto the same cell, so
    // cell 0 is twice as wide as every other cell and the whole negative half of the board is off by
    // one. It only shows up left of the origin, which is exactly where nobody tests.
    let l = SqLayout::new(Pt::new(10.0, 10.0));
    assert_eq!(
        l.sq_at(Pt::new(-0.1, -0.1)),
        Sq::new(-1, -1),
        "just left of the origin is cell -1"
    );
    assert_eq!(l.sq_at(Pt::new(-9.9, -9.9)), Sq::new(-1, -1));
    assert_eq!(l.sq_at(Pt::new(-10.1, -10.1)), Sq::new(-2, -2));
}

#[test]
fn square_corners_bound_the_cell() {
    let l = SqLayout::new(Pt::new(10.0, 20.0));
    let c = l.corners(Sq::new(0, 0));
    assert_eq!(
        c[0],
        Pt::new(0.0, 0.0),
        "top-left of the first cell is the origin"
    );
    assert_eq!(c[2], Pt::new(10.0, 20.0), "bottom-right is one cell along");
}

// ---------------------------------------------------------------------------------------------
// The whole point: picking a cell on a real board
// ---------------------------------------------------------------------------------------------

#[test]
fn clicking_a_board_finds_a_cell_and_clicking_past_it_does_not() {
    let g = FullGrid::hexagon(4);
    let l = HexLayout::pointy(Pt::new(30.0, 30.0)).at(Pt::new(400.0, 300.0));

    // Every cell of the board can be clicked, and yields itself.
    for i in g.indices() {
        let clicked = g.index_of(l.hex_at(l.center(g.coord(i))));
        assert_eq!(clicked, Some(i));
    }

    // And the board's `Option` does the rest — no bounds check to write, no special case.
    assert_eq!(
        g.index_of(l.hex_at(Pt::new(4000.0, 3000.0))),
        None,
        "way off the board"
    );
    assert_eq!(
        g.index_of(l.hex_at(Pt::new(400.0, 300.0))),
        g.index_of(Hex::new(0, 0))
    );
}

#[test]
fn a_holed_board_reports_a_click_on_the_hole_as_nothing() {
    // `filtered` leaves a gap. The layout still happily names the cell that *would* be there —
    // geometry does not care — and `index_of` is what says it is not on this board.
    let g = FullGrid::hexagon(3).filtered(|h| h != Hex::new(1, 0));
    let l = HexLayout::pointy(Pt::new(30.0, 30.0));

    let hole = l.center(Hex::new(1, 0));
    assert_eq!(
        l.hex_at(hole),
        Hex::new(1, 0),
        "the geometry still knows where it would be"
    );
    assert_eq!(
        g.index_of(l.hex_at(hole)),
        None,
        "the board says there is nothing there"
    );
}

// ---------------------------------------------------------------------------------------------
// Hostile input — the crate's standing rule: bounded, or loud. Never silently wrong.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_click_at_the_end_of_the_world_still_names_a_real_hex() {
    // A stray click must not wrap. And the cell it lands on must be a genuine lattice cell: a `Hex`
    // is three cube axes summing to zero, and `s()` is DERIVED as `-q - r`. Clamp q and r to
    // `i32::MAX` independently — the obvious thing — and `s` needs 33 bits, so it wraps in release
    // and the "cell" is not on the lattice at all. Hence the tighter bound.
    let l = HexLayout::pointy(Pt::new(1.0, 1.0));

    for p in [
        Pt::new(1e30, 1e30),
        Pt::new(-1e30, 1e30),
        Pt::new(f32::INFINITY, f32::NEG_INFINITY),
        Pt::new(f32::MAX, f32::MAX),
    ] {
        let h = l.hex_at(p);
        let sum = i64::from(h.q) + i64::from(h.r) + i64::from(h.s());
        assert_eq!(
            sum, 0,
            "{p:?} produced {h:?}, which is not on the lattice (q + r + s != 0)"
        );
        assert_eq!(
            FullGrid::hexagon(20).index_of(h),
            None,
            "and no board holds it"
        );
    }
}

#[test]
fn a_nan_click_does_not_quietly_select_the_origin_cell() {
    // `NaN as i32` is 0, so the lazy version of this hands back `Hex(0, 0)` — the middle of the
    // board, which on most maps is your capital. A click that means nothing must land nowhere.
    let l = HexLayout::pointy(Pt::new(32.0, 32.0));
    let g = FullGrid::hexagon(8);

    for p in [
        Pt::new(f32::NAN, 0.0),
        Pt::new(0.0, f32::NAN),
        Pt::new(f32::NAN, f32::NAN),
    ] {
        let h = l.hex_at(p);
        assert_ne!(
            h,
            Hex::new(0, 0),
            "a NaN click picked the origin cell: {p:?}"
        );
        assert_eq!(g.index_of(h), None, "{p:?} -> {h:?}");
    }

    assert_eq!(
        SqLayout::new(Pt::new(32.0, 32.0))
            .sq_at(Pt::new(f32::NAN, 0.0))
            .x,
        (1 << 30) - 1
    );
}

#[test]
#[should_panic(expected = "zero size")]
fn a_layout_with_no_size_is_refused_rather_than_dividing_by_zero() {
    let _ = HexLayout::pointy(Pt::new(0.0, 32.0)).hex_at(Pt::new(1.0, 1.0));
}

#[test]
#[should_panic(expected = "zero size")]
fn a_square_layout_with_no_size_is_refused_too() {
    let _ = SqLayout::new(Pt::new(32.0, 0.0)).sq_at(Pt::new(1.0, 1.0));
}

#[test]
fn a_microscopic_cell_cannot_overflow_the_pick() {
    // The worst ratio two `f32`s can make is about 3.4e38 / 1e-45. In `f32` arithmetic that is
    // infinity, and infinity rounds to a garbage cell. The whole reason the maths inside is `f64` is
    // that the same quotient is merely 3.4e83 there — large, finite, and clampable.
    let l = HexLayout::pointy(Pt::new(1e-30, 1e-30));
    let h = l.hex_at(Pt::new(1e30, 1e30));
    assert_eq!(
        i64::from(h.q) + i64::from(h.r) + i64::from(h.s()),
        0,
        "still a lattice cell"
    );
}

#[test]
fn picking_is_deterministic() {
    // A replay records a mouse position, not a cell. If picking were not a pure function of its
    // input, the replay would diverge from the game it recorded.
    let l = HexLayout::pointy(Pt::new(31.7, 29.3)).at(Pt::new(-12.5, 8.25));
    let ps: Vec<Pt> = (0..500)
        .map(|i| Pt::new((i * 7 % 331) as f32 - 165.0, (i * 13 % 227) as f32 - 113.0))
        .collect();

    let first: Vec<Hex> = ps.iter().map(|&p| l.hex_at(p)).collect();
    for _ in 0..20 {
        let again: Vec<Hex> = ps.iter().map(|&p| l.hex_at(p)).collect();
        assert_eq!(again, first);
    }
}

// ---------------------------------------------------------------------------------------------
// The refactor: line-drawing and picking are the same question, so they share an answer
// ---------------------------------------------------------------------------------------------

#[test]
fn drawing_a_line_and_picking_a_cell_agree() {
    // `FullGrid::line` and `HexLayout::hex_at` both round a fractional hex to a real one, and they now
    // do it with the same code. This is the cross-check that justifies the sharing: the cell halfway
    // along a line must be the cell under the pixel halfway between its endpoints.
    let g = FullGrid::hexagon(6);
    let l = HexLayout::pointy(Pt::new(40.0, 40.0));

    for a in hexes(4) {
        for b in hexes(4) {
            if a.distance(b) < 2 {
                continue;
            }
            let (ia, ib) = (g.at(a), g.at(b));
            let line = g.line(ia, ib);

            let (pa, pb) = (l.center(a), l.center(b));
            let mid = Pt::new((pa.x + pb.x) / 2.0, (pa.y + pb.y) / 2.0);
            let picked = l.hex_at(mid);

            // The midpoint pixel lands on (or immediately beside) the middle cell of the line. Ties
            // on a cell boundary are broken differently by the two — the line nudges, picking does
            // not — so allow one cell of slack rather than pretend that is a bug.
            let middle = g.coord(line[line.len() / 2]);
            assert!(
                picked.distance(middle) <= 1,
                "line {a:?}->{b:?} passes {middle:?} but the midpoint pixel picks {picked:?}"
            );
        }
    }
}
