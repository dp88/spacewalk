//! The floating-point `core` does not have.
//!
//! `core` carries the `f64` operations that are pure bit inspection — `abs`, `clamp`, `is_nan`,
//! `is_finite`, `signum` — and leaves the ones that would call into a maths library. This crate
//! needs four of those, at six call sites, all in [`crate::layout`] and [`crate::coord`]: rounding
//! a fractional coordinate onto the lattice, and the sine and cosine of a hexagon's corners.
//!
//! None of them needs a maths library.
//!
//! - The corner angles are six fixed values per orientation, so they are a table. The old code
//!   computed known constants every time it drew a hexagon.
//! - Rounding is integer arithmetic once the value is known to be finite and small enough to have a
//!   fractional part at all.
//!
//! # These are checked against `std`
//!
//! Hand-written floating point is worth being suspicious of, so `tests/float.rs` compares every
//! function here against the `std` method it replaces — over a wide sweep and over the awkward
//! values by hand. An integration test is its own crate and may use `std` freely, which is what
//! makes that possible.

/// `2^52`. At or above this an `f64` has no fractional bits left, so it is already an integer and
/// both functions below can hand it straight back.
const NO_FRACTION: f64 = 4_503_599_627_370_496.0;

/// The largest integer, and the smallest, that the truncation below can trust.
///
/// A float-to-integer cast saturates rather than wrapping, so a value past `i64`'s range would come
/// back as `i64::MAX` and the arithmetic after it would be nonsense. [`NO_FRACTION`] is far inside
/// this, so the guard on it covers this too — but the two facts are independent and only one of
/// them is obvious.
const _: () = assert!(NO_FRACTION < i64::MAX as f64);

/// The largest integer no greater than `v`. `f64::floor`, which is not in `core`.
///
/// NaN and the infinities come back unchanged, which matters more than it looks: `crate::coord`
/// sends NaN to the edge of the lattice on purpose, so that a NaN pixel names no cell. A helper
/// that quietly turned NaN into `0.0` would put the mouse on the origin instead.
pub(crate) fn floor(v: f64) -> f64 {
    if !v.is_finite() || v.abs() >= NO_FRACTION {
        return v;
    }
    let t = trunc(v);
    if t > v { t - 1.0 } else { t }
}

