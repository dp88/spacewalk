//! The engine: A\* and budget-bounded Dijkstra, over a board's dense indices.
//!
//! [`crate::path`] is the vocabulary a caller writes — what a step costs, what a route is. This is
//! what runs.
//!
//! # Why this is not a graph library
//!
//! A general search must key its bookkeeping on whatever a node happens to be, so it reaches for a
//! hash map. This one does not: a [`Grid`]'s cells are numbered `0..len`, so "what did this cost"
//! and "how did I get here" are two `Vec`s sized from the board and read by subscript. No hashing,
//! no allocation per cell, and the whole search is bounded by the board — which is the rule the
//! rest of the crate keeps too.
//!
//! # Nothing here is sized by a number you passed in
//!
//! The two tables are one entry per cell. The queue holds at most one entry per *edge*, because a
//! cell is only enqueued when a step into it improves on the best cost so far, and there are
//! `cells × directions` steps in total. A budget, a radius, or a cost cannot make any of it bigger.

use alloc::collections::BinaryHeap;

use crate::coord::{Idx, Tag};
use crate::grid::{Grid, slot};
use crate::path::{Cost, Movement, Path, Step};
use alloc::vec;
use alloc::vec::Vec;

/// A running total, which **saturates instead of wrapping**.
///
/// This exists because of a bug that took a machine down, and it is worth knowing about.
///
/// Dijkstra and A\* rest on one invariant: *extending a path may never make it cheaper*. That is
/// what lets them settle a cell and stop reconsidering it. Rust does not check integer overflow in
/// release builds — it wraps — so a total that runs past [`Cost::MAX`] comes back round as a
/// **small** number, and a longer path suddenly looks cheaper than a short one. The invariant
/// breaks, cells re-open forever, the search's heap grows without bound, and the process eats
/// memory until the machine dies. In release only. Which is the build a game ships.
///
/// Saturating addition is monotone non-decreasing, so the invariant holds no matter what numbers
/// come in. The search stays bounded and terminates; a total that would have overflowed simply pegs
/// at the maximum. Wrong-but-bounded beats fatal, and [`Movement::scan`] refuses the inputs that
/// would get here in the first place — so this is the second line of defence, not the first.
///
/// There is deliberately **no `Add`**. An operator that silently saturates is a trap; a method
/// named for what it does is not. Every sum the two searches perform goes through [`Acc::plus`].
///
/// # Why it is wider than a `Cost`
///
/// A total pegs at [`Cost::MAX`], and [`Acc::UNREACHED`] must be a value no total can ever hold —
/// otherwise a route whose cost saturated would be indistinguishable from a cell nothing reached,
/// and [`Frontier::relax`] would refuse to improve on it. On a ten-cell corridor of very dear steps
/// that is the difference between the cheapest route and no route at all; `tests/robust.rs` has it.
///
/// One more byte of headroom makes the two values different by construction rather than by luck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Acc(u64);

impl Acc {
    /// A cell nothing has reached yet. Above the ceiling, so no total can reach it.
    const UNREACHED: Self = Self(u64::MAX);

    /// The start of a search: it costs nothing to stand where you already are.
    const ZERO: Self = Self(0);

    /// The most a total may say. Past here it stops counting rather than wrapping.
    const CEILING: u64 = Cost::MAX as u64;

    /// Extend a total by one step, saturating at the ceiling. The only addition in this module.
    fn plus(self, step: Self) -> Self {
        Self(self.0.saturating_add(step.0).min(Self::CEILING))
    }

    /// What this total is worth as a [`Cost`].
    pub(crate) fn get(self) -> Cost {
        debug_assert!(self.0 <= Self::CEILING, "a total escaped its ceiling");
        #[allow(clippy::cast_possible_truncation)]
        {
            self.0 as Cost
        }
    }

    /// Wrap a step's price. Crate-internal, so a caller can never mint a total.
    pub(crate) fn of(cost: Cost) -> Self {
        Self(u64::from(cost))
    }
}

