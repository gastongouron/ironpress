//! Process-lifetime memoization with a hard entry ceiling.
//!
//! Font setup repeats the same expensive work on every conversion: the same
//! byte buffers are parsed again and the same family names are resolved again.
//! Memoizing it across calls is what makes a warm process fast, but the keys
//! are supplied by the document, so the table needs a limit that the document
//! cannot raise.

use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::{Mutex, MutexGuard, PoisonError};

use lru::LruCache;

/// A memo table shared by every conversion in the process, capped at a fixed
/// number of entries.
///
/// A caller can register unlimited font blobs and CSS can name unlimited
/// families, so an unbounded table would let input decide how much memory the
/// process keeps for its whole life. Discarding the least recently used entry
/// keeps the working set of a repeated render hot while giving that growth a
/// ceiling.
///
/// Entries are owned here and handed out as clones, so a cached value never
/// borrows from the table and nothing has to outlive it.
pub(crate) struct BoundedProcessCache<K, V> {
    entries: Mutex<LruCache<K, V>>,
}

impl<K: Eq + Hash, V: Clone> BoundedProcessCache<K, V> {
    pub(crate) fn new(capacity: NonZeroUsize) -> Self {
        Self {
            entries: Mutex::new(LruCache::new(capacity)),
        }
    }

    /// The value stored for `key`, which becomes the most recently used entry.
    pub(crate) fn get(&self, key: &K) -> Option<V> {
        self.entries().get(key).cloned()
    }

    /// Store `value` for `key`, discarding the least recently used entry when
    /// the table is already full.
    ///
    /// An existing entry for `key` is replaced. That is what lets a caller
    /// whose key is a hash bucket rather than an identity treat an unwanted hit
    /// as a miss and put the right value in its place.
    pub(crate) fn insert(&self, key: K, value: V) {
        self.entries().put(key, value);
    }

    /// A cache holds only recomputable values, so a panic elsewhere cannot
    /// leave it semantically broken. Taking the guard back from a poisoned lock
    /// keeps one panicking render from turning every later one into a panic.
    fn entries(&self) -> MutexGuard<'_, LruCache<K, V>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(capacity: usize) -> BoundedProcessCache<u32, &'static str> {
        BoundedProcessCache::new(NonZeroUsize::new(capacity).expect("non-zero test capacity"))
    }

    #[test]
    fn returns_a_stored_value() {
        let cache = cache(2);
        cache.insert(1, "one");
        assert_eq!(cache.get(&1), Some("one"));
    }

    #[test]
    fn reports_a_key_that_was_never_stored() {
        let cache = cache(2);
        cache.insert(1, "one");
        assert_eq!(cache.get(&2), None);
    }

    #[test]
    fn holds_no_more_entries_than_its_capacity() {
        let cache = cache(2);
        for key in 0..10 {
            cache.insert(key, "value");
        }
        let resident = (0..10).filter(|key| cache.get(key).is_some()).count();
        assert_eq!(resident, 2);
    }

    #[test]
    fn evicts_the_least_recently_used_entry_when_full() {
        let cache = cache(2);
        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some("two"));
        assert_eq!(cache.get(&3), Some("three"));
    }

    #[test]
    fn a_read_protects_an_entry_from_the_next_eviction() {
        let cache = cache(2);
        cache.insert(1, "one");
        cache.insert(2, "two");
        assert_eq!(cache.get(&1), Some("one"));
        cache.insert(3, "three");
        assert_eq!(cache.get(&1), Some("one"));
        assert_eq!(cache.get(&2), None);
    }

    #[test]
    fn inserting_an_existing_key_replaces_its_value() {
        let cache = cache(2);
        cache.insert(1, "first");
        cache.insert(1, "second");
        assert_eq!(cache.get(&1), Some("second"));
    }
}
