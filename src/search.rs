//! The engine: A\* and budget-bounded Dijkstra, over nodes numbered `0..nodes`.
//!
//! [`crate::path`] is the vocabulary a caller writes — what a step costs, what a route is. This is
//! what runs.
//!
//! # Two halves
//!
//! The engine knows nothing of cells. It takes a node count, node numbers, and a closure that
//! yields the edges out of a node. The second half of this file is the [`Grid`] side of that seam:
//! it checks the caller's indices, hands the engine the board's priced steps, and mints indices
//! from the answer.
//!
//! # Why this is not a graph library
//!
//! A general search must key its bookkeeping on whatever a node happens to be, so it reaches for a
//! hash map. This one does not: nodes are numbered `0..nodes`, as a [`Grid`]'s cells are, so "what
//! did this cost" and "how did I get here" are two `Vec`s sized from the board and read by
//! subscript. No hashing, no allocation per cell, and the whole search is bounded by the board —
//! which is the rule the rest of the crate keeps too.
//!
//! # Nothing here is sized by a number you passed in
//!
//! The two tables are one entry per cell. The queue holds at most one entry per *edge*, because a
//! cell is only enqueued when a step into it improves on the best cost so far, and there are
//! `cells × directions` steps in total. A budget, a radius, or a cost cannot make any of it bigger.

use alloc::collections::BinaryHeap;

use crate::coord::Idx;
use crate::grid::{Grid, slot};
use crate::path::{Cost, Movement, Path, Step};
use crate::tag::Tag;
use alloc::vec;
use alloc::vec::Vec;

const NO_PARENT: u32 = u32::MAX;
const CEILING: u64 = Cost::MAX as u64;

fn add(a: u64, b: u64) -> u64 {
    a.saturating_add(b).min(CEILING)
}

/// One node waiting to be expanded, and what reaching it is estimated to cost in total.
///
/// # The ordering, and what it is actually for
///
/// [`BinaryHeap`] is a max-heap, so `Ord` reads backwards: **cheapest first, then lowest number.**
///
/// Be clear about the second half. It is *not* what makes the crate deterministic — removing it
/// leaves `tests/determinism.rs` and `tests/search.rs` green, because the heap is itself
/// deterministic and the order steps go in is fixed by [`Grid::dirs`]. Determinism here is a
/// property of the structure, and it would hold with ties broken arbitrarily.
///
/// What the tie-break buys is that the choice is *stated* rather than emergent. A different heap,
/// a different insertion order, or a `sort_unstable` somewhere upstream would all silently change
/// which of several equally cheap routes comes back; with the rule written down, none of them can.
/// A node's number is its cell's index, so the rule survives rebuilding the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visit {
    /// What the queue is ordered by: the cost so far plus the heuristic.
    est: u64,
    /// The cost so far, alone. Kept because `est` cannot be turned back into it — the heuristic
    /// saturates — and because staleness is a question about cost, not about the estimate.
    cost: u64,
    at: u32,
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

/// A route the engine found, as node numbers, and what walking it costs.
pub(crate) struct Route {
    pub(crate) nodes: Vec<u32>,
    pub(crate) cost: Cost,
}

/// The queue of nodes to visit, and what the search has learned about the ones it has seen.
///
/// One object, not three, and that is the point. A cheaper route to a node means three things at
/// once — a new cost, a new predecessor, and a visit to schedule — and they are only ever correct
/// together. [`Frontier::relax`] is the single way to say it, so they cannot drift apart.
///
/// Both searches below are this type plus a stopping rule.
pub(crate) struct Frontier {
    /// The cheapest known total to each node, or `u64::MAX`.
    cost: Vec<u64>,
    /// How each node was first reached at its cheapest known cost.
    parent: Vec<u32>,
    queue: BinaryHeap<Visit>,
}

impl Frontier {
    /// A search that has reached its start and nothing else.
    fn new(nodes: usize, start: u32) -> Self {
        let mut it = Self {
            cost: vec![u64::MAX; nodes],
            parent: vec![NO_PARENT; nodes],
            queue: BinaryHeap::new(),
        };
        it.cost[start as usize] = 0;
        it.queue.push(Visit {
            est: 0,
            cost: 0,
            at: start,
        });
        it
    }

    /// Whether a popped visit has been overtaken by a cheaper route found since it was queued.
    ///
    /// The queue holds no way to lower a key, so a cheaper route pushes a second entry and leaves
    /// the first behind. Skipping the stale one here is what stops a node being expanded twice.
    fn is_stale(&self, v: &Visit) -> bool {
        self.cost[v.at as usize] < v.cost
    }

