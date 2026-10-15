//! The ender dragon's node graph search: vanilla `EnderDragon.findPath` over its 24 `Node`s,
//! with the `BinaryHeap` open set ported as-is so ties break the same way.

use pumpkin_util::math::vector3::Vector3;

pub const NODE_COUNT: usize = 24;

/// Vanilla `EnderDragon.nodeAdjacency`: bit `i` links a node to node `i`.
pub const NODE_ADJACENCY: [u32; NODE_COUNT] = [
    6146, 8197, 8202, 16404, 32808, 32848, 65696, 131392, 131712, 263424, 526848, 525313, 1581057,
    3166214, 2138120, 6373424, 4358208, 12910976, 9044480, 9706496, 15216640, 13688832, 11763712,
    8257536,
];

/// Vanilla `pathfinder.Node`, the coordinates only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Node {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Node {
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Vanilla `Node.distanceTo(Node)`.
    #[must_use]
    pub fn distance_to(&self, to: &Self) -> f32 {
        self.distance_to_sqr(to).sqrt()
    }

    /// Vanilla `Node.distanceToSqr(Node)`.
    #[must_use]
    pub fn distance_to_sqr(&self, to: &Self) -> f32 {
        let xd = (to.x - self.x) as f32;
        let yd = (to.y - self.y) as f32;
        let zd = (to.z - self.z) as f32;
        xd * xd + yd * yd + zd * zd
    }
}

/// Vanilla `Path` as the dragon uses it: `isDone`, `advance` and `getNextNodePos`.
#[derive(Clone, Debug)]
pub struct DragonPath {
    nodes: Vec<Node>,
    next_node_index: usize,
}

impl DragonPath {
    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.next_node_index >= self.nodes.len()
    }

    pub const fn advance(&mut self) {
        self.next_node_index += 1;
    }

    #[must_use]
    pub fn get_next_node_pos(&self) -> Vector3<i32> {
        let node = self.nodes[self.next_node_index];
        Vector3::new(node.x, node.y, node.z)
    }
}

/// The search state vanilla keeps on each `Node`. Index `NODE_COUNT` is the final node.
#[derive(Clone, Copy)]
struct SearchNode {
    node: Node,
    closed: bool,
    f: f32,
    g: f32,
    h: f32,
    came_from: Option<usize>,
    heap_idx: i32,
}

/// Vanilla `pathfinder.BinaryHeap`, holding indices into the search nodes.
struct BinaryHeap {
    heap: Vec<usize>,
}

impl BinaryHeap {
    fn insert(&mut self, nodes: &mut [SearchNode], node: usize) {
        self.heap.push(node);
        nodes[node].heap_idx = (self.heap.len() - 1) as i32;
        self.up_heap(nodes, self.heap.len() - 1);
    }

    fn pop(&mut self, nodes: &mut [SearchNode]) -> usize {
        let popped = self.heap[0];
        let last = self.heap.len() - 1;
        self.heap.swap(0, last);
        self.heap.pop();
        if !self.heap.is_empty() {
            self.down_heap(nodes, 0);
        }
        nodes[popped].heap_idx = -1;
        popped
    }

    fn change_cost(&mut self, nodes: &mut [SearchNode], node: usize, new_cost: f32) {
        let old_cost = nodes[node].f;
        nodes[node].f = new_cost;
        let idx = nodes[node].heap_idx as usize;
        if new_cost < old_cost {
            self.up_heap(nodes, idx);
        } else {
            self.down_heap(nodes, idx);
        }
    }

