//! Routes through nodes that you name yourself, with no grid.
//!
//! A grid finds its edges in its geometry: a cell leads to the cells beside it. Some maps have no
//! geometry. Rooms and the doors between them, towns and their roads, a handful of waypoints — the
//! edges are a list, and that list is all there is. [`Graph`] is that list, with the searches a
//! [`Grid`](crate::Grid) has.
//!
//! ```
//! use spacewalk::Graph;
//!
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
//! enum Room { Hall, Kitchen, Cellar, Vault }
//! use Room::*;
//!
//! // One way only: from, to, and what it costs.
//! let house = Graph::new([
//!     (Hall, Kitchen, 4),
//!     (Hall, Cellar, 1),
//!     (Cellar, Kitchen, 2),
//!     (Kitchen, Vault, 5),
//! ]);
//!
//! let route = house.path(house.at(Hall), house.at(Vault)).unwrap();
//! let rooms: Vec<Room> = house.keys_of(route.steps().iter().copied()).collect();
//!
//! assert_eq!(rooms, [Hall, Cellar, Kitchen, Vault]);
//! assert_eq!(route.cost(), 8);
//! assert!(house.path(house.at(Vault), house.at(Hall)).is_none(), "no edge leads back");
//! ```
//!
//! # The same rules as a grid
//!
//! A node has two names, as a cell has. Its key is yours and is what you save. Its [`Idx`] is a
//! dense number that only this graph issues, and the searches speak it. **Serialize keys, never
//! indices.** A debug build refuses an index that a different graph or grid issued.
//!
//! The edges are directed, so a [`Path`] cannot be reversed, and [`Graph::reachable`] and
//! [`Graph::reaching`] are different questions.
//!
//! # What it does not have
//!
//! A graph has no distance between two nodes, so it has no A\* heuristic, no range query, and no
//! `path_toward`. Every search is Dijkstra, which is always correct. It also keeps its costs: to
//! close a door, build the graph again without that edge. Building is one pass over the list.

use core::fmt;
use core::hash::Hash;

use alloc::vec;
use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::coord::Idx;
use crate::full::MAX_CELLS;
use crate::grid::cost_ceiling;
use crate::path::{Cost, Path};
use crate::search;
use crate::tag::Tag;

/// Why a graph could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphError {
    /// The edges name more nodes than [`MAX_CELLS`].
    TooManyNodes {
        /// The number of nodes named when the limit was passed.
        nodes: u64,
    },
    /// An edge costs more than a path on this graph can safely accumulate.
    CostTooHigh {
        /// The largest cost in the edge list.
        cost: Cost,
        /// The largest safe cost for a graph of this size.
        ceiling: Cost,
        /// The number of nodes in the graph.
        nodes: usize,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooManyNodes { nodes } => write!(
                f,
                "the edges name {nodes} nodes; a graph may hold at most {MAX_CELLS}",
            ),
            Self::CostTooHigh {
                cost,
                ceiling,
                nodes,
            } => write!(
                f,
                "an edge costs {cost}, but on a graph of {nodes} nodes no edge may cost more than \
                 {ceiling} without overflowing Cost ({})",
                Cost::MAX,
            ),
        }
    }
}

/// Edges in compressed-row form: the edges of node `n` are `node[start[n]..start[n + 1]]`, each
/// with its cost.
#[derive(Debug, Clone)]
struct Rows {
    start: Vec<usize>,
    node: Vec<u32>,
    cost: Vec<Cost>,
}

impl Rows {
    /// Two passes: count what lands in each row, then place it. Placement keeps the input order,
    /// so the order the searches meet edges in is the order the caller listed them in.
    fn of(nodes: usize, edges: impl Iterator<Item = (u32, u32, Cost)> + Clone) -> Self {
        let mut start = vec![0usize; nodes + 1];
        for (row, _, _) in edges.clone() {
            start[row as usize + 1] += 1;
        }
        for k in 1..start.len() {
            start[k] += start[k - 1];
        }

        let total = start[nodes];
        let (mut node, mut cost) = (vec![0; total], vec![0; total]);
        let mut at = start.clone();
        for (row, other, c) in edges {
            let put = at[row as usize];
            node[put] = other;
            cost[put] = c;
            at[row as usize] += 1;
        }

        Self { start, node, cost }
    }

