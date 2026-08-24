//! Fixtures shared by more than one test file. Each integration test is its own crate, so this
//! lives in the standard `tests/common` subdirectory, which cargo does not build as a test binary.
#![allow(dead_code, unused_macros, unused_imports)]

use spacewalk::{Cost, FullGrid, Grid, Movement, Sq, Step};

/// A movement where every step costs the same, so cost is a plain count of steps.
pub fn open(g: &FullGrid<Sq>) -> Movement<impl Fn(Step<Sq>) -> Option<Cost> + use<>> {
    Movement::scan(g, |_| Some(10))
}

/// Eight-way movement where a diagonal costs √2, on a scale where an orthogonal step is 10.
pub fn diagonal_aware(g: &FullGrid<Sq>) -> Movement<impl Fn(Step<Sq>) -> Option<Cost> + use<>> {
    Movement::scan(g, |s| Some(if s.dir.is_diagonal() { 14 } else { 10 }))
}

/// A one-dimensional integer [`Coord`](spacewalk::Coord) — the shape every pathological-lattice
/// test wants. `coord_1d!(P, E, |x| P(x.0 + 1))` declares `struct P(i32)` stepping along the
/// single fresh direction `E` by the given closure; `coord_1d!(P, Spin = Spin::Round, ...)`
/// reuses an existing direction type instead. `Add`/`Sub` are ordinary vector arithmetic — the
/// behaviour a test is actually about belongs in the step closure, next to the test.
macro_rules! coord_1d {
    ($coord:ident, $dir:ident, $step:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        struct $dir;
        crate::common::coord_1d!($coord, $dir = $dir, $step);
    };
    ($coord:ident, $dir:ty = $unit:expr, $step:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        struct $coord(i32);
        impl ::std::ops::Add for $coord {
            type Output = Self;
            fn add(self, o: Self) -> Self {
                $coord(self.0 + o.0)
            }
        }
        impl ::std::ops::Sub for $coord {
            type Output = Self;
            fn sub(self, o: Self) -> Self {
                $coord(self.0 - o.0)
            }
        }
        impl ::spacewalk::Coord for $coord {
            type Dir = $dir;
            const DIRS: &'static [$dir] = &[$unit];
            fn step(self, _: $dir) -> Self {
                let step: fn(Self) -> Self = $step;
                step(self)
            }
        }
    };
}
pub(crate) use coord_1d;
