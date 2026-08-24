//! Acceptance test: three-dimensional chess, on a coordinate this crate has never heard of.
//!
//! This is the extensibility claim, made falsifiable. Everything below — the coordinate, its ten
//! directions, its metric — is defined *here*, in the test. The crate ships nothing three
//! dimensional, and needed no change to accept it.
//!
//! It also makes a second point. Chess is not pathfinding: a rook slides, a knight leaps, and
//! neither has a movement cost. A grid crate that shipped only A\* would be useless to it. What
//! chess wants is [`FullGrid::ray`] (slide until something stops you), [`FullGrid::offset`] (leap,
//! ignoring what lies between), and [`FullGrid::step`] — and those are the same primitives the
//! pathfinder is built on.

use std::ops::{Add, Sub};

use spacewalk::{Coord, FullGrid, Grid, Idx, Metric};

// ---------------------------------------------------------------------------------------------
// A coordinate of our own: a square board, stacked into layers.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
struct Cell3 {
    x: i32,
    y: i32,
    z: i32, // the layer
}

impl Cell3 {
    const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Chebyshev in three dimensions. One step changes it by at most one — which is all the A\*
    /// heuristic asks of a metric, and all a custom grid must promise.
    fn chebyshev(self, o: Self) -> u32 {
        let d = self - o;
        d.x.unsigned_abs()
            .max(d.y.unsigned_abs())
            .max(d.z.unsigned_abs())
    }
}

impl Add for Cell3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Cell3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Cell3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Cell3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

/// The eight compass directions within a layer, plus up and down between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Dir3 {
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Nw,
    Up,
    Down,
}

impl Dir3 {
    const ALL: [Dir3; 10] = [
        Dir3::N,
        Dir3::Ne,
        Dir3::E,
        Dir3::Se,
        Dir3::S,
        Dir3::Sw,
        Dir3::W,
        Dir3::Nw,
        Dir3::Up,
        Dir3::Down,
    ];
    /// A rook's lines: the four orthogonals, and straight up and down through the layers.
    const ROOK: [Dir3; 6] = [Dir3::N, Dir3::E, Dir3::S, Dir3::W, Dir3::Up, Dir3::Down];
    /// A bishop's lines: the four diagonals within a layer.
    const BISHOP: [Dir3; 4] = [Dir3::Ne, Dir3::Se, Dir3::Sw, Dir3::Nw];
}

impl Coord for Cell3 {
    type Dir = Dir3;
    const DIRS: &'static [Dir3] = &Dir3::ALL;

    fn step(self, d: Dir3) -> Self {
        let (dx, dy, dz) = match d {
            Dir3::N => (0, -1, 0),
            Dir3::Ne => (1, -1, 0),
            Dir3::E => (1, 0, 0),
            Dir3::Se => (1, 1, 0),
            Dir3::S => (0, 1, 0),
            Dir3::Sw => (-1, 1, 0),
            Dir3::W => (-1, 0, 0),
            Dir3::Nw => (-1, -1, 0),
            Dir3::Up => (0, 0, 1),
            Dir3::Down => (0, 0, -1),
        };
        self + Cell3::new(dx, dy, dz)
    }
}

// ---------------------------------------------------------------------------------------------
// The board, and the pieces that move on it
// ---------------------------------------------------------------------------------------------

/// Three stacked 8×8 layers.
fn board() -> FullGrid<Cell3> {
    let cells =
        (0..3).flat_map(|z| (0..8).flat_map(move |y| (0..8).map(move |x| Cell3::new(x, y, z))));
    // `Metric::scanning`, deliberately. An offset table grows as (2r+1)^d, so on a THREE
    // dimensional board it is far worse than scanning — r = 1000 would be eight billion offsets to
    // interrogate a 192-cell board. The crate lets a custom coordinate say so.
    FullGrid::new(
        cells,
        &Dir3::ALL,
        Metric::scanning(|a: Cell3, b: Cell3| a.chebyshev(b)),
    )
}

/// Slide along every line in `dirs` until something is in the way. A blocker can be taken, but not
/// passed through.
fn slide(
    g: &FullGrid<Cell3>,
    from: Idx,
    dirs: &[Dir3],
    occupied: &dyn Fn(Idx) -> bool,
) -> Vec<Idx> {
    let mut seen = Vec::new();
    for &d in dirs {
        for i in g.ray(from, d) {
            seen.push(i);
            if occupied(i) {
                break; // take it, but go no further
            }
        }
    }
    seen
}

/// A knight's eight leaps, within its own layer. A leap ignores whatever it flies over.
fn knight(g: &FullGrid<Cell3>, from: Idx) -> Vec<Idx> {
    const LEAPS: [(i32, i32); 8] = [
        (1, 2),
        (2, 1),
        (2, -1),
        (1, -2),
        (-1, -2),
        (-2, -1),
        (-2, 1),
        (-1, 2),
    ];
    LEAPS
        .iter()
        .filter_map(|&(dx, dy)| g.offset(from, Cell3::new(dx, dy, 0)))
        .collect()
}

#[test]
fn the_board_is_three_layers_of_sixty_four() {
    let g = board();
    assert_eq!(g.len(), 192);
    assert!(g.contains(Cell3::new(7, 7, 2)));
    assert!(!g.contains(Cell3::new(0, 0, 3)), "there is no fourth layer");
}