/// Where a cell was reached from, or that it was not reached at all.
///
/// A newtype over the sentinel rather than a bare `u32`, for the same reason [`Idx`] is a newtype:
/// `u32::MAX` in a table of indices reads as a cell four billion along, and the compiler cannot see
/// the difference. Here it cannot be read as an index at all — [`Parent::get`] hands back an
/// `Option`, and there is nothing else to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Parent(u32);

impl Parent {
    /// The search's root, or a cell it never reached. No board has this many cells; see `MAX_CELLS`.
    const NONE: Self = Self(u32::MAX);

    /// Record that a cell was reached from `i`.
    fn of(i: Idx) -> Self {
        Self(i.get())
    }

    /// The cell this one was reached from, or `None` at a root or an unreached cell.
    fn get(self) -> Option<u32> {
        (self != Self::NONE).then_some(self.0)
    }
}

/// One cell waiting to be expanded, and what reaching it is estimated to cost in total.
///
/// # The ordering, and what it is actually for
///
/// [`BinaryHeap`] is a max-heap, so `Ord` reads backwards: **cheapest first, then lowest index.**
///
/// Be clear about the second half. It is *not* what makes the crate deterministic — removing it
/// leaves `tests/determinism.rs` and `tests/search.rs` green, because the heap is itself
/// deterministic and the order steps go in is fixed by [`Grid::dirs`]. Determinism here is a
/// property of the structure, and it would hold with ties broken arbitrarily.
///
/// What the tie-break buys is that the choice is *stated* rather than emergent. A different heap,
/// a different insertion order, or a `sort_unstable` somewhere upstream would all silently change
/// which of several equally cheap routes comes back; with the rule written down, none of them can.
/// An [`Idx`] compares by its number alone, so it survives rebuilding the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visit {
    /// What the queue is ordered by: the cost so far plus the heuristic.
    est: Acc,
    /// The cost so far, alone. Kept because `est` cannot be turned back into it — the heuristic
    /// saturates — and because staleness is a question about cost, not about the estimate.
    cost: Acc,
    at: Idx,
}

impl Ord for Visit {
    fn cmp(&self, o: &Self) -> core::cmp::Ordering {
        o.est.cmp(&self.est).then_with(|| o.at.cmp(&self.at))
    }
}

