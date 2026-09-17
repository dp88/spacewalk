//! Acceptance test: pathfinding with no grid.
//!
//! A house is rooms and doors. It has no coordinates, no directions, and no distance, so nothing
//! here implements [`spacewalk::Coord`]. The edge list is the whole map.

use spacewalk::{Adjacency, FullGrid, Graph, GraphError, Grid, Movement, Sq};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Room {
    Hall,
    Kitchen,
    Cellar,
    Vault,
    Attic,
}
use Room::*;

fn house() -> Graph<Room> {
    Graph::new([
        (Hall, Kitchen, 4),
        (Hall, Cellar, 1),
        (Cellar, Kitchen, 2),
        (Kitchen, Vault, 5),
    ])
}

#[test]
fn a_route_is_found_by_the_names_the_caller_gave() {
    let g = house();

    let route = g.path(g.at(Hall), g.at(Vault)).unwrap();
    let rooms: Vec<Room> = g.keys_of(route.steps().iter().copied()).collect();

    assert_eq!(
        rooms,
        [Hall, Cellar, Kitchen, Vault],
        "the cellar stairs beat the direct door"
    );
    assert_eq!(route.cost(), 8);
    assert!(
        g.path(g.at(Vault), g.at(Hall)).is_none(),
        "the edges are directed"
    );
}

#[test]
fn reach_runs_forward_and_backward() {
    let g = house();
    let named = |found: Vec<(spacewalk::Idx, u32)>| -> Vec<(Room, u32)> {
        found.into_iter().map(|(i, c)| (g.key(i), c)).collect()
    };

    assert_eq!(
        named(g.reachable(g.at(Hall), 3)),
        [(Hall, 0), (Cellar, 1), (Kitchen, 3)],
        "cheapest first, and the budget stops it short of the vault"
    );
    assert_eq!(
        named(g.reaching(g.at(Vault), 100)),
        [(Vault, 0), (Kitchen, 5), (Cellar, 7), (Hall, 8)],
        "who can get to the vault, and for how much"
    );
}

#[test]
fn nodes_are_numbered_in_the_order_the_edges_name_them() {
    // An edge from a room to itself names the room and adds no edge.
    let g = Graph::new([(Hall, Kitchen, 1), (Attic, Attic, 0), (Kitchen, Cellar, 1)]);

    assert_eq!(g.keys().collect::<Vec<_>>(), [Hall, Kitchen, Attic, Cellar]);
    assert_eq!(g.len(), 4);
    assert_eq!(g.index_of(Vault), None, "no edge names the vault");

    let attic = g.at(Attic);
    assert_eq!(
        g.reachable(attic, 100),
        [(attic, 0)],
        "the attic leads nowhere"
    );
    assert_eq!(g.reaching(attic, 100), [(attic, 0)]);
}

#[test]
fn the_cheaper_of_two_doors_between_the_same_rooms_wins() {
    let g = Graph::new([(Hall, Kitchen, 9), (Hall, Kitchen, 3)]);
    assert_eq!(g.path(g.at(Hall), g.at(Kitchen)).unwrap().cost(), 3);
}

#[test]
fn a_cost_that_could_overflow_a_path_total_is_refused() {
    // Three nodes allow a two-edge path, so no edge may cost more than half of `u32::MAX`.
    let dear = u32::MAX / 2 + 1;
    let built = Graph::try_new([(Hall, Kitchen, dear), (Kitchen, Vault, 1)]);

    assert_eq!(
        built.unwrap_err(),
        GraphError::CostTooHigh {
            cost: dear,
            ceiling: u32::MAX / 2,
            nodes: 3
        }
    );
}

#[test]
fn a_graph_and_a_grid_agree_about_the_same_map() {
    // The two are separate callers of one search engine. Give them the same edges and the same
    // costs, and they must find routes of the same cost and the same reach.
    let grid = FullGrid::square(7, 7, Adjacency::Eight);
    let wall = |c: Sq| c.x == 3 && c.y != 5;
    let price = |to: Sq, diagonal: bool| {
        (!wall(to)).then_some(if to.y % 2 == 0 {
            30
        } else if diagonal {
            14
        } else {
            10
        })
    };
    let walk = Movement::scan(&grid, |s| price(grid.coord(s.to), s.dir.is_diagonal()));

    let mut edges = Vec::new();
    for i in grid.indices() {
        for (d, j) in grid.neighbors(i) {
            if let Some(cost) = price(grid.coord(j), d.is_diagonal()) {
                edges.push((grid.coord(i), grid.coord(j), cost));
            }
        }
    }
    let graph = Graph::new(edges);

    let (a, b) = (Sq::new(0, 0), Sq::new(6, 0));
    assert_eq!(
        graph.path(graph.at(a), graph.at(b)).unwrap().cost(),
        grid.path(grid.at(a), grid.at(b), &walk).unwrap().cost(),
    );

    // The two number their cells differently, so compare what was reached, not in what order.
    let mut by_graph: Vec<(Sq, u32)> = graph
        .reaching(graph.at(b), 60)
        .into_iter()
        .map(|(i, c)| (graph.key(i), c))
        .collect();
    let mut by_grid: Vec<(Sq, u32)> = grid
        .reaching(grid.at(b), 60, &walk)
        .into_iter()
        .map(|(i, c)| (grid.coord(i), c))
        .collect();
    by_graph.sort_unstable();
    by_grid.sort_unstable();
    assert_eq!(by_graph, by_grid);
    assert!(by_grid.len() > 1, "the search went somewhere");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "issued by a different graph")]
fn an_index_from_another_graph_is_refused() {
    // Both graphs hold two nodes, so no bound can tell their indices apart. The tag can, and a
    // debug build checks it.
    let here = Graph::new([(Hall, Kitchen, 1)]);
    let there = Graph::new([(Vault, Cellar, 1)]);
    let _ = there.key(here.at(Hall));
}