    const fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    #[expect(
        clippy::neg_cmp_op_on_partial_ord,
        reason = "vanilla compares floats this way"
    )]
    fn up_heap(&mut self, nodes: &mut [SearchNode], mut idx: usize) {
        let node = self.heap[idx];
        let cost = nodes[node].f;
        while idx > 0 {
            let parent_idx = (idx - 1) >> 1;
            let parent = self.heap[parent_idx];
            if !(cost < nodes[parent].f) {
                break;
            }
            self.heap[idx] = parent;
            nodes[parent].heap_idx = idx as i32;
            idx = parent_idx;
        }
        self.heap[idx] = node;
        nodes[node].heap_idx = idx as i32;
    }

    #[expect(
        clippy::neg_cmp_op_on_partial_ord,
        reason = "vanilla compares floats this way"
    )]
    fn down_heap(&mut self, nodes: &mut [SearchNode], mut idx: usize) {
        let node = self.heap[idx];
        let cost = nodes[node].f;
        let size = self.heap.len();
        loop {
            let left_idx = 1 + (idx << 1);
            let right_idx = left_idx + 1;
            if left_idx >= size {
                break;
            }
            let left_node = self.heap[left_idx];
            let left_cost = nodes[left_node].f;
            let (right_node, right_cost) = if right_idx >= size {
                (None, f32::INFINITY)
            } else {
                let right = self.heap[right_idx];
                (Some(right), nodes[right].f)
            };
            if left_cost < right_cost {
                if !(left_cost < cost) {
                    break;
                }
                self.heap[idx] = left_node;
                nodes[left_node].heap_idx = idx as i32;
                idx = left_idx;
            } else {
                let Some(right_node) = right_node else {
                    break;
                };
                if !(right_cost < cost) {
                    break;
                }
                self.heap[idx] = right_node;
                nodes[right_node].heap_idx = idx as i32;
                idx = right_idx;
            }
        }
        self.heap[idx] = node;
        nodes[node].heap_idx = idx as i32;
    }
}

/// Vanilla `EnderDragon.findPath`. `minimum_node_index` is 12 when the fight has no crystals
/// left (or there is no fight), which keeps the search off the outer ring.
#[must_use]
pub fn find_path(
    graph: &[Node; NODE_COUNT],
    start_index: usize,
    end_index: usize,
    final_node: Option<Node>,
    minimum_node_index: usize,
) -> Option<DragonPath> {
    let mut nodes: Vec<SearchNode> = graph
        .iter()
        .chain(final_node.iter())
        .map(|&node| SearchNode {
            node,
            closed: false,
            f: 0.0,
            g: 0.0,
            h: 0.0,
            came_from: None,
            heap_idx: -1,
        })
        .collect();

    let from = start_index;
    let mut to = end_index;
    nodes[from].g = 0.0;
    nodes[from].h = nodes[from].node.distance_to(&nodes[to].node);
    nodes[from].f = nodes[from].h;
    let mut open_set = BinaryHeap { heap: Vec::new() };
    open_set.insert(&mut nodes, from);
    let mut closest = from;

    while !open_set.is_empty() {
        let open_node = open_set.pop(&mut nodes);
        if nodes[open_node].node == nodes[to].node {
            if final_node.is_some() {
                nodes[NODE_COUNT].came_from = Some(to);
                to = NODE_COUNT;
            }
            return Some(reconstruct_path(&nodes, to));
        }

        let to_node = nodes[to].node;
        if nodes[open_node].node.distance_to(&to_node) < nodes[closest].node.distance_to(&to_node) {
            closest = open_node;
        }

        nodes[open_node].closed = true;
        let x_index = open_node;

        for i in minimum_node_index..NODE_COUNT {
            if NODE_ADJACENCY[x_index] & (1 << i) > 0 {
                let adjacent = i;
                if !nodes[adjacent].closed {
                    let tentative_g_score = nodes[open_node].g
                        + nodes[open_node].node.distance_to(&nodes[adjacent].node);
                    let in_open_set = nodes[adjacent].heap_idx >= 0;
                    if !in_open_set || tentative_g_score < nodes[adjacent].g {
                        nodes[adjacent].came_from = Some(open_node);
                        nodes[adjacent].g = tentative_g_score;
                        nodes[adjacent].h = nodes[adjacent].node.distance_to(&to_node);
                        let cost = nodes[adjacent].g + nodes[adjacent].h;
                        if in_open_set {
                            open_set.change_cost(&mut nodes, adjacent, cost);
                        } else {
                            nodes[adjacent].f = cost;
                            open_set.insert(&mut nodes, adjacent);
                        }
                    }
                }
            }
        }
    }

    if closest == from {
        return None;
    }

    tracing::debug!("Failed to find path from {start_index} to {end_index}");
    if final_node.is_some() {
        nodes[NODE_COUNT].came_from = Some(closest);
        closest = NODE_COUNT;
    }
    Some(reconstruct_path(&nodes, closest))
}

/// Vanilla `EnderDragon.reconstructPath`: the chain back to the start, which it includes.
fn reconstruct_path(nodes: &[SearchNode], to: usize) -> DragonPath {
    let mut path = vec![nodes[to].node];
    let mut node = to;
    while let Some(came_from) = nodes[node].came_from {
        node = came_from;
        path.push(nodes[node].node);
    }
    path.reverse();
    DragonPath {
        nodes: path,
        next_node_index: 0,
    }
}