/// The nearest integer to `v`, halfway cases away from zero. `f64::round`, which is not in `core`.
///
/// Written as a comparison against the fractional part, **not** as `floor(v + 0.5)`. That addition
/// rounds before the floor sees it, so the largest `f64` below a half comes out one too high:
/// `floor(0.499_999_999_999_999_94 + 0.5)` is `1.0`, where `std` says `0.0`.
pub(crate) fn round(v: f64) -> f64 {
    if !v.is_finite() || v.abs() >= NO_FRACTION {
        return v;
    }
    let t = trunc(v);
    let frac = v - t;
    if frac >= 0.5 {
        t + 1.0
    } else if frac <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// `v` with its fractional part removed, towards zero. `f64::trunc` is not in `core` either.
///
/// Only ever called on a finite value below [`NO_FRACTION`], where the cast cannot saturate.
///
/// # Zero has a sign, and the cast loses it
///
/// `-0.3 as i64` is `0`, and `0 as f64` is **positive** zero — but `(-0.3).trunc()` is `-0.0`, and
/// so is `(-0.0).trunc()`. The two zeroes compare equal, so nothing downstream would notice until
/// something formatted one. The differential test noticed on its first run.
fn trunc(v: f64) -> f64 {
    debug_assert!(v.is_finite() && v.abs() < NO_FRACTION);
    #[allow(clippy::cast_possible_truncation)]
    let t = v as i64 as f64;

    if t == 0.0 { t.copysign(v) } else { t }
}

/// `√3 / 2`, which is where every hexagon corner that is not on an axis sits.
const H: f64 = 0.866_025_403_784_438_6;

/// The six corners of a flat-top hexagon, as `(cos, sin)` of `0°, 60° … 300°`.
///
/// The old code built these from `PI / 3.0` and a call to `cos` and `sin` per corner, per hexagon,
/// per frame. They were never anything but these twelve numbers.
pub(crate) const FLAT_CORNERS: [(f64, f64); 6] = [
    (1.0, 0.0),
    (0.5, H),
    (-0.5, H),
    (-1.0, 0.0),
    (-0.5, -H),
    (0.5, -H),
];

/// The six corners of a pointy-top hexagon, as `(cos, sin)` of `30°, 90° … 330°`.
///
/// The same ring as [`FLAT_CORNERS`], turned thirty degrees, which is the whole difference between
/// the two orientations.
pub(crate) const POINTY_CORNERS: [(f64, f64); 6] = [
    (H, 0.5),
    (0.0, 1.0),
    (-H, 0.5),
    (-H, -0.5),
    (0.0, -1.0),
    (H, -0.5),
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Every awkward value that has ever broken a hand-written rounding routine.
    fn corners() -> Vec<f64> {
        let mut v = vec![
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.5,
            -1.5,
            2.5,
            -2.5,
            // The largest f64 below a half. `floor(x + 0.5)` gets this one wrong.
            0.499_999_999_999_999_94,
            -0.499_999_999_999_999_94,
            1.0 - f64::EPSILON,
            NO_FRACTION,
            -NO_FRACTION,
            NO_FRACTION - 1.0,
            -(NO_FRACTION - 1.0),
            NO_FRACTION + 2.0,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            i64::MAX as f64,
            i64::MIN as f64,
            // The lattice limit either side, which is what `clamp_i32` cares about.
            f64::from((1i32 << 30) - 1),
            f64::from(-((1i32 << 30) - 1)),
        ];
        for i in -600..600 {
            let x = f64::from(i) / 7.0;
            v.push(x);
            v.push(x * 1e9);
            v.push(x * 1e-9);
        }
        v
    }

    /// Same bits, or both NaN. `==` says false for NaN, and `assert_eq!` would too.
    fn alike(a: f64, b: f64) -> bool {
        (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
    }

    #[test]
    fn floor_agrees_with_std_on_every_value_that_has_ever_caused_trouble() {
        for v in corners() {
            assert!(
                alike(floor(v), v.floor()),
                "floor({v:?}): got {:?}, std says {:?}",
                floor(v),
                v.floor(),
            );
        }
    }

    #[test]
    fn round_agrees_with_std_on_every_value_that_has_ever_caused_trouble() {
        for v in corners() {
            assert!(
                alike(round(v), v.round()),
                "round({v:?}): got {:?}, std says {:?}",
                round(v),
                v.round(),
            );
        }
    }

    #[test]
    fn both_agree_with_std_across_a_wide_sweep() {
        // Two million values spread over the whole range a pixel or a lerp could produce, from a
        // fixed generator so a failure can be reproduced.
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..2_000_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;

            // A spread of magnitudes, not just a spread of bit patterns: most random u64s read as
            // floats are astronomically large and would never exercise the interesting branch.
            #[allow(clippy::cast_precision_loss)]
            let mag = f64::from((s >> 40) as u32) / 1024.0;
            let v = if s & 1 == 0 { mag } else { -mag };

            assert!(alike(floor(v), v.floor()), "floor({v:?})");
            assert!(alike(round(v), v.round()), "round({v:?})");
        }
    }

    #[test]
    fn the_corner_tables_are_the_angles_they_replace() {
        use core::f64::consts::PI;

        for (i, &(c, s)) in FLAT_CORNERS.iter().enumerate() {
            let a = (i as f64).mul_add(PI / 3.0, 0.0);
            assert!(
                (c - a.cos()).abs() < 1e-15,
                "flat {i} cos: {c} vs {}",
                a.cos()
            );
            assert!(
                (s - a.sin()).abs() < 1e-15,
                "flat {i} sin: {s} vs {}",
                a.sin()
            );
        }

        for (i, &(c, s)) in POINTY_CORNERS.iter().enumerate() {
            let a = (i as f64).mul_add(PI / 3.0, PI / 6.0);
            assert!(
                (c - a.cos()).abs() < 1e-15,
                "pointy {i} cos: {c} vs {}",
                a.cos()
            );
            assert!(
                (s - a.sin()).abs() < 1e-15,
                "pointy {i} sin: {s} vs {}",
                a.sin()
            );
        }
    }

    #[test]
    fn every_corner_is_on_the_unit_circle() {
        for &(c, s) in FLAT_CORNERS.iter().chain(POINTY_CORNERS.iter()) {
            assert!((c * c + s * s - 1.0).abs() < 1e-15, "({c}, {s})");
        }
    }
}