    /// Take an edge out of a settled node, and keep it if it beats the cheapest route to `to`.
    ///
    /// Adventure's `EnqueueOrUpdateNeighbor`, and the same bargain: cost, predecessor, and queue
    /// move together or not at all.
    ///
    /// `h` is the heuristic for `to`, and it is a closure because most steps are rejected — on a
    /// board where every cell has eight neighbours, computing an estimate for all of them and then
    /// discarding seven is most of the work. Zero makes this Dijkstra.
    fn relax(&mut self, from: &Visit, to: u32, step: Cost, h: impl FnOnce() -> Cost) {
        let total = add(from.cost, u64::from(step));
        let at = to as usize;
        if total >= self.cost[at] {
            return;
        }

        self.cost[at] = total;
        self.parent[at] = from.at;
        self.queue.push(Visit {
            est: add(total, u64::from(h())),
            cost: total,
            at: to,
        });
    }

    /// The route to `goal`, read back down the predecessors to the node the search set out from.
    ///
    /// # Panics
    ///
    /// If the predecessors do not lead home. They are a tree rooted at the start, so they always
    /// do — but a chain longer than the graph has nodes would mean a cycle, and stopping beats
    /// filling memory with one.
    pub(crate) fn walk_home(&self, goal: u32) -> Route {
        let mut nodes = vec![goal];
        let mut at = goal;

        loop {
            let up = self.parent[at as usize];
            if up == NO_PARENT {
                break;
            }
            assert!(
                nodes.len() <= self.cost.len(),
                "the search's predecessors are cyclic — this is a bug in spacewalk",
            );
            at = up;
            nodes.push(at);
        }

        nodes.reverse();
        Route {
            nodes,
            cost: self.cost[goal as usize] as Cost,
        }
    }
}

/// A\*: the cheapest route from `start` to `goal`, or `None` if there is none.
///
/// `edges` yields each node that a node leads to, and what that edge costs. `estimate` must never
/// exceed the true cost from a node to `goal`; that is what keeps the answer optimal rather than
/// merely plausible. An estimate of zero makes this Dijkstra.
pub(crate) fn astar<E, I>(
    nodes: usize,
    start: u32,
    goal: u32,
    mut edges: E,
    estimate: impl Fn(u32) -> Cost,
) -> Option<Route>
where
    E: FnMut(u32) -> I,
    I: IntoIterator<Item = (u32, Cost)>,
{
    let mut frontier = Frontier::new(nodes, start);

    while let Some(v) = frontier.queue.pop() {
        if frontier.is_stale(&v) {
            continue;
        }
        // The first time the goal is settled it is settled at its cheapest, because the heuristic
        // never overestimates. That is the whole of A*'s claim, and the whole of why it may stop.
        if v.at == goal {
            return Some(frontier.walk_home(goal));
        }
        for (to, step) in edges(v.at) {
            frontier.relax(&v, to, step, || estimate(to));
        }
    }

    None
}

/// Dijkstra, bounded by a budget: every node within reach, and how it was reached.
///
/// Which way the search runs is the caller's choice of `edges`. Out-edges ask where you can get to
/// from here; in-edges ask who can get to here. The two are one search over a graph and its
/// reverse, and on a directed board they give genuinely different answers: a ledge you can drop
/// off is an edge out and no edge back.
///
/// Nodes come back in **non-decreasing cost order**, so the moment one exceeds the budget every
/// node after it would too, and the search stops. That early exit is only sound because totals
/// cannot wrap: with wrapping totals the order is arbitrary and the break would fire at a random
/// point, silently truncating the answer.
pub(crate) fn explore<E, I>(
    nodes: usize,
    start: u32,
    budget: Cost,
    mut edges: E,
) -> (Vec<(u32, Cost)>, Frontier)
where
    E: FnMut(u32) -> I,
    I: IntoIterator<Item = (u32, Cost)>,
{
    let cap = u64::from(budget);
    let mut frontier = Frontier::new(nodes, start);
    let mut reached = Vec::new();

    while let Some(v) = frontier.queue.pop() {
        if v.cost > cap {
            break;
        }
        if frontier.is_stale(&v) {
            continue;
        }
        reached.push((v.at, v.cost as Cost));

        // No heuristic: a search with nowhere in particular to be has nothing to estimate.
        for (to, step) in edges(v.at) {
            frontier.relax(&v, to, step, || 0);
        }
    }

    (reached, frontier)
}

// -- the grid's side of the seam ---------------------------------------------------------------

