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

const NO_PARENT: u32 = u32::MAX;
const CEILING: u64 = Cost::MAX as u64;

fn add(a: u64, b: u64) -> u64 {
    a.saturating_add(b).min(CEILING)
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
    est: u64,
    /// The cost so far, alone. Kept because `est` cannot be turned back into it — the heuristic
    /// saturates — and because staleness is a question about cost, not about the estimate.
    cost: u64,
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
    /// The cheapest known total to each cell, or `u64::MAX`.
    cost: Vec<u64>,
    /// How each cell was first reached at its cheapest known cost.
    parent: Vec<u32>,
    queue: BinaryHeap<Visit>,
}

impl Frontier {
    /// A search that has reached its start and nothing else.
    fn new<B: Grid + ?Sized>(b: &B, start: Idx) -> Self {
        let at = slot(b.len(), b.tag(), start);

        let mut it = Self {
            tag: b.tag(),
            cost: vec![u64::MAX; b.len()],
            parent: vec![NO_PARENT; b.len()],
            queue: BinaryHeap::new(),
        };
        it.cost[at] = 0;
        it.queue.push(Visit {
            est: 0,
            cost: 0,
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

    /// The cheapest known total to a cell, or `u64::MAX`.
    fn cost_of(&self, i: Idx) -> u64 {
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
    fn relax(&mut self, from: &Visit, to: Idx, step: u64, h: impl FnOnce() -> u64) {
        let total = add(from.cost, step);
        let at = slot(self.cost.len(), self.tag, to);
        if total >= self.cost[at] {
            return;
        }

        self.cost[at] = total;
        self.parent[at] = from.at.get();
        self.queue.push(Visit {
            est: add(total, h()),
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

        loop {
            let up = self.parent[slot(self.cost.len(), self.tag, at)];
            if up == NO_PARENT {
                break;
            }
            assert!(
                steps.len() <= self.cost.len(),
                "the search's predecessors are cyclic — this is a bug in spacewalk",
            );
            at = Idx::new(self.tag, up);
            steps.push(at);
        }

        steps.reverse();
        Path::of(steps, self.cost_of(goal) as Cost)
    }
}

/// One priced step, or `None` where the rules forbid it.
///
/// The `debug_assert` is the admissibility contract — no step may cost less than the minimum the
/// heuristic was promised — checked while you test, compiled out when you ship. In release the
/// contract rests on [`Movement::scan`], which guarantees it by construction.
fn edge(cost: Option<Cost>, node: Idx, floor: Cost) -> Option<(Idx, u64)> {
    let cost = cost?;
    debug_assert!(
        cost >= floor,
        "a step costs {cost}, below the promised minimum of {floor}: the A* heuristic \
         will overestimate and paths will not be optimal. Use Movement::scan."
    );
    Some((node, u64::from(cost)))
}

/// The neighbours of `i` that can actually be entered, and what entering them costs.
pub(crate) fn succ<'a, B, F>(
    b: &'a B,
    i: Idx,
    m: &'a Movement<F>,
) -> impl Iterator<Item = (Idx, u64)> + 'a
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
) -> impl Iterator<Item = (Idx, u64)> + 'a
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

    // Saturating arithmetic keeps the queue ordered even for extreme costs.
    let heuristic = |i: Idx| u64::from(b.distance(i, goal).saturating_mul(m.min_step()));

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
/// cell after it would too, and the search stops. That early exit is only sound because totals
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
    let cap = u64::from(budget);
    let mut frontier = Frontier::new(b, start);
    let mut reached = Vec::new();

    while let Some(v) = frontier.pop() {
        if v.cost > cap {
            break;
        }
        if frontier.is_stale(&v) {
            continue;
        }
        reached.push((v.at, v.cost as Cost));

        // No heuristic: a search with nowhere in particular to be has nothing to estimate.
        match way {
            Way::Out => {
                for (to, step) in succ(b, v.at, m) {
                    frontier.relax(&v, to, step, || 0);
                }
            }
            Way::In => {
                for (from, step) in pred(b, v.at, m) {
                    frontier.relax(&v, from, step, || 0);
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