impl PartialOrd for Visit {
    fn partial_cmp(&self, o: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// The queue of cells to visit, and what the search has learned about the ones it has seen.
///
/// One object, not three, and that is the point. A cheaper route to a cell means three things at
/// once — a new cost, a new predecessor, and a visit to schedule — and they are only ever correct
/// together. [`Frontier::relax`] is the single way to say it, so they cannot drift apart.
///
/// Both searches below are this type plus a stopping rule.
struct Frontier {
    /// The board being searched. Every index handed in is checked against it.
    tag: Tag,
    /// The cheapest known total to each cell, or [`Acc::UNREACHED`].
    cost: Vec<Acc>,
    /// How each cell was first reached at its cheapest known cost.
    parent: Vec<Parent>,
    queue: BinaryHeap<Visit>,
}

impl Frontier {
    /// A search that has reached its start and nothing else.
    fn new<B: Grid + ?Sized>(b: &B, start: Idx) -> Self {
        let at = slot(b.len(), b.tag(), start);

        let mut it = Self {
            tag: b.tag(),
            cost: vec![Acc::UNREACHED; b.len()],
            parent: vec![Parent::NONE; b.len()],
            queue: BinaryHeap::new(),
        };
        it.cost[at] = Acc::ZERO;
        it.queue.push(Visit {
            est: Acc::ZERO,
            cost: Acc::ZERO,
            at: start,
        });
        it
    }

    /// The cheapest cell still waiting, or `None` when the search is exhausted.
    fn pop(&mut self) -> Option<Visit> {
        self.queue.pop()
    }

    /// Whether a popped visit has been overtaken by a cheaper route found since it was queued.
    ///
    /// The queue holds no way to lower a key, so a cheaper route pushes a second entry and leaves
    /// the first behind. Skipping the stale one here is what stops a cell being expanded twice.
    fn is_stale(&self, v: &Visit) -> bool {
        self.cost_of(v.at) < v.cost
    }

    /// The cheapest known total to a cell, or [`Acc::UNREACHED`].
    fn cost_of(&self, i: Idx) -> Acc {
        self.cost[slot(self.cost.len(), self.tag, i)]
    }

    /// Take a step out of a settled cell, and keep it if it beats the cheapest route to `to`.
    ///
    /// Adventure's `EnqueueOrUpdateNeighbor`, and the same bargain: cost, predecessor, and queue
    /// move together or not at all.
    ///
    /// `h` is the heuristic for `to`, and it is a closure because most steps are rejected — on a
    /// board where every cell has eight neighbours, computing an estimate for all of them and then
    /// discarding seven is most of the work. Zero makes this Dijkstra.
    fn relax(&mut self, from: &Visit, to: Idx, step: Acc, h: impl FnOnce() -> Acc) {
        let total = from.cost.plus(step);
        let at = slot(self.cost.len(), self.tag, to);
        if total >= self.cost[at] {
            return;
        }

        self.cost[at] = total;
        self.parent[at] = Parent::of(from.at);
        self.queue.push(Visit {
            est: total.plus(h()),
            cost: total,
            at: to,
        });
    }

    /// The route to `goal`, read back down the predecessors to the cell the search set out from.
    ///
    /// # Panics
    ///
    /// If the predecessors do not lead home. They are a tree rooted at the start, so they always
    /// do — but a chain longer than the board has cells would mean a cycle, and stopping beats
    /// filling memory with one.
    fn walk_home(&self, goal: Idx) -> Path {
        let mut steps = vec![goal];
        let mut at = goal;

        while let Some(up) = self.parent[slot(self.cost.len(), self.tag, at)].get() {
            assert!(
                steps.len() <= self.cost.len(),
                "the search's predecessors are cyclic — this is a bug in spacewalk",
            );
            at = Idx::new(self.tag, up);
            steps.push(at);
        }

        steps.reverse();
        Path::of(steps, self.cost_of(goal).get())
    }
}

/// One priced step, or `None` where the rules forbid it.
///
/// The `debug_assert` is the admissibility contract — no step may cost less than the minimum the
/// heuristic was promised — checked while you test, compiled out when you ship. In release the
/// contract rests on [`Movement::scan`], which guarantees it by construction.
fn edge(cost: Option<Cost>, node: Idx, floor: Cost) -> Option<(Idx, Acc)> {
    let cost = cost?;
    debug_assert!(
        cost >= floor,
        "a step costs {cost}, below the promised minimum of {floor}: the A* heuristic \
         will overestimate and paths will not be optimal. Use Movement::scan."
    );
    Some((node, Acc::of(cost)))
}

/// The neighbours of `i` that can actually be entered, and what entering them costs.
pub(crate) fn succ<'a, B, F>(
    b: &'a B,
    i: Idx,
    m: &'a Movement<F>,
) -> impl Iterator<Item = (Idx, Acc)> + 'a
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    b.neighbors(i)
        .filter_map(move |(dir, to)| edge(m.enter(Step { from: i, to, dir }), to, m.min_step()))
}

/// The cells that can step into `j`, and what that step costs them. [`succ`], in reverse.
pub(crate) fn pred<'a, B, F>(
    b: &'a B,
    j: Idx,
    m: &'a Movement<F>,
) -> impl Iterator<Item = (Idx, Acc)> + 'a
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    b.in_neighbors(j)
        .filter_map(move |(dir, from)| edge(m.enter(Step { from, to: j, dir }), from, m.min_step()))
}

