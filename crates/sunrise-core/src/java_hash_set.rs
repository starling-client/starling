//! Java HashSet traversal, backed by the same buckets and tree bins as HashMap.
use crate::java_hash_map::{JavaHashMap, JavaKey};
use std::{borrow::Borrow, cmp::Ordering, fmt};

#[derive(Clone)]
pub struct JavaHashSet<K: JavaKey>(JavaHashMap<K, ()>);

impl<K: JavaKey> Default for JavaHashSet<K> {
    fn default() -> Self {
        Self(JavaHashMap::default())
    }
}

impl<K: JavaKey> JavaHashSet<K> {
    /// `new HashSet<>(collection)`: size from the complete source collection,
    /// including duplicate elements, then insert in traversal order.
    pub fn from_collection(keys: impl ExactSizeIterator<Item = K>) -> Self {
        let mut set = Self(JavaHashMap::with_expected_entries(keys.len().max(12)));
        set.extend(keys);
        set
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn contains<Q: JavaKey + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.0.get(key).is_some()
    }

    pub fn insert(&mut self, key: K) -> bool {
        self.0.put(key, ()).is_none()
    }

    pub fn remove<Q: JavaKey + ?Sized>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.0.remove(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.0.entries().map(|(key, ())| key)
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// An empty replacement that will resume with this set's bucket count,
    /// as `clear()` would. A decoder can prepare it without changing live state.
    pub fn cleared(&self) -> Self {
        Self(self.0.cleared())
    }

    /// Keep members in place, as `HashSet.removeIf` removes through its iterator.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let mut cursor = self.0.cursor();
        while cursor.has_next() {
            let (key, ()) = self.0.next_entry(&mut cursor).unwrap().unwrap();
            if !keep(key) {
                self.0.remove_current(&mut cursor).unwrap();
            }
        }
    }
}

impl<K: JavaKey> Extend<K> for JavaHashSet<K> {
    fn extend<T: IntoIterator<Item = K>>(&mut self, iter: T) {
        for key in iter {
            self.insert(key);
        }
    }
}

impl<K: JavaKey + fmt::Debug> fmt::Debug for JavaHashSet<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

// Stateful equality: callers can observe the order even with identical members.
impl<K: JavaKey> PartialEq for JavaHashSet<K> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

/// A UUID key in Minecraft's stored four-int representation. Java's UUID
/// compares the two signed 64-bit halves in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JavaUuid(pub [i32; 4]);

impl JavaKey for JavaUuid {
    fn java_hash(&self) -> i32 {
        let [a, b, c, d] = self.0;
        a ^ b ^ c ^ d
    }

    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        let halves = |Self([a, b, c, d])| ((a, b as u32), (c, d as u32));
        Some(halves(*self).cmp(&halves(*other)))
    }

    fn tie_break(&self, _other: &Self) -> Ordering {
        unreachable!("unequal UUIDs are Comparable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplementary_string_keys_follow_java_utf16_bucket_order() {
        let mut set = JavaHashSet::default();
        set.extend(["\u{1f600}", "a", "h", "p"].map(str::to_owned));
        // With Java's two-code-unit hash, the emoji and 'h' share a bucket.
        assert!(
            set.iter()
                .map(String::as_str)
                .eq(["p", "a", "\u{1f600}", "h"])
        );
        assert!(set.contains("\u{1f600}"));
        assert!(set.remove("\u{1f600}"));
    }

    #[test]
    fn uuid_tree_traversal_and_iterator_removal_match_jdk_25() {
        // These UUIDs share hashCode 0. Indices below are Java 25's HashSet
        // traversal before and after removeIf(u -> u.getLeastSignificantBits() >= 5).
        let keys = (0..16).map(|i| JavaUuid([i, 0, 0, i]));
        let decoded = JavaHashSet::from_collection(keys);
        let mut live = JavaHashSet::default();
        live.extend(decoded.iter().copied());
        assert_eq!(
            live.iter().map(|u| u.0[0]).collect::<Vec<_>>(),
            [3, 0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        );
        live.retain(|u| u.0[0] < 5);
        assert_eq!(
            live.iter().map(|u| u.0[0]).collect::<Vec<_>>(),
            [3, 0, 1, 2, 4]
        );
    }
}