/// One priced step, or `None` where the rules forbid it.
///
/// The `debug_assert` is the admissibility contract — no step may cost less than the minimum the
/// heuristic was promised — checked while you test, compiled out when you ship. In release the
/// contract rests on [`Movement::scan`], which guarantees it by construction.
fn edge(cost: Option<Cost>, node: Idx, floor: Cost) -> Option<(u32, Cost)> {
    let cost = cost?;
    debug_assert!(
        cost >= floor,
        "a step costs {cost}, below the promised minimum of {floor}: the A* heuristic \
         will overestimate and paths will not be optimal. Use Movement::scan."
    );
    Some((node.raw(), cost))
}

/// The neighbours of `i` that can actually be entered, and what entering them costs.
fn succ<'a, B, F>(b: &'a B, i: Idx, m: &'a Movement<F>) -> impl Iterator<Item = (u32, Cost)> + 'a
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    b.neighbors(i)
        .filter_map(move |(dir, to)| edge(m.enter(Step { from: i, to, dir }), to, m.min_step()))
}

/// The cells that can step into `j`, and what that step costs them. [`succ`], in reverse.
fn pred<'a, B, F>(b: &'a B, j: Idx, m: &'a Movement<F>) -> impl Iterator<Item = (u32, Cost)> + 'a
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    b.in_neighbors(j)
        .filter_map(move |(dir, from)| edge(m.enter(Step { from, to: j, dir }), from, m.min_step()))
}

/// The engine's node numbers, as the indices of the board that was searched.
fn minted(tag: Tag, found: Vec<(u32, Cost)>) -> Vec<(Idx, Cost)> {
    found
        .into_iter()
        .map(|(n, cost)| (Idx::new(tag, n), cost))
        .collect()
}

/// The engine's route, as a [`Path`] over the board that was searched.
fn as_path(tag: Tag, route: Route) -> Path {
    let steps = route.nodes.into_iter().map(|n| Idx::new(tag, n)).collect();
    Path::of(steps, route.cost)
}

/// Behind [`Grid::path`](crate::Grid::path): A\* over the board's priced steps.
///
/// The heuristic is the board's own metric scaled by the cheapest step the rules allow. That never
/// overestimates — a cell `d` steps away cannot be reached for less than `d` cheapest steps — which
/// is what keeps the answer optimal rather than merely plausible. See [`Movement::scan`].
pub(crate) fn find<B, F>(b: &B, start: Idx, goal: Idx, m: &Movement<F>) -> Option<Path>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let tag = b.tag();
    let _ = slot(b.len(), tag, goal);
    let _ = slot(b.len(), tag, start);

    // Saturating arithmetic keeps the queue ordered even for extreme costs.
    let estimate = |n| {
        b.distance(Idx::new(tag, n), goal)
            .saturating_mul(m.min_step())
    };
    let steps = |n| succ(b, Idx::new(tag, n), m);

    astar(b.len(), start.raw(), goal.raw(), steps, estimate).map(|route| as_path(tag, route))
}

/// Behind [`Grid::reachable`](crate::Grid::reachable): where you can get to, and what it costs.
pub(crate) fn reachable<B, F>(b: &B, start: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let tag = b.tag();
    let _ = slot(b.len(), tag, start);

    let (found, _) = explore(b.len(), start.raw(), budget, |n| {
        succ(b, Idx::new(tag, n), m)
    });
    minted(tag, found)
}

/// Behind [`Grid::reaching`](crate::Grid::reaching): one backward search, bounded by the budget.
pub(crate) fn reaching<B, F>(b: &B, goal: Idx, budget: Cost, m: &Movement<F>) -> Vec<(Idx, Cost)>
where
    B: Grid + ?Sized,
    F: Fn(Step<B::Cell>) -> Option<Cost>,
{
    let tag = b.tag();
    let _ = slot(b.len(), tag, goal);

    let (found, _) = explore(b.len(), goal.raw(), budget, |n| {
        pred(b, Idx::new(tag, n), m)
    });
    minted(tag, found)
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
    let tag = b.tag();
    let _ = slot(b.len(), tag, target);
    let _ = slot(b.len(), tag, start);

    let (seen, frontier) = explore(b.len(), start.raw(), budget, |n| {
        succ(b, Idx::new(tag, n), m)
    });
    let &(goal, _) = seen
        .iter()
        .min_by_key(|&&(n, cost)| (b.distance(Idx::new(tag, n), target), cost, n))?;

    Some(as_path(tag, frontier.walk_home(goal)))
}
