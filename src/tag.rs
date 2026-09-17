//! Which board's numbering an index belongs to.
//!
//! [`Tag`] is `pub` inside a private module. The [`Grid`](crate::Grid) trait is sealed by a
//! supertrait that returns one, so the type must be public. No outside crate can name it, and none
//! has a use for it.

use core::hash::Hash;
#[cfg(debug_assertions)]
use core::hash::Hasher;

/// FNV-1a, because `core` has no hasher and this needs no more than one.
///
/// A [`Tag`] is compared only against other tags made in the same process, in a debug build, to
/// catch a mistake. It does not have to resist an adversary or survive a restart — it has to be
/// cheap, deterministic, and mix well enough that two different boards rarely collide.
#[cfg(debug_assertions)]
struct Fnv(u64);

#[cfg(debug_assertions)]
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Which board's numbering an [`Idx`] belongs to.
///
/// A tag names a *numbering*, not an object. Two boards that number the same cells in the same
/// order share one, and their indices are interchangeable — which is the property
/// [`FullGrid::new`](crate::FullGrid::new) promises and `tests/save.rs` rests on. A tag derived
/// from a counter would break that, so this is derived from the cells.
///
/// In release builds this is a zero-sized type: every check below compiles to nothing, and an
/// [`Idx`] is a bare `u32` again.
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Tag(u32);

/// Which board's numbering an [`Idx`] belongs to. Zero-sized in release; see the debug definition.
#[cfg(not(debug_assertions))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Tag;

impl Tag {
    /// Derive a tag from a board's cells, in index order.
    ///
    /// The iterator is **never consumed in release**, so a caller may hand over one that would be
    /// expensive to walk, and pay nothing for it in a shipped build.
    #[cfg(debug_assertions)]
    pub(crate) fn of<H: Hash>(items: impl IntoIterator<Item = H>) -> Self {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        let mut n: u64 = 0;
        for item in items {
            item.hash(&mut h);
            n += 1;
        }
        // Length is mixed in last so that a prefix of another board's cells cannot collide with it.
        h.write_u64(n);
        // Forced odd, which keeps zero free to mean [`Tag::ANY`].
        #[allow(clippy::cast_possible_truncation)]
        Self(h.finish() as u32 | 1)
    }

    /// Derive a tag from a board's cells, in index order. Ignores its argument in release.
    #[cfg(not(debug_assertions))]
    pub(crate) fn of<H: Hash>(items: impl IntoIterator<Item = H>) -> Self {
        let _ = items;
        Self
    }

    /// Whether an index carrying `self` may be handed to a board carrying `other`.
    ///
    /// Equal tags, or [`Tag::ANY`] on either side. One definition serves both profiles: in release
    /// a `Tag` is zero-sized, so every arm is trivially true — and every caller is inside a
    /// `debug_assert`, which is gone by then anyway.
    pub(crate) fn agrees(self, other: Self) -> bool {
        self == other || self == Self::ANY || other == Self::ANY
    }
}

impl Tag {
    /// A tag that matches every board: what an index carries when nothing named its grid.
    ///
    /// One thing mints these — [`CellMap::iter`](crate::CellMap::iter) on a map that came back from
    /// serde, which has no grid to name. Dropping the check there is deliberate: what makes such a
    /// map line up is the cells saved beside it, and those are already the rule. See `tests/save.rs`.
    pub(crate) const ANY: Self = Self::any();

    #[cfg(debug_assertions)]
    const fn any() -> Self {
        Self(0)
    }

    #[cfg(not(debug_assertions))]
    const fn any() -> Self {
        Self
    }
}