    fn row(&self, n: u32) -> impl Iterator<Item = (u32, Cost)> + '_ {
        let (lo, hi) = (self.start[n as usize], self.start[n as usize + 1]);
        (lo..hi).map(move |k| (self.node[k], self.cost[k]))
    }
}

/// A directed graph over nodes that you name with a type of your own.
///
/// See the [module documentation](self) for an example, and for what a graph shares with a
/// [`Grid`](crate::Grid).
///
/// `K` is the name of a node: an enum, a newtype, or anything else that is cheap to copy and can
/// key a hash map. Do not use a bare integer when a type of your own will do. With your own type,
/// the compiler refuses a cost written in a node's place.
#[derive(Debug, Clone)]
pub struct Graph<K> {
    /// Every node's key, in index order: `keys[i]` is the key of node `i`.
    keys: Vec<K>,
    /// The reverse lookup: key to index.
    index: HashMap<K, u32>,
    /// Where each node leads.
    out: Rows,
    /// Who leads to each node.
    back: Rows,
    /// This graph's numbering, hashed from `keys`. See [`Tag`].
    tag: Tag,
}

impl<K: Copy + Eq + Hash + fmt::Debug> Graph<K> {
    /// Build a graph from its edges: `(from, to, cost)`.
    ///
    /// The nodes are the keys that the edges name. There is no separate list of nodes, so an edge
    /// cannot name a node that is missing from it. Nodes are numbered in the order the list first
    /// names them, `from` before `to`. That order fixes the indices, and with them how a search
    /// breaks a tie between two equally cheap routes.
    ///
    /// An edge from a node to itself adds the node and no edge. Use one to name a node that has no
    /// other edge.
    ///
    /// Two edges may join the same pair of nodes. A search takes the cheaper one.
    ///
    /// # Panics
    ///
    /// Where [`Graph::try_new`] returns an error.
    #[must_use]
    pub fn new(edges: impl IntoIterator<Item = (K, K, Cost)>) -> Self {
        Self::try_new(edges).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Fallibly build a graph from its edges: `(from, to, cost)`.
    ///
    /// This numbers and checks the edges as [`Graph::new`] does.
    ///
    /// # Errors
    ///
    /// Returns a [`GraphError`] when the edges name more than [`MAX_CELLS`] nodes, or when an edge
    /// costs so much that a path total could overflow [`Cost`]. The ceiling is
    /// `Cost::MAX / (nodes - 1)`, because no simple path visits a node twice.
    pub fn try_new(edges: impl IntoIterator<Item = (K, K, Cost)>) -> Result<Self, GraphError> {
        let mut keys = Vec::new();
        let mut index = HashMap::new();
        let mut number = |key: K| -> Result<u32, GraphError> {
            if let Some(&n) = index.get(&key) {
                return Ok(n);
            }
            // Checked as we go, as `FullGrid::try_new` does: an edge list that never ends must
            // stop here, not after it has been counted.
            let n = match u32::try_from(keys.len()) {
                Ok(n) if u64::from(n) < MAX_CELLS => n,
                _ => {
                    return Err(GraphError::TooManyNodes {
                        nodes: keys.len() as u64 + 1,
                    });
                }
            };
            index.insert(key, n);
            keys.push(key);
            Ok(n)
        };

        let mut numbered = Vec::new();
        let mut dearest = 0;
        for (from, to, cost) in edges {
            let (from, to) = (number(from)?, number(to)?);
            if from != to {
                numbered.push((from, to, cost));
                dearest = dearest.max(cost);
            }
        }

        let ceiling = cost_ceiling(keys.len());
        if dearest > ceiling {
            return Err(GraphError::CostTooHigh {
                cost: dearest,
                ceiling,
                nodes: keys.len(),
            });
        }

        let out = Rows::of(keys.len(), numbered.iter().copied());
        let back = Rows::of(
            keys.len(),
            numbered.iter().map(|&(from, to, cost)| (to, from, cost)),
        );
        Ok(Self {
            tag: Tag::of(keys.iter()),
            keys,
            index,
            out,
            back,
        })
    }

    /// Mint one of this graph's indices. The single door between a bare number and an [`Idx`].
    const fn idx(&self, n: u32) -> Idx {
        Idx::new(self.tag, n)
    }

    /// Check that `i` belongs to this graph, and hand back its number.
    #[track_caller]
    fn slot(&self, i: Idx) -> u32 {
        debug_assert!(
            i.tag().agrees(self.tag),
            "node {i} was issued by a different graph or grid than the one being asked \
             (indices are per-graph, and this one may be in range for both). \
             Look the node up again with `Graph::index_of` or `Graph::at` on the graph you mean.",
        );
        assert!(
            (i.raw() as usize) < self.len(),
            "node {i} is not in this graph, which has {} nodes",
            self.len(),
        );
        i.raw()
    }

    /// How many nodes the graph has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the graph has no nodes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The index of a node that you know is in the graph.
    ///
    /// [`Graph::index_of`] is the same question when a missing node is an answer, not a mistake.
    ///
    /// # Panics
    ///
    /// If no edge names `key`.
    #[must_use]
    #[track_caller]
    pub fn at(&self, key: K) -> Idx {
        self.index_of(key)
            .unwrap_or_else(|| panic!("{key:?} is not a node of this graph"))
    }

    /// The index of a node, or `None` if no edge names it.
    #[must_use]
    pub fn index_of(&self, key: K) -> Option<Idx> {
        self.index.get(&key).map(|&n| self.idx(n))
    }

    /// The key of the node at an index.
    ///
    /// # Panics
    ///
    /// If `i` is not a node of this graph.
    #[must_use]
    pub fn key(&self, i: Idx) -> K {
        self.keys[self.slot(i) as usize]
    }

    /// Every node index, in order.
    pub fn indices(&self) -> impl Iterator<Item = Idx> + '_ {
        (0u32..).take(self.keys.len()).map(|n| self.idx(n))
    }

