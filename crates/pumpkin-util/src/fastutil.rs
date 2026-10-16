//! The key layout of fastutil's open-addressing hash maps, which decides the order vanilla
//! iterates them in.

use crate::math::java_string_hash;

/// `HashCommon.INT_PHI`.
const INT_PHI: i32 = 0x9E37_79B9u32.cast_signed();
/// `Hash.DEFAULT_INITIAL_SIZE`.
const DEFAULT_INITIAL_SIZE: usize = 16;
/// `Hash.DEFAULT_LOAD_FACTOR`.
const DEFAULT_LOAD_FACTOR: f32 = 0.75;

/// `HashCommon.mix`.
const fn mix(x: i32) -> i32 {
    let h = x.wrapping_mul(INT_PHI);
    h ^ (h.cast_unsigned() >> 16).cast_signed()
}

/// `HashCommon.arraySize`.
fn array_size(expected: usize, load_factor: f32) -> usize {
    let needed = (expected as f64 / f64::from(load_factor)).ceil() as usize;
    needed.next_power_of_two().max(2)
}

/// `HashCommon.maxFill`.
fn max_fill(n: usize, load_factor: f32) -> usize {
    ((n as f64 * f64::from(load_factor)).ceil() as usize).min(n - 1)
}

/// The string keys of a fastutil `Object2ObjectOpenHashMap`, laid out like its table.
///
/// The same puts and removes leave them where fastutil would (linear probing, backward-shift
/// removal, growth and shrinking). Iteration follows the table from the first slot, like the
/// map's spliterator and streams.
#[derive(Clone, Debug)]
pub struct OpenHashKeys {
    table: Vec<Option<Box<str>>>,
    size: usize,
    min_n: usize,
    load_factor: f32,
}

impl Default for OpenHashKeys {
    /// `new Object2ObjectOpenHashMap()`.
    fn default() -> Self {
        Self::new(DEFAULT_INITIAL_SIZE, DEFAULT_LOAD_FACTOR)
    }
}

impl OpenHashKeys {
    /// `new Object2ObjectOpenHashMap(expected, loadFactor)`.
    #[must_use]
    pub fn new(expected: usize, load_factor: f32) -> Self {
        let n = array_size(expected, load_factor);
        Self {
            table: vec![None; n],
            size: 0,
            min_n: n,
            load_factor,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.size
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.find(key).is_some()
    }

    /// Adds `key` if it is absent, like `put` or `computeIfAbsent`.
    pub fn insert(&mut self, key: &str) {
        if self.contains(key) {
            return;
        }
        let mut pos = self.slot(key);
        while self.table[pos].is_some() {
            pos = (pos + 1) & self.mask();
        }
        self.table[pos] = Some(key.into());
        // `if (size++ >= maxFill)`
        let max_fill = max_fill(self.table.len(), self.load_factor);
        self.size += 1;
        if self.size > max_fill {
            self.rehash(array_size(self.size + 1, self.load_factor));
        }
    }

    /// Removes `key` if it is present, like `remove`.
    pub fn remove(&mut self, key: &str) {
        let Some(pos) = self.find(key) else {
            return;
        };
        self.size -= 1;
        self.shift_keys(pos);
        let n = self.table.len();
        if n > self.min_n
            && self.size < max_fill(n, self.load_factor) / 4
            && n > DEFAULT_INITIAL_SIZE
        {
            self.rehash(n / 2);
        }
    }

    /// The keys in the order of the map's spliterator and streams.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.table.iter().flatten().map(AsRef::as_ref)
    }

    /// The keys in the order of the map's iterators (and so of for-each loops), which walk the
    /// table from the last slot.
    pub fn iter_rev(&self) -> impl Iterator<Item = &str> {
        self.table.iter().rev().flatten().map(AsRef::as_ref)
    }

    const fn mask(&self) -> usize {
        self.table.len() - 1
    }

    fn slot(&self, key: &str) -> usize {
        mix(java_string_hash(key)).cast_unsigned() as usize & self.mask()
    }

    fn find(&self, key: &str) -> Option<usize> {
        let mut pos = self.slot(key);
        while let Some(current) = &self.table[pos] {
            if &**current == key {
                return Some(pos);
            }
            pos = (pos + 1) & self.mask();
        }
        None
    }

    /// `shiftKeys`: closes the gap a removal leaves in the probe sequence.
    fn shift_keys(&mut self, mut pos: usize) {
        let mask = self.mask();
        loop {
            let last = pos;
            pos = (last + 1) & mask;
            loop {
                let Some(current) = &self.table[pos] else {
                    self.table[last] = None;
                    return;
                };
                let slot = self.slot(current);
                let movable = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if movable {
                    break;
                }
                pos = (pos + 1) & mask;
            }
            self.table[last] = self.table[pos].take();
        }
    }

    /// `rehash`: reinserts the keys from the last slot of the old table to the first.
    fn rehash(&mut self, new_n: usize) {
        let old = std::mem::replace(&mut self.table, vec![None; new_n]);
        for key in old.into_iter().rev().flatten() {
            let mut pos = self.slot(&key);
            while self.table[pos].is_some() {
                pos = (pos + 1) & self.mask();
            }
            self.table[pos] = Some(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpenHashKeys;

    #[test]
    fn iterates_like_fastutil() {
        // Order fastutil 8.5.18 streams these keys in after the same operations, with the
        // scoreboard's `new Object2ObjectOpenHashMap(16, 0.5F)`.
        let mut keys = OpenHashKeys::new(16, 0.5);
        for i in 0..20 {
            keys.insert(&format!("Bot{i}"));
        }
        for i in 0..15 {
            keys.remove(&format!("Bot{i}"));
        }
        keys.insert("#p1");
        let order: Vec<&str> = keys.iter().collect();
        assert_eq!(order, ["Bot15", "Bot17", "Bot16", "Bot19", "Bot18", "#p1"]);
    }
}