/// A\*: the cheapest route from `start` to `goal`, or `None` if there is none.
///
/// The heuristic is the board's own metric scaled by the cheapest step the rules allow. That never
/// overestimates — a cell `d` steps away cannot be reached for less than `d` cheapest steps — which
/// is what keeps the answer optimal rather than merely plausible. See [`Movement::scan`].
pub(crate) fn find<B, F>(b: &B, start: Idx, goal: Idx, m: &Movement<F>) -> Option<Path>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let _ = slot(b.len(), b.tag(), goal);
    let mut frontier = Frontier::new(b, start);

    // Saturating on both halves. A colossal `h` used to overflow on the very next addition, which
    // is the bug `Acc` exists to stop; the multiply saturates for the same reason.
    let heuristic = |i: Idx| Acc::of(b.distance(i, goal).saturating_mul(m.min_step()));

    while let Some(v) = frontier.pop() {
        if frontier.is_stale(&v) {
            continue;
        }
        // The first time the goal is settled it is settled at its cheapest, because the heuristic
        // never overestimates. That is the whole of A*'s claim, and the whole of why it may stop.
        if v.at == goal {
            return Some(frontier.walk_home(goal));
        }
        for (to, step) in succ(b, v.at, m) {
            frontier.relax(&v, to, step, || heuristic(to));
        }
    }

    None
}

/// Behind [`Grid::reachable`](crate::Grid::reachable): where you can get to, and what it costs.
pub(crate) fn reachable<B, F>(b: &B, start: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    explore(b, start, budget, m, Way::Out).0
}

/// Which way a search runs.
///
/// The two are one search over a graph and its reverse, and on a directed board they give genuinely
/// different answers: a ledge you can drop off is an edge out and no edge back. Naming the direction
/// rather than passing a closure keeps that visible, and keeps the neighbour iterators concrete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Way {
    /// Where you can get to from here. [`succ`].
    Out,
    /// Who can get to here. [`pred`].
    In,
}

/// Dijkstra, bounded by a budget: every cell within reach, and how it was reached.
///
/// Cells come back in **non-decreasing cost order**, so the moment one exceeds the budget every
/// cell after it would too, and the search stops. That early exit is only sound because [`Acc`]
/// cannot wrap: with wrapping totals the order is arbitrary and the break would fire at a random
/// point, silently truncating the answer.
fn explore<B, F>(
    b: &B,
    start: Idx,
    budget: Cost,
    m: &Movement<F>,
    way: Way,
) -> (Vec<(Idx, Cost)>, Frontier)
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let cap = Acc::of(budget);
    let mut frontier = Frontier::new(b, start);
    let mut reached = Vec::new();

    while let Some(v) = frontier.pop() {
        if v.cost > cap {
            break;
        }
        if frontier.is_stale(&v) {
            continue;
        }
        reached.push((v.at, v.cost.get()));

        // No heuristic: a search with nowhere in particular to be has nothing to estimate.
        match way {
            Way::Out => {
                for (to, step) in succ(b, v.at, m) {
                    frontier.relax(&v, to, step, || Acc::ZERO);
                }
            }
            Way::In => {
                for (from, step) in pred(b, v.at, m) {
                    frontier.relax(&v, from, step, || Acc::ZERO);
                }
            }
        }
    }

    (reached, frontier)
}

/// Behind [`Grid::reaching`](crate::Grid::reaching): one backward search, bounded by the budget.
pub(crate) fn reaching<B, F>(b: &B, goal: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    explore(b, goal, budget, m, Way::In).0
}

/// Behind [`Grid::path_toward`](crate::Grid::path_toward): one bounded search, then the route home.
///
/// The cell it settles for is the one nearest the target, breaking ties by cost and then by index —
/// so a perfectly symmetric board still answers the same way every time.
pub(crate) fn toward<B, F>(
    b: &B,
    start: Idx,
    target: Idx,
    budget: Cost,
    m: &Movement<F>,
) -> Option<Path>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let _ = slot(b.len(), b.tag(), target);

    let (seen, frontier) = explore(b, start, budget, m, Way::Out);
    let &(goal, _) = seen
        .iter()
        .min_by_key(|&&(i, c)| (b.distance(i, target), c, i))?;

    Some(frontier.walk_home(goal))
}
