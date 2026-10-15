use rustc_hash::{FxBuildHasher, FxHashMap};

use pumpkin_util::math::vector3::Vector3;

use crate::entity::ai::pathfinder::node::{Coordinate, Node};

// Binary heap implementation that uses the node's f score as the node's value
// The node's position in the heap is stored in `node.heap_idx`, this is just copying vanilla
// behavior, I'm not sure it's necessary. Infact, it's always going to be 0 when popping so it's
// only use is when peeking into the heap. Possibly could be removed?

#[derive(Debug, Clone)]
pub struct BinaryHeap {
    heap: Vec<Option<Node>>,
    position_map: FxHashMap<Vector3<i32>, usize>,
    size: usize,
}

impl BinaryHeap {
    #[must_use]
    pub fn new() -> Self {
        let mut heap = Vec::with_capacity(1024);
        heap.push(None);

        Self {
            heap,
            position_map: FxHashMap::default(),
            size: 0,
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let mut heap = Vec::with_capacity(capacity + 1);
        heap.push(None);

        Self {
            heap,
            position_map: FxHashMap::with_capacity_and_hasher(capacity, FxBuildHasher),
            size: 0,
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.size
    }

    pub fn clear(&mut self) {
        self.heap.clear();
        self.heap.push(None);
        self.position_map.clear();
        self.size = 0;
    }

    pub fn insert(&mut self, mut node: Node) {
        self.size += 1;

        if self.heap.len() <= self.size {
            self.heap.resize(self.size * 2, None);
        }

        node.heap_idx = self.size as i32;
        self.position_map.insert(node.as_vector3(), self.size);

        self.heap[self.size] = Some(node);
        self.bubble_up(self.size);
    }

    pub fn pop(&mut self) -> Option<Node> {
        if self.is_empty() {
            return None;
        }

        let min_node = self.heap[1].take()?;
        self.position_map.remove(&min_node.as_vector3());

        if self.size == 1 {
            self.size = 0;
            return Some(min_node);
        }

        if let Some(mut last_node) = self.heap[self.size].take() {
            last_node.heap_idx = 1;
            self.position_map.insert(last_node.as_vector3(), 1);
            self.heap[1] = Some(last_node);
        }

        self.size -= 1;
        self.bubble_down(1);

        Some(min_node)
    }

    #[must_use]
    pub fn peek(&self) -> Option<&Node> {
        if self.is_empty() {
            None
        } else {
            self.heap[1].as_ref()
        }
    }

    pub fn change_cost(&mut self, coords: &dyn Coordinate, new_f_score: f32) {
        if let Some(&index) = self.position_map.get(&coords.as_vector3())
            && let Some(ref mut node) = self.heap[index]
        {
            let old_f = node.f;
            node.f = new_f_score;

            if new_f_score < old_f {
                self.bubble_up(index);
            } else if new_f_score > old_f {
                self.bubble_down(index);
            }
        }
    }

    pub fn contains(&self, coords: &dyn Coordinate) -> bool {
        self.position_map.contains_key(&coords.as_vector3())
    }

    /// Get a reference to the node at the given coordinates, if it exists in the heap.
    pub fn get_node(&self, coords: &dyn Coordinate) -> Option<&Node> {
        self.position_map
            .get(&coords.as_vector3())
            .and_then(|&index| self.heap[index].as_ref())
    }

    /// Updates an existing node's fields and reorders the heap.
    /// This is used when we find a better path to an already-open node.
    pub fn update_node(&mut self, coords: &dyn Coordinate, updated: Node) {
        if let Some(&index) = self.position_map.get(&coords.as_vector3())
            && let Some(ref mut node) = self.heap[index]
        {
            let old_f = node.f;
            let heap_idx = node.heap_idx;
            *node = updated;
            node.heap_idx = heap_idx;

            if node.f < old_f {
                self.bubble_up(index);
            } else if node.f > old_f {
                self.bubble_down(index);
            }
        }
    }

    /// Drain all nodes from the heap, returning them as a Vec.
    pub fn drain(&mut self) -> Vec<Node> {
        let nodes: Vec<Node> = self.heap[1..=self.size]
            .iter()
            .filter_map(|node_opt| *node_opt)
            .collect();
        self.clear();
        nodes
    }

    #[must_use]
    pub fn get_heap(&self) -> Vec<Node> {
        self.heap[1..=self.size]
            .iter()
            .filter_map(|node_opt| *node_opt)
            .collect()
    }

    fn bubble_up(&mut self, mut index: usize) {
        while index > 1 {
            let parent_index = index / 2;

            let should_swap = {
                if let (Some(node), Some(parent)) = (&self.heap[index], &self.heap[parent_index]) {
                    node.f < parent.f
                } else {
                    false
                }
            };

            if !should_swap {
                break;
            }

            self.swap_nodes(index, parent_index);
            index = parent_index;
        }
    }

    fn f_at(&self, index: usize) -> f32 {
        self.heap[index].map_or(f32::INFINITY, |node| node.f)
    }

    fn bubble_down(&mut self, mut index: usize) {
        let cost = self.f_at(index);
        loop {
            let left_child = index * 2;
            let right_child = left_child + 1;
            if left_child > self.size {
                break;
            }

            let left_cost = self.f_at(left_child);
            let right_cost = if right_child > self.size {
                f32::INFINITY
            } else {
                self.f_at(right_child)
            };

            // Vanilla takes the right child on a tie between the two children.
            let (child, child_cost) = if left_cost < right_cost {
                (left_child, left_cost)
            } else {
                (right_child, right_cost)
            };
            let moves_down = child_cost < cost;
            if !moves_down {
                break;
            }

            self.swap_nodes(index, child);
            index = child;
        }
    }

    fn swap_nodes(&mut self, i: usize, j: usize) {
        if let Some(ref mut node_i) = self.heap[i] {
            node_i.heap_idx = j as i32;
            self.position_map.insert(node_i.as_vector3(), j);
        }
        if let Some(ref mut node_j) = self.heap[j] {
            node_j.heap_idx = i as i32;
            self.position_map.insert(node_j.as_vector3(), i);
        }

        self.heap.swap(i, j);
    }
}

impl Default for BinaryHeap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::math::position::BlockPos;

    use super::BinaryHeap;
    use crate::entity::ai::pathfinder::node::Node;

    fn node(x: i32, f: f32) -> Node {
        let mut node = Node::new(BlockPos::new(x, 0, 0));
        node.f = f;
        node
    }

    #[test]
    fn down_heap_takes_right_child_on_tie() {
        // Vanilla `BinaryHeap` pops these as 0, 2, 1, 3: after the first pop
        // the last node sinks below two equal children and swaps with the right one.
        let mut heap = BinaryHeap::new();
        heap.insert(node(0, 0.0));
        heap.insert(node(1, 1.0));
        heap.insert(node(2, 1.0));
        heap.insert(node(3, 5.0));

        let order: Vec<i32> = std::iter::from_fn(|| heap.pop().map(|n| n.pos.0.x)).collect();
        assert_eq!(order, [0, 2, 1, 3]);
    }
}
