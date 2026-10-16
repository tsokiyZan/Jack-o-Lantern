//! A string-keyed map that iterates in the order of Java's `HashMap`, which is the order vanilla
//! writes and prints compound tags in.

// Keys come out as `&Box<str>` like they did from the `HashMap` this replaced.
#![allow(clippy::borrowed_box)]

use indexmap::IndexMap;

const DEFAULT_INITIAL_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// Java's `String.hashCode`, over UTF-16 code units.
#[must_use]
pub fn java_string_hash(string: &str) -> i32 {
    string.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

/// `HashMap.hash`: folds the high bits of the key's hash into the low ones.
const fn spread(hash: i32) -> u32 {
    let hash = hash as u32;
    hash ^ (hash >> 16)
}

/// A map that iterates like a Java `HashMap` given the same puts and removes.
///
/// It keeps entries in insertion order and tracks the table size the Java map would have. Java
/// iterates bucket by bucket, and each bucket holds its keys in
/// insertion order, so that is enough to reproduce its iteration order. Treeified buckets (eight
/// or more colliding keys in a table of 64 or more) are not modelled.
#[derive(Clone, Debug)]
pub struct JavaHashMap<V> {
    entries: IndexMap<Box<str>, V>,
    capacity: usize,
}

impl<V> Default for JavaHashMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: PartialEq> PartialEq for JavaHashMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<V> JavaHashMap<V> {
    /// Creates an empty map, like `new HashMap<>()`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: IndexMap::new(),
            capacity: DEFAULT_INITIAL_CAPACITY,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries.get_mut(key)
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// `HashMap.put`: replaces the value in place for an existing key, otherwise appends the key
    /// to its bucket and grows the table like `putVal`.
    pub fn insert(&mut self, key: Box<str>, value: V) -> Option<V> {
        if let Some(existing) = self.entries.get_mut(&*key) {
            return Some(std::mem::replace(existing, value));
        }
        if self.entries.len() >= TREEIFY_THRESHOLD && self.capacity < MIN_TREEIFY_CAPACITY {
            // `treeifyBin` resizes instead of treeifying while the table is small.
            let bucket = self.bucket_of(&key);
            let in_bucket = self
                .entries
                .keys()
                .filter(|k| self.bucket_of(k) == bucket)
                .count();
            if in_bucket >= TREEIFY_THRESHOLD {
                self.resize();
            }
        }
        self.entries.insert(key, value);
        if self.entries.len() > self.threshold() {
            self.resize();
        }
        None
    }

    /// `HashMap.remove`: the table never shrinks.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        self.entries.shift_remove(key)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn retain(&mut self, keep: impl FnMut(&Box<str>, &mut V) -> bool) {
        self.entries.retain(keep);
    }

    /// Iterates in Java `HashMap` order.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, V> {
        Iter {
            order: self.java_order().into_iter(),
            entries: &self.entries,
        }
    }

    /// Iterates in Java `HashMap` order.
    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        let order = self.java_order();
        let mut slots: Vec<Option<(&Box<str>, &mut V)>> =
            self.entries.iter_mut().map(Some).collect();
        let ordered: Vec<_> = order.into_iter().filter_map(|i| slots[i].take()).collect();
        IterMut {
            inner: ordered.into_iter(),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &Box<str>> {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.iter_mut().map(|(_, v)| v)
    }

    const fn threshold(&self) -> usize {
        if self.capacity >= MAXIMUM_CAPACITY {
            usize::MAX
        } else {
            self.capacity / 4 * 3
        }
    }

    const fn resize(&mut self) {
        if self.capacity < MAXIMUM_CAPACITY {
            self.capacity *= 2;
        }
    }

    fn bucket_of(&self, key: &str) -> usize {
        spread(java_string_hash(key)) as usize & (self.capacity - 1)
    }

    /// Insertion indices sorted by bucket; the sort is stable, so each bucket keeps insertion order.
    fn java_order(&self) -> Vec<usize> {
        let mut order: Vec<(usize, usize)> = self
            .entries
            .keys()
            .enumerate()
            .map(|(i, k)| (self.bucket_of(k), i))
            .collect();
        order.sort_by_key(|&(bucket, _)| bucket);
        order.into_iter().map(|(_, i)| i).collect()
    }
}

/// Iterator over a [`JavaHashMap`] in Java `HashMap` order.
pub struct Iter<'a, V> {
    order: std::vec::IntoIter<usize>,
    entries: &'a IndexMap<Box<str>, V>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a Box<str>, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.order.next().and_then(|i| self.entries.get_index(i))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.order.size_hint()
    }
}

impl<V> ExactSizeIterator for Iter<'_, V> {}

/// Mutable iterator over a [`JavaHashMap`] in Java `HashMap` order.
pub struct IterMut<'a, V> {
    inner: std::vec::IntoIter<(&'a Box<str>, &'a mut V)>,
}

impl<'a, V> Iterator for IterMut<'a, V> {
    type Item = (&'a Box<str>, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<V> IntoIterator for JavaHashMap<V> {
    type Item = (Box<str>, V);
    type IntoIter = std::vec::IntoIter<(Box<str>, V)>;

    /// Iterates in Java `HashMap` order.
    fn into_iter(self) -> Self::IntoIter {
        let order = self.java_order();
        let mut slots: Vec<Option<(Box<str>, V)>> = self.entries.into_iter().map(Some).collect();
        order
            .into_iter()
            .filter_map(|i| slots[i].take())
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl<'a, V> IntoIterator for &'a JavaHashMap<V> {
    type Item = (&'a Box<str>, &'a V);
    type IntoIter = Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, V> IntoIterator for &'a mut JavaHashMap<V> {
    type Item = (&'a Box<str>, &'a mut V);
    type IntoIter = IterMut<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<V> Extend<(Box<str>, V)> for JavaHashMap<V> {
    fn extend<T: IntoIterator<Item = (Box<str>, V)>>(&mut self, iter: T) {
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<V> FromIterator<(Box<str>, V)> for JavaHashMap<V> {
    fn from_iter<T: IntoIterator<Item = (Box<str>, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        map.extend(iter);
        map
    }
}

#[cfg(test)]
mod tests {
    use super::JavaHashMap;

    #[test]
    fn iterates_like_java() {
        // The order a vanilla 26.3 server printed this compound in, keys given in insertion order.
        let keys = [
            "k0", "k1", "k2", "k3", "k4", "k5", "k6", "k7", "k8", "k9", "k10", "k11", "k12", "k13",
            "Pos", "Motion", "Rotation", "UUID", "Health", "id",
        ];
        let map: JavaHashMap<()> = keys.iter().map(|&k| (k.into(), ())).collect();
        let order: Vec<&str> = map.keys().map(|k| &**k).collect();
        assert_eq!(
            order,
            [
                "Motion", "k0", "Health", "k1", "k2", "k3", "k4", "k5", "k11", "k6", "k10", "k7",
                "k13", "k8", "k12", "Rotation", "k9", "Pos", "id", "UUID"
            ]
        );
    }
}
