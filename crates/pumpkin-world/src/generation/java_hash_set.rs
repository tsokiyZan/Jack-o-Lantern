use std::collections::HashSet;

use pumpkin_util::math::position::BlockPos;

/// A set of block positions that iterates in the order vanilla's `java.util.HashSet<BlockPos>`
/// does, for features that draw random numbers while walking such a set.
///
/// Java's iteration order depends only on the members and the order they were added: entries sit
/// in buckets chosen by `BlockPos.hashCode`, and a bucket keeps its entries in insertion order.
#[derive(Default, Clone, Debug)]
pub struct JavaBlockPosSet {
    inserted: Vec<BlockPos>,
    members: HashSet<BlockPos>,
}

impl JavaBlockPosSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `Set.add`: returns whether the position was new.
    pub fn insert(&mut self, pos: BlockPos) -> bool {
        if self.members.insert(pos) {
            self.inserted.push(pos);
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn contains(&self, pos: &BlockPos) -> bool {
        self.members.contains(pos)
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.inserted.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.inserted.is_empty()
    }

    /// The members in the order a Java `HashSet` holding them iterates.
    #[must_use]
    pub fn to_java_order(&self) -> Vec<BlockPos> {
        java_hash_map_order(&self.inserted)
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

/// Replays `HashMap.putVal` for distinct keys added in `inserted` order and returns the order of
/// the resulting table: buckets in index order, each bucket's entries in insertion order.
///
/// The table starts at 16 buckets and doubles when the size passes 3/4 of the capacity, or when a
/// bucket reaches 9 entries while there are fewer than 64 buckets (`treeifyBin`). With 64 or more
/// buckets such a bucket becomes a red-black tree, whose order this does not model; position sets
/// spread too evenly for that to happen in practice.
fn java_hash_map_order(inserted: &[BlockPos]) -> Vec<BlockPos> {
    const TREEIFY_THRESHOLD: usize = 8;
    const MIN_TREEIFY_CAPACITY: usize = 64;

    let mut table: Vec<Vec<(i32, BlockPos)>> = vec![Vec::new(); 16];
    let mut size = 0;
    for &pos in inserted {
        let hash = spread(block_pos_hash(&pos));
        let mask = table.len() - 1;
        let bucket = &mut table[hash as usize & mask];
        bucket.push((hash, pos));
        if bucket.len() > TREEIFY_THRESHOLD && table.len() < MIN_TREEIFY_CAPACITY {
            table = resize(table);
        }
        size += 1;
        if size > table.len() / 4 * 3 {
            table = resize(table);
        }
    }
    table.into_iter().flatten().map(|(_, pos)| pos).collect()
}

/// `HashMap.resize`: doubles the table, splitting each bucket in order.
fn resize(table: Vec<Vec<(i32, BlockPos)>>) -> Vec<Vec<(i32, BlockPos)>> {
    let mut resized = vec![Vec::new(); table.len() * 2];
    let mask = resized.len() - 1;
    for entry in table.into_iter().flatten() {
        resized[entry.0 as usize & mask].push(entry);
    }
    resized
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