    /// Every node's key, in index order.
    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.keys.iter().copied()
    }

    /// The keys of some indices, in the order given. This is how you read a [`Path`].
    ///
    /// # Panics
    ///
    /// If any index is not a node of this graph.
    pub fn keys_of(&self, of: impl IntoIterator<Item = Idx>) -> impl Iterator<Item = K> {
        of.into_iter().map(|i| self.key(i))
    }

    /// The cheapest path from `start` to `goal`, or `None` if no route leads there.
    ///
    /// # Panics
    ///
    /// If `start` or `goal` is not a node of this graph.
    #[must_use]
    pub fn path(&self, start: Idx, goal: Idx) -> Option<Path> {
        let (from, to) = (self.slot(start), self.slot(goal));

        let route = search::astar(self.len(), from, to, |n| self.out.row(n), |_| 0)?;
        let steps = route.nodes.into_iter().map(|n| self.idx(n)).collect();
        Some(Path::of(steps, route.cost))
    }

    /// Every node that `start` can reach for no more than `budget`, and what reaching it costs.
    ///
    /// Cheapest first, and `start` itself comes back at cost 0.
    ///
    /// # Panics
    ///
    /// If `start` is not a node of this graph.
    #[must_use]
    pub fn reachable(&self, start: Idx, budget: Cost) -> Vec<(Idx, Cost)> {
        let from = self.slot(start);
        let (found, _) = search::explore(self.len(), from, budget, |n| self.out.row(n));
        self.minted(found)
    }

    /// Every node that can reach `goal` for no more than `budget`, and what it costs them.
    ///
    /// The edges are directed, so this is not [`Graph::reachable`] read backwards. It is one search
    /// over the edges that lead *into* each node.
    ///
    /// # Panics
    ///
    /// If `goal` is not a node of this graph.
    #[must_use]
    pub fn reaching(&self, goal: Idx, budget: Cost) -> Vec<(Idx, Cost)> {
        let to = self.slot(goal);
        let (found, _) = search::explore(self.len(), to, budget, |n| self.back.row(n));
        self.minted(found)
    }

    fn minted(&self, found: Vec<(u32, Cost)>) -> Vec<(Idx, Cost)> {
        found
            .into_iter()
            .map(|(n, cost)| (self.idx(n), cost))
            .collect()
    }
}
