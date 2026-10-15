use std::collections::HashSet;

use pumpkin_util::math::position::BlockPos;

/// A set of block positions that iterates in the order vanilla's `java.util.HashSet<BlockPos>`
/// does, for features that draw random numbers or settle values while walking such a set.
///
/// This replays `HashMap`'s table: entries sit in buckets chosen by `BlockPos.hashCode`, a bucket
/// keeps its entries in insertion order, and iteration walks the buckets in index order.
#[derive(Clone, Debug)]
pub struct JavaBlockPosSet {
    table: Vec<Vec<(i32, BlockPos)>>,
    members: HashSet<BlockPos>,
}

impl Default for JavaBlockPosSet {
    fn default() -> Self {
        Self::new()
    }
}

impl FromIterator<BlockPos> for JavaBlockPosSet {
    fn from_iter<I: IntoIterator<Item = BlockPos>>(positions: I) -> Self {
        let mut set = Self::new();
        for pos in positions {
            set.insert(pos);
        }
        set
    }
}

/// `HashMap.DEFAULT_INITIAL_CAPACITY`.
const DEFAULT_INITIAL_CAPACITY: usize = 16;
/// `HashMap.TREEIFY_THRESHOLD`.
const TREEIFY_THRESHOLD: usize = 8;
/// `HashMap.MIN_TREEIFY_CAPACITY`.
const MIN_TREEIFY_CAPACITY: usize = 64;

impl JavaBlockPosSet {
    #[must_use]
    pub fn new() -> Self {
        Self {
            table: vec![Vec::new(); DEFAULT_INITIAL_CAPACITY],
            members: HashSet::new(),
        }
    }

    /// `HashMap.putVal`: returns whether the position was new.
    ///
    /// The table doubles when the size passes 3/4 of the capacity, or when a bucket reaches 9
    /// entries while there are fewer than 64 buckets (`treeifyBin`). With 64 or more buckets such a
    /// bucket becomes a red-black tree, whose order this does not model; position sets spread too
    /// evenly for that to happen in practice.
    pub fn insert(&mut self, pos: BlockPos) -> bool {
        if !self.members.insert(pos) {
            return false;
        }
        let hash = spread(block_pos_hash(&pos));
        let mask = self.table.len() - 1;
        let bucket = &mut self.table[hash as usize & mask];
        bucket.push((hash, pos));
        if bucket.len() > TREEIFY_THRESHOLD && self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        }
        if self.members.len() > self.table.len() / 4 * 3 {
            self.resize();
        }
        true
    }

    #[must_use]
    pub fn contains(&self, pos: &BlockPos) -> bool {
        self.members.contains(pos)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members in the order a Java `HashSet` holding them iterates.
    #[must_use]
    pub fn to_java_order(&self) -> Vec<BlockPos> {
        self.table.iter().flatten().map(|&(_, pos)| pos).collect()
    }

    /// `HashMap.resize`: doubles the table, splitting each bucket in order.
    fn resize(&mut self) {
        let mut resized = vec![Vec::new(); self.table.len() * 2];
        let mask = resized.len() - 1;
        for entry in std::mem::take(&mut self.table).into_iter().flatten() {
            resized[entry.0 as usize & mask].push(entry);
        }
        self.table = resized;
    }
}

/// `Vec3i.hashCode`.
const fn block_pos_hash(pos: &BlockPos) -> i32 {
    pos.0
        .y
        .wrapping_add(pos.0.z.wrapping_mul(31))
        .wrapping_mul(31)
        .wrapping_add(pos.0.x)
}

/// `HashMap.hash`: spreads the high bits down.
const fn spread(hash: i32) -> i32 {
    hash ^ ((hash as u32) >> 16) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(x, y, z)
    }

    #[test]
    fn hash_matches_vec3i() {
        // (y + z * 31) * 31 + x
        assert_eq!(block_pos_hash(&pos(1, 2, 3)), (2 + 3 * 31) * 31 + 1);
        assert_eq!(block_pos_hash(&pos(-5, 64, -7)), (64 - 7 * 31) * 31 - 5);
    }

    #[test]
    fn order_follows_buckets_then_insertion() {
        // Hashes 0, 16, 1 and -1 spread to buckets 0, 0, 1 and 0 of the 16-bucket table
        // (`-1 ^ (-1 >>> 16)` is 0xFFFF0000), so (-1,0,0) follows (16,0,0) in bucket 0.
        let mut set = JavaBlockPosSet::new();
        for p in [pos(0, 0, 0), pos(16, 0, 0), pos(1, 0, 0), pos(-1, 0, 0)] {
            set.insert(p);
        }
        assert_eq!(
            set.to_java_order(),
            vec![pos(0, 0, 0), pos(16, 0, 0), pos(-1, 0, 0), pos(1, 0, 0)]
        );
    }

    #[test]
    fn resizing_regroups_buckets() {
        // 13 entries pass the 12-entry threshold, so the table has 32 buckets: (16,0,0) moves to
        // bucket 16, after (1..=12,0,0).
        let mut set = JavaBlockPosSet::new();
        set.insert(pos(0, 0, 0));
        set.insert(pos(16, 0, 0));
        for x in 1..=11 {
            set.insert(pos(x, 0, 0));
        }
        let mut expected = vec![pos(0, 0, 0)];
        expected.extend((1..=11).map(|x| pos(x, 0, 0)));
        expected.push(pos(16, 0, 0));
        assert_eq!(set.to_java_order(), expected);
    }

    #[test]
    fn insert_dedups() {
        let mut set = JavaBlockPosSet::new();
        assert!(set.insert(pos(1, 2, 3)));
        assert!(!set.insert(pos(1, 2, 3)));
        assert_eq!(set.len(), 1);
    }
}