#[test]
fn a_rook_slides_its_rank_its_file_and_its_column() {
    let g = board();
    let from = g.index_of(Cell3::new(0, 0, 0)).unwrap();
    let empty = |_| false;

    let reach = slide(&g, from, &Dir3::ROOK, &empty);

    // From a corner of the bottom layer: 7 east, 7 south, and 2 straight up through the layers.
    assert_eq!(reach.len(), 16);
    assert!(reach.contains(&g.index_of(Cell3::new(7, 0, 0)).unwrap()));
    assert!(
        reach.contains(&g.index_of(Cell3::new(0, 0, 2)).unwrap()),
        "up two layers"
    );
    assert!(
        !reach.contains(&g.index_of(Cell3::new(1, 1, 0)).unwrap()),
        "not a diagonal"
    );
}

#[test]
fn a_rook_takes_the_first_piece_in_its_way_and_stops() {
    let g = board();
    let from = g.index_of(Cell3::new(0, 0, 0)).unwrap();
    let blocker = g.index_of(Cell3::new(3, 0, 0)).unwrap();

    let reach = slide(&g, from, &Dir3::ROOK, &|i| i == blocker);

    assert!(reach.contains(&blocker), "it may take the blocker");
    assert!(
        !reach.contains(&g.index_of(Cell3::new(4, 0, 0)).unwrap()),
        "but it may not slide through him"
    );
}

#[test]
fn a_rook_cannot_slide_through_a_hole_in_the_board() {
    // Star-Trek-style boards are not solid cuboids. A missing cell stops a slide dead, and the
    // ray does that on its own — the piece never learns the board has a shape.
    let g = board().filtered(|c| c != Cell3::new(3, 0, 0));
    let from = g.index_of(Cell3::new(0, 0, 0)).unwrap();

    let reach = slide(&g, from, &Dir3::ROOK, &|_| false);
    assert!(!reach.contains(&g.index_of(Cell3::new(4, 0, 0)).unwrap()));
    assert!(
        reach.contains(&g.index_of(Cell3::new(2, 0, 0)).unwrap()),
        "up to the gap"
    );
}

#[test]
fn a_bishop_stays_on_its_diagonals() {
    let g = board();
    let from = g.index_of(Cell3::new(0, 0, 1)).unwrap();

    let reach = slide(&g, from, &Dir3::BISHOP, &|_| false);
    assert_eq!(reach.len(), 7, "the long diagonal of the middle layer");
    assert!(
        reach.iter().all(|&i| g.coord(i).z == 1),
        "and never changes layer"
    );
}

#[test]
fn a_knight_leaps_over_whatever_is_in_the_way() {
    let g = board();
    let from = g.index_of(Cell3::new(1, 0, 0)).unwrap();

    // Its own back rank is packed solid; it leaps anyway, because an offset is a hop and not a walk.
    let leaps: Vec<Cell3> = knight(&g, from).iter().map(|&i| g.coord(i)).collect();

    assert_eq!(
        leaps.len(),
        3,
        "from the edge: three of the eight land on the board"
    );
    assert!(leaps.contains(&Cell3::new(0, 2, 0)));
    assert!(leaps.contains(&Cell3::new(2, 2, 0)));
    assert!(leaps.contains(&Cell3::new(3, 1, 0)));
}

#[test]
fn a_knight_in_the_middle_has_all_eight() {
    let g = board();
    let from = g.index_of(Cell3::new(4, 4, 1)).unwrap();
    assert_eq!(knight(&g, from).len(), 8);
}

#[test]
fn a_piece_can_change_layer() {
    let g = board();
    let from = g.index_of(Cell3::new(4, 4, 0)).unwrap();

    let up = g.step(from, Dir3::Up).unwrap();
    assert_eq!(g.coord(up), Cell3::new(4, 4, 1));
    assert!(
        g.step(from, Dir3::Down).is_none(),
        "nothing below the bottom layer"
    );
}

#[test]
fn pathfinding_still_works_on_a_board_it_was_never_designed_for() {
    // Chess does not want a pathfinder, but a three-dimensional roguelike would — and it gets one,
    // for free, on a coordinate the crate has never seen.
    use spacewalk::Movement;

    let g = board();
    let m = Movement::scan(&g, |_| Some(10));

    let from = g.index_of(Cell3::new(0, 0, 0)).unwrap();
    let to = g.index_of(Cell3::new(7, 7, 2)).unwrap();

    // The true cheapest route is nine steps: this board has no move that crosses *and* changes
    // layer at once, so it is seven diagonals within a layer plus two rungs up.
    assert_eq!(g.path(from, to, &m).unwrap().len(), 9);
}

#[test]
fn a_loose_metric_is_still_a_correct_one() {
    // Worth pinning down, because it is the trap a custom coordinate falls into.
    //
    // Our metric says the far corner is 7 away, while walking there really takes 9 steps — three
    // dimensional Chebyshev assumes you can move diagonally *and* change layer in one go, and this
    // board has no such move. That is an UNDERestimate, which is exactly what A* requires: a
    // heuristic may promise the goal is nearer than it is, and merely searches a little harder. It
    // may never promise the goal is further, which would let it settle for a worse path.
    //
    // So a metric that is loose is slow. A metric that is optimistic is wrong. If you cannot be
    // sure which yours is, return 0 — A* becomes Dijkstra, and Dijkstra is never wrong.
    use spacewalk::{Movement, Step};

    let g = board();
    let from = g.index_of(Cell3::new(0, 0, 0)).unwrap();
    let to = g.index_of(Cell3::new(7, 7, 2)).unwrap();

    assert!(
        g.distance(from, to) <= 9,
        "the metric must never exceed the true number of steps"
    );

    // And the proof it did not cost us correctness: A* agrees with Dijkstra.
    let enter = |_: Step<Cell3>| Some(10);
    let astar = g.path(from, to, &Movement::scan(&g, enter)).unwrap().cost;
    let dijkstra = g.path(from, to, &Movement::new(enter, 0)).unwrap().cost;
    assert_eq!(astar, dijkstra);
}
