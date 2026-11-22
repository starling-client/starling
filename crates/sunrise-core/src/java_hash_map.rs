// Written for #6741 from a behaviour description, the Java SE API documentation and the iteration
// orders a real JVM produces; the red-black tree is the textbook one (CLRS, chapter 13).
//! Java HashMap traversal for non-null keys with pure hash/equality/order
//! operations. Tree bins preserve native linked iteration order.
//! This is the storage/iterator subset needed by simulation callback lifecycles.
//!
//! Layout: one slab block holds parallel hash/next, key and value arrays addressed by `u32` slot; slot 0 is a sentinel.
use std::{
    alloc::{self, Layout},
    borrow::Borrow,
    cmp::Ordering,
    marker::PhantomData,
    mem,
    ptr::{self, NonNull},
    sync::atomic::{self, AtomicU64},
};

pub trait JavaKey: Eq {
    fn java_hash(&self) -> i32;
    /// Comparable comparison when both keys have the appropriate same Java
    /// class. None and Equal both require subtree searches for unequal keys.
    fn comparable_compare(&self, _other: &Self) -> Option<Ordering> {
        None
    }
    /// Native class-name ordering, then identity-hash ordering. Equal identity
    /// hashes break to Less, even for two different objects. Never return Equal.
    fn tie_break(&self, other: &Self) -> Ordering;
}

impl JavaKey for str {
    fn java_hash(&self) -> i32 {
        crate::random::java_string_hash(self.encode_utf16())
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        Some(self.encode_utf16().cmp(other.encode_utf16()))
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        unreachable!("unequal strings are Comparable")
    }
}

impl JavaKey for String {
    fn java_hash(&self) -> i32 {
        self.as_str().java_hash()
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        self.as_str().comparable_compare(other.as_str())
    }
    fn tie_break(&self, other: &Self) -> Ordering {
        self.as_str().tie_break(other.as_str())
    }
}

/// No tree link: an absent child or parent.
const NIL: u32 = u32::MAX;
/// Slot 0: the end of a list and an empty bucket; its hash/next record is always readable.
const END: u32 = 0;
/// Set in a free slot's `next`; live links stay below it.
const FREE: u32 = 1 << 31;
/// The table the first insertion allocates, unless `copied` sized it.
const DEFAULT_CAPACITY: usize = 16;
/// The largest table; it never grows past this.
const MAX_CAPACITY: usize = 1 << 30;
/// A plain bin that already holds this many nodes when a new key is appended becomes a tree
/// bin, once the table has `MIN_TREE_CAPACITY` buckets; a smaller table grows instead.
const TREEIFY_LEN: usize = 8;
const MIN_TREE_CAPACITY: usize = 64;
/// When a tree bin splits, a half with at most this many nodes becomes a plain bin.
const UNTREEIFY_LEN: usize = 6;
/// The first slab holds the default table's 12 entries; tripling once, then doubling, keeps the 33-entry peak low.
const FIRST_SLAB: u32 = 12;

/// A slot's spread hash and the next slot of its bin's list.
#[derive(Clone, Copy)]
#[repr(C)]
struct Meta {
    hash: i32,
    next: u32,
}

static SENTINEL: Meta = Meta { hash: 0, next: END };
static NO_TABLE: u32 = END;

/// A tree bin node's red-black links.
#[derive(Clone, Copy)]
struct Links {
    parent: u32,
    left: u32,
    right: u32,
    red: bool,
    in_tree: bool,
}

impl Links {
    const UNUSED: Self = Self {
        parent: NIL,
        left: NIL,
        right: NIL,
        red: false,
        in_tree: false,
    };
    /// A plain bin's head whose bin a rehash will turn into a tree; never set on a tree node.
    const PENDING: Self = Self {
        red: true,
        ..Self::UNUSED
    };
    fn is_pending(&self) -> bool {
        self.red && !self.in_tree
    }
}

pub struct JavaHashMap<K, V> {
    /// `mask + 1` bucket heads; the shared one-bucket `NO_TABLE` until the first insertion.
    table: NonNull<u32>,
    mask: usize,
    /// `cap + 1` records, or the shared `SENTINEL` while `cap` is 0.
    meta: NonNull<Meta>,
    /// Slot i's key lives at index i - 1, its value at index i (index 0 is never written).
    keys: NonNull<K>,
    values: NonNull<V>,
    cap: u32,
    /// Slots 1..=used have been handed out since the slab was last emptied.
    used: u32,
    /// The first free slot, END when none.
    free: u32,
    len: u32,
    /// The table grows when a new key takes the size past this.
    threshold: u32,
    /// Structural generation. At u64::MAX it is exhausted and no cursor may access a slot.
    mod_count: u64,
    /// The capacity the first insertion allocates.
    initial_capacity: u32,
    /// Tells this map's cursors from other maps' cursors.
    id: u64,
    /// Tree links by slot: empty until the map makes a tree bin, then `cap + 1` long.
    links: Vec<Links>,
    _owns: PhantomData<(K, V)>,
}

// SAFETY: the map owns its keys and values; the raw pointers are unique or point at immutable statics,
// so moving the map to another thread moves everything it can reach.
unsafe impl<K: Send, V: Send> Send for JavaHashMap<K, V> {}
// SAFETY: as for `Send`; a shared reference reads the keys and values and the statics, and nothing else.
unsafe impl<K: Sync, V: Sync> Sync for JavaHashMap<K, V> {}

/// Borrowed Java bucket/list traversal, without a cursor or allocation.
pub struct Iter<'a, K, V> {
    map: &'a JavaHashMap<K, V>,
    bucket: usize,
    next: u32,
    remaining: usize,
}

impl<'a, K: JavaKey, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let table = self.map.table();
        while self.next == END {
            self.next = table[self.bucket];
            self.bucket += 1;
        }
        let i = self.next;
        self.next = self.map.meta(i).next;
        self.remaining -= 1;
        Some((self.map.key(i), self.map.value(i)))
    }
    #[inline]
    fn fold<B, F: FnMut(B, Self::Item) -> B>(self, mut acc: B, mut f: F) -> B {
        let map = self.map;
        let mut i = self.next;
        while i != END {
            acc = f(acc, (map.key(i), map.value(i)));
            i = map.meta(i).next;
        }
        for &head in &map.table()[self.bucket..] {
            let mut i = head;
            while i != END {
                acc = f(acc, (map.key(i), map.value(i)));
                i = map.meta(i).next;
            }
        }
        acc
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<K: JavaKey, V> ExactSizeIterator for Iter<'_, K, V> {}
impl<K: JavaKey, V> std::iter::FusedIterator for Iter<'_, K, V> {}

/// Consuming traversal moves keys and values out of the existing slab.
pub struct IntoIter<K, V> {
    map: JavaHashMap<K, V>,
    bucket: usize,
}
impl<K: JavaKey, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        if self.map.len == 0 {
            return None;
        }
        let mut i = self.map.table()[self.bucket];
        while i == END {
            self.bucket += 1;
            i = self.map.table()[self.bucket];
        }
        let next = self.map.meta(i).next;
        self.map.set_head(self.bucket, next);
        self.map.len -= 1;
        Some(self.map.take(i))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.map.len as usize;
        (len, Some(len))
    }
}
impl<K: JavaKey, V> ExactSizeIterator for IntoIter<K, V> {}
impl<K: JavaKey, V> std::iter::FusedIterator for IntoIter<K, V> {}

pub struct Entry<'a, K, V> {
    map: &'a mut JavaHashMap<K, V>,
    key: K,
    hash: i32,
    index: Option<u32>,
}
impl<'a, K: JavaKey, V> Entry<'a, K, V> {
    pub fn or_insert(self, value: V) -> &'a mut V {
        self.or_insert_with(|| value)
    }
    pub fn or_insert_with(self, make: impl FnOnce() -> V) -> &'a mut V {
        let index = self.index.unwrap_or_else(|| {
            let index = if self.map.free != END {
                self.map.free
            } else {
                self.map.used + 1
            };
            self.map.put_hashed(self.key, make(), self.hash);
            index
        });
        // The predicted slot must now be live: checked, because `value_mut` reads it unchecked.
        assert!(index != END && index <= self.map.used && self.map.meta(index).next & FREE == 0);
        self.map.value_mut(index)
    }
}

impl<K: JavaKey, V> IntoIterator for JavaHashMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            map: self,
            bucket: 0,
        }
    }
}

pub struct Cursor {
    map: u64,
    expected_mod_count: u64,
    next: u32,
    next_bucket: u32,
    current: u32,
    // Removal still uses the visited bucket when next has already crossed into a new bin.
    current_bucket: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorError {
    ConcurrentModification,
    NoCurrentEntry,
    WrongMap,
}

impl Cursor {
    pub fn has_next(&self) -> bool {
        self.next != END
    }
}

impl<K, V> Default for JavaHashMap<K, V> {
    fn default() -> Self {
        Self {
            table: NonNull::from(&NO_TABLE),
            mask: 0,
            meta: NonNull::from(&SENTINEL),
            keys: NonNull::dangling(),
            values: NonNull::dangling(),
            cap: 0,
            used: 0,
            free: END,
            len: 0,
            threshold: 0,
            mod_count: 0,
            initial_capacity: DEFAULT_CAPACITY as u32,
            id: next_id(),
            links: Vec::new(),
            _owns: PhantomData,
        }
    }
}

impl<K: JavaKey + Clone, V: Clone> Clone for JavaHashMap<K, V> {
    /// A Rust clone preserves the Java table state, unlike [`Self::copied`].
    fn clone(&self) -> Self {
        self.clone_with(Clone::clone)
    }
}

impl<K, V> Drop for JavaHashMap<K, V> {
    fn drop(&mut self) {
        if mem::needs_drop::<K>() || mem::needs_drop::<V>() {
            for i in 1..=self.used {
                if self.meta(i).next & FREE == 0 {
                    // SAFETY: a live slot holds an initialized key and value, dropped once here.
                    unsafe {
                        ptr::drop_in_place(self.key_ptr(i));
                        ptr::drop_in_place(self.value_ptr(i));
                    }
                }
            }
        }
        // SAFETY: both blocks were allocated with these layouts and nothing refers to them now.
        unsafe {
            if self.cap != 0 {
                alloc::dealloc(self.meta.as_ptr().cast(), Self::slab_layout(self.cap).0);
            }
            if self.mask != 0 {
                alloc::dealloc(self.table.as_ptr().cast(), table_layout(self.mask + 1));
            }
        }
    }
}

/// Each thread hands out map ids from its own block of 2^ID_BLOCK_BITS.
const ID_BLOCK_BITS: u32 = 20;
/// Blocks ever issued: the last block is never handed out, so ids never repeat and `id + block` never overflows.
const ID_BLOCKS: u64 = (1 << (64 - ID_BLOCK_BITS)) - 1;

/// The first id of a fresh block, or None (with `blocks` left unchanged) once the namespace is used up.
fn reserve_id_block(blocks: &AtomicU64) -> Option<u64> {
    let mut next = blocks.load(atomic::Ordering::Relaxed);
    loop {
        if next >= ID_BLOCKS {
            return None;
        }
        match blocks.compare_exchange_weak(
            next,
            next + 1,
            atomic::Ordering::Relaxed,
            atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return Some(next << ID_BLOCK_BITS),
            Err(seen) => next = seen,
        }
    }
}

/// Abort rather than panic: a caught panic must not let a later map reuse an id.
#[cold]
#[inline(never)]
fn id_blocks_exhausted() -> ! {
    std::process::abort()
}

/// A map id unique across threads for the life of the process.
fn next_id() -> u64 {
    static BLOCKS: AtomicU64 = AtomicU64::new(0);
    thread_local! {
        static NEXT: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    }
    NEXT.with(|next| {
        let (mut id, mut end) = next.get();
        if id == end {
            id = reserve_id_block(&BLOCKS).unwrap_or_else(|| id_blocks_exhausted());
            end = id + (1 << ID_BLOCK_BITS);
        }
        next.set((id + 1, end));
        id
    })
}

/// `h ^ (h >>> 16)`: the hash the map places a key by.
fn spread(hash: i32) -> i32 {
    hash ^ ((hash as u32) >> 16) as i32
}

/// Three quarters of the capacity; the largest table never grows.
fn threshold_for(capacity: usize) -> u32 {
    if capacity >= MAX_CAPACITY {
        u32::MAX
    } else {
        (capacity * 3 / 4) as u32
    }
}

fn table_layout(buckets: usize) -> Layout {
    Layout::array::<u32>(buckets).expect("table size")
}

fn new_table(buckets: usize) -> NonNull<u32> {
    let layout = table_layout(buckets);
    // Small tables fit the per-thread malloc cache on allocators where calloc bypasses it.
    // Large tables keep alloc_zeroed so an allocator can take advantage of demand-zero pages.
    let explicit_zero = buckets <= 128;
    // SAFETY: a table of at least two buckets has a non-zero size.
    let block = unsafe {
        if explicit_zero {
            alloc::alloc(layout)
        } else {
            alloc::alloc_zeroed(layout)
        }
    };
    let table =
        NonNull::new(block.cast::<u32>()).unwrap_or_else(|| alloc::handle_alloc_error(layout));
    if explicit_zero {
        // SAFETY: the allocation is exactly `buckets` u32 words.
        unsafe { table.as_ptr().write_bytes(0, buckets) };
    }
    table
}

impl<K, V> JavaHashMap<K, V> {
    #[inline(always)]
    fn table(&self) -> &[u32] {
        // SAFETY: `table` always holds `mask + 1` initialized heads.
        unsafe { std::slice::from_raw_parts(self.table.as_ptr(), self.mask + 1) }
    }
    #[inline(always)]
    fn head(&self, b: usize) -> u32 {
        debug_assert!(b <= self.mask);
        // SAFETY: callers pass a bucket index at most `mask`.
        unsafe { *self.table.as_ptr().add(b) }
    }
    #[inline(always)]
    fn set_head(&mut self, b: usize, i: u32) {
        assert!(self.mask != 0 && b <= self.mask);
        // SAFETY: an allocated table, never the shared NO_TABLE, and an in-bounds bucket.
        unsafe { *self.table.as_ptr().add(b) = i }
    }
    #[inline(always)]
    fn meta(&self, i: u32) -> Meta {
        debug_assert!(i <= self.cap);
        // SAFETY: list links, heads and the sentinel name records 0..=cap.
        unsafe { *self.meta.as_ptr().add(i as usize) }
    }
    /// Relinks list node `i`, which must be a live slot taken from a bin list (unchecked in release).
    #[inline(always)]
    fn set_next(&mut self, i: u32, next: u32) {
        debug_assert!(i != END && i <= self.used && self.meta(i).next & FREE == 0);
        // SAFETY: callers pass slots from a bin list, which are live slots of the map's own block.
        unsafe { (*self.meta.as_ptr().add(i as usize)).next = next };
    }
    #[inline(always)]
    fn meta_mut(&mut self, i: u32) -> &mut Meta {
        assert!(i != END && i <= self.used);
        // SAFETY: a handed-out slot of the map's own block.
        unsafe { &mut *self.meta.as_ptr().add(i as usize) }
    }
    #[inline(always)]
    fn key_ptr(&self, i: u32) -> *mut K {
        debug_assert!(i != END && i <= self.used);
        // SAFETY: slots 1..=cap map to array indices 0..cap.
        unsafe { self.keys.as_ptr().add(i as usize - 1) }
    }
    #[inline(always)]
    fn value_ptr(&self, i: u32) -> *mut V {
        debug_assert!(i != END && i <= self.used);
        // SAFETY: slots 1..=cap index the value array directly, which keeps the pointer provably non-null.
        unsafe { self.values.as_ptr().add(i as usize) }
    }
    #[inline(always)]
    fn key(&self, i: u32) -> &K {
        // SAFETY: only live slots reach here: those in a list, a cursor or a tree.
        unsafe { &*self.key_ptr(i) }
    }
    #[inline(always)]
    fn value(&self, i: u32) -> &V {
        // SAFETY: as for `key`.
        unsafe { &*self.value_ptr(i) }
    }
    #[inline(always)]
    fn value_mut(&mut self, i: u32) -> &mut V {
        // SAFETY: as for `key`, with the map borrowed exclusively.
        unsafe { &mut *self.value_ptr(i) }
    }
    #[inline(always)]
    fn key_value_mut(&mut self, i: u32) -> (&K, &mut V) {
        // SAFETY: as for `value_mut`; the key and value arrays do not overlap.
        unsafe { (&*self.key_ptr(i), &mut *self.value_ptr(i)) }
    }
    /// Moves a live slot's key and value out and frees the slot; unlink it first.
    fn take(&mut self, i: u32) -> (K, V) {
        assert!(self.meta(i).next & FREE == 0);
        // SAFETY: a live slot, marked free right after so nothing drops or reads it again.
        let entry = unsafe { (ptr::read(self.key_ptr(i)), ptr::read(self.value_ptr(i))) };
        let free = self.free;
        self.meta_mut(i).next = FREE | free;
        self.free = i;
        entry
    }

    /// The slab block layout for `cap` slots, and its key and value array offsets.
    fn slab_layout(cap: u32) -> (Layout, usize, usize) {
        Self::try_slab_layout(cap).expect("slab size")
    }
    fn try_slab_layout(cap: u32) -> Option<(Layout, usize, usize)> {
        let cap = cap as usize;
        let (layout, keys) = Layout::array::<Meta>(cap.checked_add(1)?)
            .and_then(|meta| meta.extend(Layout::array::<K>(cap)?))
            .ok()?;
        let (layout, values) = layout
            .extend(Layout::array::<V>(cap.checked_add(1)?).ok()?)
            .ok()?;
        Some((layout.pad_to_align(), keys, values))
    }
    /// Moves the slab into a block of `cap` slots; false if the allocator refused it.
    fn resize_slab(&mut self, cap: u32) -> bool {
        debug_assert!(cap > self.cap && cap < FREE);
        let Some((layout, keys, values)) = Self::try_slab_layout(cap) else {
            return false;
        };
        // SAFETY: the meta array alone makes the layout non-empty.
        let block = unsafe { alloc::alloc(layout) };
        if block.is_null() {
            return false;
        }
        // SAFETY: the new block has room for every handed-out slot; free slots copy uninitialized bytes.
        unsafe {
            let meta = block.cast::<Meta>();
            let (key_array, value_array) =
                (block.add(keys).cast::<K>(), block.add(values).cast::<V>());
            meta.write(Meta { hash: 0, next: END });
            if self.cap != 0 {
                let used = self.used as usize;
                ptr::copy_nonoverlapping(self.meta.as_ptr().add(1), meta.add(1), used);
                ptr::copy_nonoverlapping(self.keys.as_ptr(), key_array, used);
                ptr::copy_nonoverlapping(self.values.as_ptr().add(1), value_array.add(1), used);
                alloc::dealloc(self.meta.as_ptr().cast(), Self::slab_layout(self.cap).0);
            }
            self.meta = NonNull::new_unchecked(meta);
            self.keys = NonNull::new_unchecked(key_array);
            self.values = NonNull::new_unchecked(value_array);
        }
        self.cap = cap;
        if !self.links.is_empty() {
            self.links.resize(cap as usize + 1, Links::UNUSED);
        }
        true
    }
    #[cold]
    #[inline(never)]
    fn grow_slab(&mut self) {
        let cap = match self.cap {
            0 => FIRST_SLAB,
            FIRST_SLAB => 3 * FIRST_SLAB,
            cap => cap.saturating_mul(2).min(FREE - 1),
        };
        assert!(cap > self.cap, "JavaHashMap slab overflow");
        if !self.resize_slab(cap) {
            alloc::handle_alloc_error(Self::slab_layout(cap).0);
        }
    }
    #[inline]
    fn new_slot(&mut self, key: K, value: V, hash: i32) -> u32 {
        let i = if self.free != END {
            let i = self.free;
            self.free = self.meta(i).next & !FREE;
            i
        } else {
            if self.used == self.cap {
                self.grow_slab();
            }
            self.used += 1;
            self.used
        };
        // SAFETY: slot i is in the block and free; writing makes it live.
        unsafe {
            self.meta
                .as_ptr()
                .add(i as usize)
                .write(Meta { hash, next: END });
            self.key_ptr(i).write(key);
            self.value_ptr(i).write(value);
        }
        i
    }

    /// Clears the tree links of every node in the list from `head`, leaving a plain bin; no user code.
    fn untreeify(&mut self, mut head: u32) {
        while head != END {
            self.links[head as usize] = Links::UNUSED;
            head = self.meta(head).next;
        }
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Requested heap storage, excluding the keys' and values' own allocations.
    pub fn storage_bytes(&self) -> usize {
        let slab = if self.cap == 0 {
            0
        } else {
            Self::slab_layout(self.cap).0.size()
        };
        let table = if self.mask == 0 {
            0
        } else {
            (self.mask + 1) * size_of::<u32>()
        };
        slab.saturating_add(table)
            .saturating_add(self.links.capacity().saturating_mul(size_of::<Links>()))
    }
    pub(crate) fn values_in_storage_order(&self) -> impl Iterator<Item = &V> {
        (1..=self.used)
            .filter(|&i| self.meta(i).next & FREE == 0)
            .map(|i| self.value(i))
    }
}

impl<K: JavaKey, V> JavaHashMap<K, V> {
    /// `HashMap.newHashMap(expected)`: pre-size the table for this many entries,
    /// without allocating it until the first put.
    pub fn with_expected_entries(expected: usize) -> Self {
        let wanted = expected.saturating_mul(4).div_ceil(3);
        let mut map = Self::default();
        // Java sizes `newHashMap(0)` to one bucket and doubles it to two on the first put; start at
        // two, since a one-bucket table has mask 0, which here means "no table yet".
        map.initial_capacity = wanted.clamp(2, MAX_CAPACITY).next_power_of_two() as u32;
        map
    }
    /// Reserve physical entry storage without changing the Java table's initial capacity,
    /// growth thresholds or traversal order. An excessive size hint is ignored.
    pub fn with_reserved_nodes(entries: usize) -> Self {
        let mut map = Self::default();
        if entries > 0 && entries < FREE as usize {
            map.resize_slab(entries as u32);
        }
        map
    }
    /// Preserve map state while cloning values in slab order. Only structural
    /// ownership cloning uses this order; observable traversal uses `entries`.
    pub(crate) fn clone_with(&self, mut clone_value: impl FnMut(&V) -> V) -> Self
    where
        K: Clone,
    {
        let mut copy = Self::default();
        if self.cap != 0 && !copy.resize_slab(self.cap) {
            alloc::handle_alloc_error(Self::slab_layout(self.cap).0);
        }
        for i in 1..=self.used {
            let meta = self.meta(i);
            if meta.next & FREE == 0 {
                let (key, value) = (self.key(i).clone(), clone_value(self.value(i)));
                // SAFETY: slot i of the copy's block is unused until `used` covers it.
                unsafe {
                    copy.key_ptr_unchecked(i).write(key);
                    copy.value_ptr_unchecked(i).write(value);
                }
            }
            // SAFETY: as above; the record makes slot i live or free in the copy.
            unsafe { copy.meta.as_ptr().add(i as usize).write(meta) };
            copy.used = i;
        }
        copy.free = self.free;
        copy.links = self.links.clone();
        if self.mask != 0 {
            copy.table = new_table(self.mask + 1);
            copy.mask = self.mask;
            // SAFETY: both tables hold `mask + 1` heads.
            unsafe {
                ptr::copy_nonoverlapping(self.table.as_ptr(), copy.table.as_ptr(), self.mask + 1)
            };
        }
        copy.initial_capacity = self.initial_capacity;
        copy.threshold = self.threshold;
        copy.len = self.len;
        copy
    }
    fn key_ptr_unchecked(&self, i: u32) -> *mut K {
        // SAFETY: callers pass 1..=cap.
        unsafe { self.keys.as_ptr().add(i as usize - 1) }
    }
    fn value_ptr_unchecked(&self, i: u32) -> *mut V {
        // SAFETY: callers pass 1..=cap.
        unsafe { self.values.as_ptr().add(i as usize) }
    }
    /// Native new HashMap(source), including sizing before entry insertion.
    /// Values are cloned according to their storage semantics (shared handles
    /// retain identity). This may change bucket order relative to the source.
    pub fn copied(&self) -> Self
    where
        K: Clone,
        V: Clone,
    {
        Self::from_copy_entries(
            self.len(),
            self.entries().map(|(k, v)| (k.clone(), v.clone())),
        )
    }
    /// Rebuild at the Java copy-constructor capacity, moving the owned entries.
    pub(crate) fn into_copy(self) -> Self {
        Self::from_copy_entries(self.len(), self)
    }
    fn from_copy_entries(size: usize, entries: impl IntoIterator<Item = (K, V)>) -> Self {
        if size == 0 {
            return Self::default();
        }
        let mut copy = Self::with_expected_entries(size);
        let slots = u32::try_from(size)
            .ok()
            .filter(|&n| n < FREE)
            .expect("JavaHashMap slab overflow");
        if !copy.resize_slab(slots) {
            alloc::handle_alloc_error(Self::slab_layout(slots).0);
        }
        for (key, value) in entries {
            copy.put(key, value);
        }
        copy
    }
    pub(crate) fn cleared(&self) -> Self {
        let mut map = Self::default();
        map.initial_capacity = if self.mask == 0 {
            self.initial_capacity
        } else {
            (self.mask + 1) as u32
        };
        map
    }
    #[inline]
    pub fn get<Q: JavaKey + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let i = self.find_slot(key, spread(key.java_hash()))?;
        Some(self.value(i))
    }
    pub fn entries(&self) -> Iter<'_, K, V> {
        Iter {
            map: self,
            bucket: 0,
            next: END,
            remaining: self.len(),
        }
    }
    pub fn get_mut<Q: JavaKey + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        let i = self.find_slot(key, spread(key.java_hash()))?;
        Some(self.value_mut(i))
    }
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        let hash = spread(key.java_hash());
        let index = self.find_slot(&key, hash);
        Entry {
            map: self,
            key,
            hash,
            index,
        }
    }
    /// Mutate values in Java traversal order without allocating or cloning keys.
    /// The callback cannot structurally change the map or retain a mutable value.
    pub fn for_each_mut(&mut self, mut visit: impl FnMut(&K, &mut V)) {
        let mut next = if self.len == 0 {
            END
        } else {
            self.first_node_from(0)
        };
        while next != END {
            let index = next;
            next = self.successor(index);
            let (key, value) = self.key_value_mut(index);
            visit(key, value);
        }
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        let mut cursor = self.cursor();
        while cursor.has_next() {
            self.next_entry(&mut cursor)
                .expect("exclusive traversal")
                .expect("entry");
            let (key, value) = self.key_value_mut(cursor.current);
            if !keep(key, value) {
                self.remove_current(&mut cursor)
                    .expect("exclusive traversal");
            }
        }
    }
    /// Java HashMap.computeIfAbsent for a non-null result. Unlike `put`, this
    /// grows before the lookup and inserts at the head of a plain bin.
    pub fn compute_if_absent(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        if self.mask == 0 || self.len > self.threshold {
            self.grow();
        }
        let hash = spread(key.java_hash());
        if let Some(index) = self.find_slot(&key, hash) {
            return self.value_mut(index);
        }
        let b = hash as u32 as usize & self.mask;
        let head = self.head(b);
        let value = make();
        if let Some(root) = self.root(head) {
            let index = self.tree_put(b, root, key, value, hash);
            self.mod_count = self.mod_count.saturating_add(1);
            self.len += 1;
            self.value_mut(index)
        } else {
            let mut count = 0;
            let mut node = head;
            while node != END {
                count += 1;
                node = self.meta(node).next;
            }
            let index = self.new_slot(key, value, hash);
            self.meta_mut(index).next = head;
            self.set_head(b, index);
            self.mod_count = self.mod_count.saturating_add(1);
            self.len += 1;
            if count >= TREEIFY_LEN - 1 {
                if self.mask + 1 < MIN_TREE_CAPACITY {
                    self.grow();
                } else {
                    self.treeify(b);
                }
            }
            self.value_mut(index)
        }
    }
    #[inline(always)]
    pub fn put(&mut self, key: K, value: V) -> Option<V> {
        let hash = spread(key.java_hash());
        self.put_hashed(key, value, hash)
    }
    #[inline(always)]
    fn put_hashed(&mut self, key: K, value: V, hash: i32) -> Option<V> {
        if self.mask == 0 {
            self.grow();
        }
        let b = hash as u32 as usize & self.mask;
        let head = self.head(b);
        let i = if head == END {
            self.new_slot(key, value, hash)
        } else {
            let first = self.meta(head);
            if first.next != END || first.hash == hash || !self.links.is_empty() {
                return self.put_in_bin(b, head, key, value, hash);
            }
            let i = self.new_slot(key, value, hash);
            self.meta_mut(head).next = i;
            i
        };
        if head == END {
            // SAFETY: `b` was masked with this allocated table's `mask` above, and nothing since resized it.
            unsafe { *self.table.as_ptr().add(b) = i };
        }
        self.added_slot();
        None
    }
    #[inline(always)]
    fn added_slot(&mut self) {
        self.mod_count = self.mod_count.saturating_add(1);
        self.len += 1;
        if self.len > self.threshold {
            self.grow();
        }
    }
    #[cold]
    #[inline(never)]
    fn put_in_bin(&mut self, b: usize, head: u32, key: K, value: V, hash: i32) -> Option<V> {
        if !self.links.is_empty() && self.root(head).is_some() {
            return self.tree_put_or_replace(b, head, key, value, hash);
        }
        let mut len = 1;
        let mut last = head;
        loop {
            let meta = self.meta(last);
            if meta.hash == hash && *self.key(last) == key {
                return Some(mem::replace(self.value_mut(last), value));
            }
            if meta.next == END {
                break;
            }
            last = meta.next;
            len += 1;
        }
        let new = self.new_slot(key, value, hash);
        self.meta_mut(last).next = new;
        // Count the node before treeify runs user comparisons, so an unwind leaves `len` and cursors exact.
        self.mod_count = self.mod_count.saturating_add(1);
        self.len += 1;
        if len >= TREEIFY_LEN {
            if self.mask + 1 < MIN_TREE_CAPACITY {
                self.grow();
            } else {
                self.treeify(b);
            }
        }
        if self.len > self.threshold {
            self.grow();
        }
        None
    }
    #[inline(never)]
    fn tree_put_or_replace(
        &mut self,
        b: usize,
        head: u32,
        key: K,
        value: V,
        hash: i32,
    ) -> Option<V> {
        let root = self.root(head).expect("tree bin");
        let found = self.tree_find(root, hash, &key);
        if found != NIL {
            return Some(mem::replace(self.value_mut(found), value));
        }
        self.tree_put(b, root, key, value, hash);
        self.mod_count = self.mod_count.saturating_add(1);
        self.len += 1;
        if self.len > self.threshold {
            self.grow();
        }
        None
    }
    pub fn remove<Q: JavaKey + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        let hash = spread(key.java_hash());
        let i = self.find_slot(key, hash)?;
        Some(self.remove_node(hash as u32 as usize & self.mask, i, true))
    }
    pub fn clear(&mut self) {
        self.mod_count = self.mod_count.saturating_add(1);
        if self.len > 0 {
            self.len = 0;
            // SAFETY: a non-empty map has its own table of `mask + 1` heads.
            unsafe { ptr::write_bytes(self.table.as_ptr(), 0, self.mask + 1) };
        }
        self.links.clear();
        let used = mem::replace(&mut self.used, 0);
        self.free = END;
        for i in 1..=used {
            // SAFETY: slots 1..=used are in the block; `used` is already 0, so a panicking drop only leaks.
            unsafe {
                let meta = &mut *self.meta.as_ptr().add(i as usize);
                if meta.next & FREE == 0 {
                    meta.next = FREE;
                    ptr::drop_in_place(self.key_ptr_unchecked(i));
                    ptr::drop_in_place(self.value_ptr_unchecked(i));
                }
            }
        }
    }
    pub fn cursor(&self) -> Cursor {
        let (next_bucket, next) = self.cursor_head_from(0);
        Cursor {
            map: self.id,
            expected_mod_count: self.mod_count,
            next,
            next_bucket,
            current: END,
            current_bucket: 0,
        }
    }
    /// Call while has_next is true. The returned values reflect replacements made
    /// after iterator creation; structural changes are rejected before access.
    #[inline]
    pub fn next_entry(&self, c: &mut Cursor) -> Result<Option<(&K, &V)>, CursorError> {
        if c.map != self.id {
            return Err(CursorError::WrongMap);
        }
        if c.expected_mod_count != self.mod_count || self.mod_count == u64::MAX {
            return Err(CursorError::ConcurrentModification);
        }
        if c.next == END {
            return Ok(None);
        }
        let i = c.next;
        c.current = i;
        c.current_bucket = c.next_bucket;
        let next = self.meta(i).next;
        if next != END {
            c.next = next;
        } else {
            (c.next_bucket, c.next) = self.cursor_head_from(c.next_bucket as usize + 1);
        }
        Ok(Some((self.key(i), self.value(i))))
    }
    #[inline(always)]
    pub fn remove_current(&mut self, c: &mut Cursor) -> Result<(), CursorError> {
        if c.map != self.id {
            return Err(CursorError::WrongMap);
        }
        if c.current == END {
            return Err(CursorError::NoCurrentEntry);
        }
        if c.expected_mod_count != self.mod_count || self.mod_count == u64::MAX {
            return Err(CursorError::ConcurrentModification);
        }
        let i = mem::replace(&mut c.current, END);
        let b = c.current_bucket as usize;
        // Inline only the common case: the visited node heads a plain bin.
        if self.links.is_empty() && self.head(b) == i {
            let next = self.meta(i).next;
            self.set_head(b, next);
            self.mod_count = self.mod_count.saturating_add(1);
            self.len -= 1;
            drop(self.take(i));
        } else {
            self.remove_node_outlined(b, i);
        }
        c.expected_mod_count = self.mod_count;
        Ok(())
    }
    #[inline(never)]
    fn remove_node_outlined(&mut self, b: usize, i: u32) {
        self.remove_node(b, i, false);
    }

    /// The live slot whose key equals `key`.
    #[inline(always)]
    fn find_slot<Q: JavaKey + ?Sized>(&self, key: &Q, hash: i32) -> Option<u32>
    where
        K: Borrow<Q>,
    {
        let head = self.head(hash as u32 as usize & self.mask);
        if head == END {
            return None;
        }
        // After any `eq` call the search continues out of line, so `hash` is never live across one.
        let first = self.meta(head);
        if first.hash == hash {
            return if self.key(head).borrow() == key {
                Some(head)
            } else {
                self.find_after(head, self.meta(head).next, key)
            };
        }
        if first.next == END {
            return None;
        }
        if !self.links.is_empty() {
            return self.find_after(head, first.next, key);
        }
        let second = self.meta(first.next);
        if second.hash == hash {
            return if self.key(first.next).borrow() == key {
                Some(first.next)
            } else {
                self.find_after(head, self.meta(first.next).next, key)
            };
        }
        if second.next == END {
            return None;
        }
        self.find_after(head, second.next, key)
    }
    /// The rest of the search: the tree of `head`'s bin, or its list from slot `i` (END: none left).
    #[inline(never)]
    fn find_after<Q: JavaKey + ?Sized>(&self, head: u32, mut i: u32, key: &Q) -> Option<u32>
    where
        K: Borrow<Q>,
    {
        let hash = spread(key.java_hash());
        if let Some(root) = self.tree_root(head) {
            let index = self.tree_find(root, hash, key);
            return (index != NIL).then_some(index);
        }
        while i != END {
            let meta = self.meta(i);
            if meta.hash == hash && self.key(i).borrow() == key {
                return Some(i);
            }
            i = meta.next;
        }
        None
    }
    #[inline(always)]
    fn tree_root(&self, head: u32) -> Option<u32> {
        if self.links.is_empty() {
            None
        } else {
            self.root(head)
        }
    }
    /// The actual tree root, even if a cursor removed nodes without moving it to the front.
    fn root(&self, head: u32) -> Option<u32> {
        let links = self.links.get(head as usize)?;
        if !links.in_tree {
            return None;
        }
        let mut i = head;
        let mut parent = links.parent;
        while parent != NIL {
            i = parent;
            parent = self.links[i as usize].parent;
        }
        Some(i)
    }
    /// The node of the key equal to `key` in the subtree at `p`. Trees are ordered by hash
    /// first, so a different hash rules out one side; nodes with the same hash can be on both.
    fn tree_find<Q: JavaKey + ?Sized>(&self, mut p: u32, hash: i32, key: &Q) -> u32
    where
        K: Borrow<Q>,
    {
        while p != NIL {
            let node_hash = self.meta(self.tree_node(p)).hash;
            let node_key = self.key(p);
            let links = self.links[p as usize];
            p = match node_hash.cmp(&hash) {
                Ordering::Greater => links.left,
                Ordering::Less => links.right,
                Ordering::Equal if node_key.borrow() == key => return p,
                Ordering::Equal => {
                    let found = self.tree_find(links.right, hash, key);
                    if found != NIL {
                        return found;
                    }
                    links.left
                }
            };
        }
        NIL
    }
    /// Whether `key`, with spread hash `hash`, goes left of tree node `p`: by hash, then by
    /// `comparable_compare`, then by `tie_break` when that is absent or equal.
    /// Checks that tree link `p` names a live slot before its key is read: a broken invariant panics instead.
    fn tree_node(&self, p: u32) -> u32 {
        assert!(
            p != END && p <= self.used && self.meta(p).next & FREE == 0,
            "tree link to a dead slot"
        );
        p
    }
    fn goes_left(&self, key: &K, hash: i32, p: u32) -> bool {
        match self.meta(self.tree_node(p)).hash.cmp(&hash) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => {
                let node_key = self.key(p);
                match key.comparable_compare(node_key) {
                    Some(Ordering::Less) => true,
                    Some(Ordering::Greater) => false,
                    _ => key.tie_break(node_key) != Ordering::Greater,
                }
            }
        }
    }
    /// RB-INSERT of node `i` into the tree at `root` (NIL for an empty tree), descending by
    /// `goes_left`, then RB-INSERT-FIXUP. Returns the new root and the node `i` hangs from.
    fn attach(&mut self, root: u32, i: u32) -> (u32, u32) {
        let (parent, to_left) = self.attach_point(root, self.key(i), self.meta(i).hash);
        self.link_into_tree(root, i, parent, to_left)
    }
    /// Where RB-INSERT hangs a node with `key` and `hash`: its parent (NIL for an empty tree) and side.
    fn attach_point(&self, root: u32, key: &K, hash: i32) -> (u32, bool) {
        let mut parent = NIL;
        let mut to_left = false;
        let mut p = root;
        while p != NIL {
            parent = p;
            to_left = self.goes_left(key, hash, p);
            let links = self.links[p as usize];
            p = if to_left { links.left } else { links.right };
        }
        (parent, to_left)
    }
    /// Links node `i` under `parent` and runs RB-INSERT-FIXUP; no user code. Returns the new root and `parent`.
    fn link_into_tree(&mut self, root: u32, i: u32, parent: u32, to_left: bool) -> (u32, u32) {
        self.links[i as usize] = Links {
            parent,
            red: true,
            in_tree: true,
            ..Links::UNUSED
        };
        let mut tree = Tree {
            links: &mut self.links,
            root,
            nil_parent: NIL,
        };
        if parent == NIL {
            tree.root = i;
        } else if to_left {
            tree.at(parent).left = i;
        } else {
            tree.at(parent).right = i;
        }
        tree.insert_fixup(i);
        (tree.root, parent)
    }
    /// Turns bucket `b`'s plain bin into a tree bin: its nodes go into a new tree one by one in
    /// list order, then the root moves to the front. The list order is otherwise unchanged.
    fn treeify(&mut self, b: usize) {
        if self.links.len() <= self.cap as usize {
            self.links.resize(self.cap as usize + 1, Links::UNUSED);
        }
        // If a user comparison unwinds, the whole bin goes back to a plain list with no tree links.
        struct PlainOnUnwind<'m, K, V> {
            map: &'m mut JavaHashMap<K, V>,
            b: usize,
            armed: bool,
        }
        impl<K, V> Drop for PlainOnUnwind<'_, K, V> {
            fn drop(&mut self) {
                if self.armed {
                    let head = self.map.head(self.b);
                    self.map.untreeify(head);
                }
            }
        }
        let mut guard = PlainOnUnwind {
            map: self,
            b,
            armed: true,
        };
        let map = &mut *guard.map;
        let mut root = NIL;
        let mut i = map.head(b);
        while i != END {
            root = map.attach(root, i).0;
            i = map.meta(i).next;
        }
        map.move_root_to_front(b, root);
        guard.armed = false;
    }
    /// A new key into the tree bin of bucket `b`: it joins the tree, and the list right after
    /// its tree parent.
    fn tree_put(&mut self, b: usize, root: u32, key: K, value: V, hash: i32) -> u32 {
        // All user comparisons happen before the slot exists, so an unwind leaves the map untouched.
        let (parent, to_left) = self.attach_point(root, &key, hash);
        let new = self.new_slot(key, value, hash);
        let (root, parent) = self.link_into_tree(root, new, parent, to_left);
        let after = self.meta(parent).next;
        self.meta_mut(new).next = after;
        self.meta_mut(parent).next = new;
        self.move_root_to_front(b, root);
        new
    }
    /// Puts the tree's root first in the bin's list, the other nodes keeping their order.
    fn move_root_to_front(&mut self, b: usize, root: u32) {
        let root = self.tree_node(root);
        let head = self.head(b);
        if root != head {
            self.unlink(b, root);
            self.meta_mut(root).next = head;
            self.set_head(b, root);
        }
    }
    /// Takes node `i` out of bucket `b`'s list; the others keep their order.
    #[inline]
    fn unlink(&mut self, b: usize, i: u32) {
        let next = self.meta(i).next;
        let head = self.head(b);
        if head == i {
            self.set_head(b, next);
            return;
        }
        let mut p = head;
        while self.meta(p).next != i {
            p = self.meta(p).next;
        }
        self.meta_mut(p).next = next;
    }
    /// Removes node `i` from bucket `b`. In a tree bin, a movable removal (`remove`) turns a
    /// small tree back into a plain bin, judged on the tree before the removal, and otherwise
    /// moves the new root to the front; a cursor's removal does neither.
    #[inline]
    fn remove_node(&mut self, b: usize, i: u32, movable: bool) -> V {
        let root = self.tree_root(self.head(b));
        self.unlink(b, i);
        if let Some(root) = root {
            self.remove_tree_node(b, i, root, movable);
        }
        self.mod_count = self.mod_count.saturating_add(1);
        self.len -= 1;
        let (key, value) = self.take(i);
        drop(key);
        value
    }
    #[cold]
    fn remove_tree_node(&mut self, b: usize, i: u32, root: u32, movable: bool) {
        let head = self.head(b);
        if head != END {
            let small = movable && {
                let top = self.links[root as usize];
                top.right == NIL || top.left == NIL || self.links[top.left as usize].left == NIL
            };
            if small {
                self.untreeify(head);
            } else {
                assert!(
                    self.links[i as usize].in_tree,
                    "tree bin holds a node outside its tree"
                );
                let mut tree = Tree {
                    links: &mut self.links,
                    root,
                    nil_parent: NIL,
                };
                tree.delete(i);
                let root = tree.root;
                if movable {
                    self.move_root_to_front(b, root);
                }
            }
        }
        self.links[i as usize] = Links::UNUSED;
    }
    /// The node after `i` in iteration order.
    fn successor(&self, i: u32) -> u32 {
        let meta = self.meta(i);
        if meta.next != END {
            meta.next
        } else {
            self.first_node_from((meta.hash as u32 as usize & self.mask) + 1)
        }
    }
    fn cursor_head_from(&self, b: usize) -> (u32, u32) {
        self.table()[b..]
            .iter()
            .position(|&head| head != END)
            .map_or((0, END), |offset| {
                ((offset + b) as u32, self.head(offset + b))
            })
    }
    /// The first node of the first non-empty bucket from `b` on.
    fn first_node_from(&self, b: usize) -> u32 {
        self.table()[b..]
            .iter()
            .copied()
            .find(|&head| head != END)
            .unwrap_or(END)
    }
    /// Allocates the first table, or doubles it. Old bucket `j` splits into `j` and `j + old`
    /// by the hash bit `old`, each half in its old order; a single node keeps its bin kind.
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        if self.mask == 0 {
            let capacity = self.initial_capacity as usize;
            self.table = new_table(capacity);
            self.mask = capacity - 1;
            self.threshold = threshold_for(capacity);
            return;
        }
        let old = self.mask + 1;
        if old >= MAX_CAPACITY {
            self.threshold = u32::MAX;
            return;
        }
        let old_table = mem::replace(&mut self.table, new_table(old * 2));
        self.mask = old * 2 - 1;
        // Phase 1 runs no user code: every bin is split, and halves due for a tree are left plain and marked.
        let mut pending = false;
        for j in 0..old {
            // SAFETY: the old table still holds `old` heads; it is freed after this loop.
            let head = unsafe { *old_table.as_ptr().add(j) };
            if head == END {
                continue;
            }
            let first = self.meta(head);
            if first.next == END {
                self.set_head(first.hash as u32 as usize & self.mask, head);
                continue;
            }
            if self
                .links
                .get(head as usize)
                .is_some_and(|link| link.in_tree)
            {
                pending |= self.split_tree_bin(j, head, old);
                continue;
            }
            let [(lo, _), (hi, _)] = self.split_list(head, old);
            if lo != END {
                self.set_head(j, lo);
            }
            if hi != END {
                self.set_head(j + old, hi);
            }
        }
        // SAFETY: allocated by `new_table(old)`; nothing refers to it now.
        unsafe { alloc::dealloc(old_table.as_ptr().cast(), table_layout(old)) };
        self.threshold = threshold_for(old * 2);
        // Phase 2: the map is consistent; each treeify that unwinds leaves its own bin plain.
        if pending {
            self.treeify_pending();
        }
    }
    /// Splits tree bin `j` of an `old`-bucket table during phase 1, with no user code; true if a half awaits treeify.
    #[cold]
    #[inline(never)]
    fn split_tree_bin(&mut self, j: usize, head: u32, old: usize) -> bool {
        let halves = self.split_list(head, old);
        let mut pending = false;
        for (half, &(head, len)) in halves.iter().enumerate() {
            if head == END {
                continue;
            }
            self.set_head(j + half * old, head);
            if len <= UNTREEIFY_LEN {
                self.untreeify(head);
            } else if halves[1 - half].0 != END {
                self.untreeify(head);
                self.links[head as usize] = Links::PENDING;
                pending = true;
            }
            // If the whole tree landed here, its links and head stay unchanged.
        }
        pending
    }
    #[cold]
    #[inline(never)]
    fn treeify_pending(&mut self) {
        // On unwind the remaining marks are cleared too, so no PENDING link outlives `grow`.
        struct ClearMarks<'m, K, V>(&'m mut JavaHashMap<K, V>);
        impl<K, V> Drop for ClearMarks<'_, K, V> {
            fn drop(&mut self) {
                for b in 0..=self.0.mask {
                    let head = self.0.head(b);
                    if head != END && self.0.links[head as usize].is_pending() {
                        self.0.links[head as usize] = Links::UNUSED;
                    }
                }
            }
        }
        let guard = ClearMarks(self);
        for b in 0..=guard.0.mask {
            let head = guard.0.head(b);
            if head != END && guard.0.links[head as usize].is_pending() {
                guard.0.links[head as usize] = Links::UNUSED;
                guard.0.treeify(b);
            }
        }
    }
    /// Splits the list at `head` into the nodes whose hash has `bit` clear and those with it
    /// set, each in its old order. Returns each half's first node and length.
    #[inline]
    fn split_list(&mut self, head: u32, bit: usize) -> [(u32, usize); 2] {
        // Scalars rather than two-element arrays keep both halves in registers.
        let (mut lo_head, mut lo_tail, mut lo_len) = (END, END, 0);
        let (mut hi_head, mut hi_tail, mut hi_len) = (END, END, 0);
        let mut i = head;
        while i != END {
            let meta = self.meta(i);
            let hi = meta.hash as u32 as usize & bit != 0;
            let tail = if hi { hi_tail } else { lo_tail };
            if tail == END {
                if hi {
                    hi_head = i;
                } else {
                    lo_head = i;
                }
            } else {
                self.set_next(tail, i);
            }
            if hi {
                hi_tail = i;
                hi_len += 1;
            } else {
                lo_tail = i;
                lo_len += 1;
            }
            i = meta.next;
        }
        if lo_tail != END {
            self.set_next(lo_tail, END);
        }
        if hi_tail != END {
            self.set_next(hi_tail, END);
        }
        [(lo_head, lo_len), (hi_head, hi_len)]
    }
}

/// One tree bin's red-black tree, as in CLRS chapter 13. NIL plays the sentinel `T.nil`: it
/// reads black, and `nil_parent` is the `T.nil.p` that RB-DELETE sets for RB-DELETE-FIXUP.
struct Tree<'a> {
    links: &'a mut [Links],
    root: u32,
    nil_parent: u32,
}

impl Tree<'_> {
    fn at(&mut self, x: u32) -> &mut Links {
        &mut self.links[x as usize]
    }
    fn left(&self, x: u32) -> u32 {
        self.links[x as usize].left
    }
    fn right(&self, x: u32) -> u32 {
        self.links[x as usize].right
    }
    fn parent(&self, x: u32) -> u32 {
        if x == NIL {
            self.nil_parent
        } else {
            self.links[x as usize].parent
        }
    }
    fn set_parent(&mut self, x: u32, parent: u32) {
        if x == NIL {
            self.nil_parent = parent;
        } else {
            self.at(x).parent = parent;
        }
    }
    fn is_red(&self, x: u32) -> bool {
        x != NIL && self.links[x as usize].red
    }
    fn set_red(&mut self, x: u32, red: bool) {
        if x != NIL {
            self.at(x).red = red;
        }
    }
    /// Points `parent`'s link to its child `old` (the root's link when `parent` is NIL) at `new`.
    fn replace_child(&mut self, parent: u32, old: u32, new: u32) {
        if parent == NIL {
            self.root = new;
        } else if self.left(parent) == old {
            self.at(parent).left = new;
        } else {
            self.at(parent).right = new;
        }
    }
    /// LEFT-ROTATE (CLRS 13.2).
    fn rotate_left(&mut self, x: u32) {
        let y = self.right(x);
        let beta = self.left(y);
        self.at(x).right = beta;
        if beta != NIL {
            self.at(beta).parent = x;
        }
        let parent = self.parent(x);
        self.at(y).parent = parent;
        self.replace_child(parent, x, y);
        self.at(y).left = x;
        self.at(x).parent = y;
    }
    /// RIGHT-ROTATE: LEFT-ROTATE with left and right exchanged.
    fn rotate_right(&mut self, x: u32) {
        let y = self.left(x);
        let beta = self.right(y);
        self.at(x).left = beta;
        if beta != NIL {
            self.at(beta).parent = x;
        }
        let parent = self.parent(x);
        self.at(y).parent = parent;
        self.replace_child(parent, x, y);
        self.at(y).right = x;
        self.at(x).parent = y;
    }
    /// RB-INSERT-FIXUP (CLRS 13.3) for the red node `z` just attached.
    fn insert_fixup(&mut self, mut z: u32) {
        while self.is_red(self.parent(z)) {
            let zp = self.parent(z);
            let zpp = self.parent(zp);
            if zp == self.left(zpp) {
                let y = self.right(zpp);
                if self.is_red(y) {
                    self.set_red(zp, false);
                    self.set_red(y, false);
                    self.set_red(zpp, true);
                    z = zpp;
                } else {
                    if z == self.right(zp) {
                        z = zp;
                        self.rotate_left(z);
                    }
                    let zp = self.parent(z);
                    let zpp = self.parent(zp);
                    self.set_red(zp, false);
                    self.set_red(zpp, true);
                    self.rotate_right(zpp);
                }
            } else {
                let y = self.left(zpp);
                if self.is_red(y) {
                    self.set_red(zp, false);
                    self.set_red(y, false);
                    self.set_red(zpp, true);
                    z = zpp;
                } else {
                    if z == self.left(zp) {
                        z = zp;
                        self.rotate_right(z);
                    }
                    let zp = self.parent(z);
                    let zpp = self.parent(zp);
                    self.set_red(zp, false);
                    self.set_red(zpp, true);
                    self.rotate_left(zpp);
                }
            }
        }
        let root = self.root;
        self.set_red(root, false);
    }
    /// RB-TRANSPLANT (CLRS 13.4): `v` takes `u`'s place under `u`'s parent.
    fn transplant(&mut self, u: u32, v: u32) {
        let parent = self.parent(u);
        self.replace_child(parent, u, v);
        self.set_parent(v, parent);
    }
    /// RB-DELETE (CLRS 13.4, third edition). A node with two children is replaced by its
    /// successor, which takes its color.
    fn delete(&mut self, z: u32) {
        let mut removed_red = self.is_red(z);
        let x = if self.left(z) == NIL {
            let x = self.right(z);
            self.transplant(z, x);
            x
        } else if self.right(z) == NIL {
            let x = self.left(z);
            self.transplant(z, x);
            x
        } else {
            let mut y = self.right(z);
            while self.left(y) != NIL {
                y = self.left(y);
            }
            removed_red = self.is_red(y);
            let x = self.right(y);
            if self.parent(y) == z {
                self.set_parent(x, y);
            } else {
                self.transplant(y, x);
                let right = self.right(z);
                self.at(y).right = right;
                self.at(right).parent = y;
            }
            self.transplant(z, y);
            let left = self.left(z);
            self.at(y).left = left;
            self.at(left).parent = y;
            let red = self.is_red(z);
            self.at(y).red = red;
            x
        };
        if !removed_red {
            self.delete_fixup(x);
        }
    }
    /// RB-DELETE-FIXUP (CLRS 13.4).
    fn delete_fixup(&mut self, mut x: u32) {
        while x != self.root && !self.is_red(x) {
            let parent = self.parent(x);
            if x == self.left(parent) {
                let mut w = self.right(parent);
                if self.is_red(w) {
                    self.set_red(w, false);
                    self.set_red(parent, true);
                    self.rotate_left(parent);
                    w = self.right(self.parent(x));
                }
                if !self.is_red(self.left(w)) && !self.is_red(self.right(w)) {
                    self.set_red(w, true);
                    x = self.parent(x);
                } else {
                    if !self.is_red(self.right(w)) {
                        let inner = self.left(w);
                        self.set_red(inner, false);
                        self.set_red(w, true);
                        self.rotate_right(w);
                        w = self.right(self.parent(x));
                    }
                    let parent = self.parent(x);
                    let red = self.is_red(parent);
                    self.set_red(w, red);
                    self.set_red(parent, false);
                    let outer = self.right(w);
                    self.set_red(outer, false);
                    self.rotate_left(parent);
                    x = self.root;
                }
            } else {
                let mut w = self.left(parent);
                if self.is_red(w) {
                    self.set_red(w, false);
                    self.set_red(parent, true);
                    self.rotate_right(parent);
                    w = self.left(self.parent(x));
                }
                if !self.is_red(self.right(w)) && !self.is_red(self.left(w)) {
                    self.set_red(w, true);
                    x = self.parent(x);
                } else {
                    if !self.is_red(self.left(w)) {
                        let inner = self.right(w);
                        self.set_red(inner, false);
                        self.set_red(w, true);
                        self.rotate_left(w);
                        w = self.left(self.parent(x));
                    }
                    let parent = self.parent(x);
                    let red = self.is_red(parent);
                    self.set_red(w, red);
                    self.set_red(parent, false);
                    let outer = self.left(w);
                    self.set_red(outer, false);
                    self.rotate_right(parent);
                    x = self.root;
                }
            }
        }
        self.set_red(x, false);
    }
}

#[cfg(test)]
mod tests {
    // Every expected order below was printed by java.util.HashMap on OpenJDK 25.0.4.1, from a
    // small Java program kept outside the repository. Its keys are the java_hash_map harness's:
    // `record Key(int id, int hash) implements Comparable<Key>`, whose hashCode() returns `hash`
    // and whose compareTo orders by `id`; each value is its key's id unless a test says otherwise.
    // Orders list the ids of `map.entrySet()` in iteration order. Each test names the steps.
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Key {
        id: u32,
        hash: i32,
    }
    impl JavaKey for Key {
        fn java_hash(&self) -> i32 {
            self.hash
        }
        fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
            Some(self.id.cmp(&other.id))
        }
        fn tie_break(&self, other: &Self) -> Ordering {
            self.id.cmp(&other.id)
        }
    }
    fn key(id: u32, hash: i32) -> Key {
        Key { id, hash }
    }
    /// Ids 3 and up share bucket 0 of tables up to 64 buckets: hash (id - 3) * 64.
    fn collider(id: u32) -> Key {
        key(id, (id as i32 - 3) * 64)
    }
    fn order(map: &JavaHashMap<Key, u32>) -> Vec<u32> {
        map.entries().map(|(k, _)| k.id).collect()
    }
    fn next(
        map: &JavaHashMap<Key, u32>,
        cursor: &mut Cursor,
    ) -> Result<Option<(u32, u32)>, CursorError> {
        map.next_entry(cursor)
            .map(|entry| entry.map(|(k, v)| (k.id, *v)))
    }
    #[test]
    fn compute_if_absent_prepends_before_treeification() {
        let mut map = JavaHashMap::default();
        for id in 0..14 {
            assert_eq!(*map.compute_if_absent(key(id, id as i32 * 64), || id), id);
            match id {
                3 => assert_eq!(order(&map), [3, 2, 1, 0]),
                9 => assert_eq!(order(&map), [6, 9, 8, 7, 5, 4, 3, 2, 1, 0]),
                _ => {}
            }
        }
        assert_eq!(order(&map), [6, 9, 10, 11, 12, 13, 8, 7, 5, 4, 3, 2, 1, 0]);
        assert_eq!(*map.compute_if_absent(key(9, 9 * 64), || unreachable!()), 9);
    }

    /// Java: `HashMap.newHashMap(0)`, put ids 5, 3, 1 with hashes 5, 3, 1: the table grows 1, 2, 4 buckets, so
    /// 5 and 1 share bucket 1 and 3 sits in bucket 3.
    #[test]
    fn a_zero_hint_map_grows_like_java() {
        let mut map = JavaHashMap::with_expected_entries(0);
        for id in [5, 3, 1] {
            assert_eq!(map.put(key(id, id as i32), id), None);
        }
        assert_eq!(order(&map), [5, 1, 3]);
        assert_eq!(map.get(&key(3, 3)), Some(&3));
        assert_eq!(map.len(), 3);
    }

    /// Java: `new HashMap<>()`, put ids 0, 1, 2 with hashes 17, 2, 33, then ids 3..=13 (colliders).
    fn colliders() -> JavaHashMap<Key, u32> {
        let mut map = JavaHashMap::default();
        for (id, hash) in [(0, 17), (1, 2), (2, 33)] {
            map.put(key(id, hash), id);
        }
        for id in 3..=13 {
            map.put(collider(id), id);
        }
        map
    }

    /// Java: ids 0..=12 with hashes 1, 17, 33, 49, 0, 16, 2, 18, 34, 3, 35, 4, 20, put into
    /// `new HashMap<>()` in the order 5, 1, 4, 0, 7, 3, 6, 2, 12, 8, 10, 9, 11.
    #[test]
    fn thirteenth_key_doubles_the_table_and_halves_keep_their_order() {
        const HASHES: [i32; 13] = [1, 17, 33, 49, 0, 16, 2, 18, 34, 3, 35, 4, 20];
        let mut map = JavaHashMap::default();
        for id in [5, 1, 4, 0, 7, 3, 6, 2, 12, 8, 10, 9] {
            map.put(key(id, HASHES[id as usize]), id);
        }
        assert_eq!(order(&map), [5, 4, 1, 0, 3, 2, 7, 6, 8, 10, 9, 12]);
        map.put(key(11, 4), 11);
        assert_eq!(order(&map), [4, 0, 2, 6, 8, 10, 9, 11, 5, 1, 3, 7, 12]);
    }

    #[test]
    fn fold_resumes_inside_a_collision_bin() {
        let map = colliders();
        let mut iter = map.entries();
        assert_eq!(iter.next().map(|(k, _)| k.id), Some(6));
        assert_eq!(iter.next().map(|(k, _)| k.id), Some(3));
        assert_eq!(iter.len(), 12);
        let rest = iter.fold(Vec::new(), |mut ids, (key, _)| {
            ids.push(key.id);
            ids
        });
        assert_eq!(rest, [4, 5, 7, 8, 9, 10, 11, 12, 13, 1, 0, 2]);
    }

    /// Java: the steps of `colliders`, printing the order after each put of ids 10..=13.
    #[test]
    fn crowded_bin_grows_a_small_table_and_becomes_a_tree_at_64() {
        let mut map = JavaHashMap::default();
        for (id, hash) in [(0, 17), (1, 2), (2, 33)] {
            map.put(key(id, hash), id);
        }
        for id in 3..=10 {
            map.put(collider(id), id);
        }
        // 16 buckets: the eight colliders of bucket 0, then 0 and 2 (bucket 1), then 1.
        assert_eq!(order(&map), [3, 4, 5, 6, 7, 8, 9, 10, 0, 2, 1]);
        // The ninth collider doubles the table instead of making a tree: 0 moves to bucket 17.
        map.put(collider(11), 11);
        assert_eq!(order(&map), [3, 4, 5, 6, 7, 8, 9, 10, 11, 2, 1, 0]);
        // The tenth doubles it again: 2 moves to bucket 33.
        map.put(collider(12), 12);
        assert_eq!(order(&map), [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 1, 0, 2]);
        // The eleventh makes a tree bin; its root, 6, moves to the front of the list.
        map.put(collider(13), 13);
        assert_eq!(order(&map), [6, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 1, 0, 2]);
    }

    #[test]
    fn compute_if_absent_preserves_java_order_across_treeification_and_split() {
        let mut map = JavaHashMap::default();
        for (id, hash) in [(0, 17), (1, 2), (2, 33)] {
            map.compute_if_absent(key(id, hash), || id);
        }
        for id in 3..=9 {
            map.compute_if_absent(collider(id), || id);
        }
        assert_eq!(order(&map), [9, 8, 7, 6, 5, 4, 3, 2, 0, 1]);
        for id in 10..=14 {
            map.compute_if_absent(collider(id), || id);
        }
        // The crowded bucket grew twice and treeified. The root moved first;
        // the remaining nodes are still in the bucket's linked-list order.
        let before_split = [9, 12, 13, 14, 11, 10, 8, 7, 6, 5, 4, 3, 1, 0, 2];
        assert_eq!(order(&map), before_split);
        for id in 15..=48 {
            map.compute_if_absent(key(id, 4000 + id as i32), || id);
        }
        let fixture_keys = |map: &JavaHashMap<_, _>| {
            order(map)
                .into_iter()
                .filter(|&id| id < 15)
                .collect::<Vec<_>>()
        };
        assert_eq!(fixture_keys(&map), before_split);
        // At 49 entries, even an existing lookup resizes before the lookup.
        assert_eq!(
            map.compute_if_absent(key(0, 17), || panic!("existing key")),
            &0
        );
        assert_eq!(
            fixture_keys(&map),
            [9, 13, 11, 7, 5, 3, 1, 0, 2, 14, 12, 10, 8, 6, 4]
        );
    }

    /// Java: `colliders`, then put id 14 with hash 320, the hash of id 8.
    #[test]
    fn new_key_in_a_tree_bin_follows_its_tree_parent() {
        let mut map = colliders();
        map.put(key(14, 320), 14);
        assert_eq!(
            order(&map),
            [6, 3, 4, 5, 7, 8, 9, 14, 10, 11, 12, 13, 1, 0, 2]
        );
    }

    /// Java: `colliders`; remove id 6; put id 15 with hash 704; remove ids 3, 4, 5, 7, 8; put id
    /// 16 with hash 384, the hash of id 9.
    #[test]
    fn map_removal_moves_the_new_root_first_then_untreeifies_a_small_tree() {
        let mut map = colliders();
        // The removed root's successor, 7, becomes the root and moves to the front.
        assert_eq!(map.remove(&collider(6)), Some(6));
        assert_eq!(order(&map), [7, 3, 4, 5, 8, 9, 10, 11, 12, 13, 1, 0, 2]);
        map.put(key(15, 704), 15);
        assert_eq!(order(&map), [7, 3, 4, 5, 8, 9, 10, 11, 12, 13, 15, 1, 0, 2]);
        let expected: [(u32, &[u32]); 5] = [
            (3, &[10, 7, 4, 5, 8, 9, 11, 12, 13, 15, 1, 0, 2]),
            (4, &[10, 7, 5, 8, 9, 11, 12, 13, 15, 1, 0, 2]),
            (5, &[10, 7, 8, 9, 11, 12, 13, 15, 1, 0, 2]),
            (7, &[10, 8, 9, 11, 12, 13, 15, 1, 0, 2]),
            (8, &[10, 9, 11, 12, 13, 15, 1, 0, 2]),
        ];
        for (id, after) in expected {
            assert_eq!(map.remove(&collider(id)), Some(id));
            assert_eq!(order(&map), after, "after removing {id}");
        }
        // Removing 8 left a plain bin: the new key goes last, not after 9 as in a tree.
        map.put(key(16, 384), 16);
        assert_eq!(order(&map), [10, 9, 11, 12, 13, 15, 16, 1, 0, 2]);
    }

    #[test]
    fn cursor_removes_the_visited_entry_after_advancing_to_another_bin() {
        let mut map = JavaHashMap::default();
        let first = key(1, 0);
        let last_in_bin = key(2, 0);
        let other_bin = key(3, 2);
        for key in [first, last_in_bin, other_bin] {
            map.put(key, key.id);
        }
        let mut cursor = map.cursor();
        assert_eq!(next(&map, &mut cursor), Ok(Some((1, 1))));
        assert_eq!(next(&map, &mut cursor), Ok(Some((2, 2))));
        assert_eq!(map.remove_current(&mut cursor), Ok(()));
        assert_eq!(map.get(&last_in_bin), None);
        assert_eq!(next(&map, &mut cursor), Ok(Some((3, 3))));
        assert_eq!(order(&map), [1, 3]);
    }

    /// Java: `colliders`; its entry set's iterator: next() (id 6) and remove(); put id 15 with
    /// hash 704. Then a new `colliders` whose iterator removes the first eight entries; put id 18
    /// with hash 512, the hash of id 11.
    #[test]
    fn cursor_removal_keeps_the_list_order_and_the_tree() {
        let mut map = colliders();
        let mut cursor = map.cursor();
        assert_eq!(next(&map, &mut cursor), Ok(Some((6, 6))));
        assert_eq!(map.remove_current(&mut cursor), Ok(()));
        assert_eq!(order(&map), [3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 1, 0, 2]);
        assert_eq!(map.get(&collider(6)), None);
        assert_eq!(map.get(&collider(7)), Some(&7));
        assert_eq!(map.get(&collider(13)), Some(&13));
        // The next tree insertion moves the root left by the removal, 7, to the front.
        map.put(key(15, 704), 15);
        assert_eq!(order(&map), [7, 3, 4, 5, 8, 9, 10, 11, 12, 13, 15, 1, 0, 2]);

        let mut map = colliders();
        let mut cursor = map.cursor();
        for _ in 0..8 {
            assert!(matches!(next(&map, &mut cursor), Ok(Some(_))));
            assert_eq!(map.remove_current(&mut cursor), Ok(()));
        }
        assert_eq!(order(&map), [11, 12, 13, 1, 0, 2]);
        // Three nodes, still a tree bin: 18 goes after its tree parent 11, then root 12 moves first.
        map.put(key(18, 512), 18);
        assert_eq!(order(&map), [12, 11, 18, 13, 1, 0, 2]);
    }

    #[test]
    fn physical_reservation_does_not_change_small_table_growth_or_tree_order() {
        let mut regular = JavaHashMap::default();
        let mut reserved = JavaHashMap::with_reserved_nodes(64);
        for (key, id) in [(key(0, 17), 0), (key(1, 2), 1), (key(2, 33), 2)]
            .into_iter()
            .chain((3..=13).map(|id| (collider(id), id)))
        {
            assert_eq!(regular.put(key, id), reserved.put(key, id));
            assert_eq!(order(&regular), order(&reserved));
        }
    }

    #[test]
    fn cursor_leaves_a_singleton_tree_that_survives_table_growth() {
        let mut map = colliders();
        let mut cursor = map.cursor();
        while let Some((id, _)) = next(&map, &mut cursor).unwrap() {
            if id != 6 {
                map.remove_current(&mut cursor).unwrap();
            }
        }
        // 49 live entries double this 64-bucket table, leaving the tree in bucket 64.
        for hash in 1..=48 {
            let id = 100 + hash as u32;
            map.put(key(id, hash), id);
        }
        assert_eq!(map.get(&collider(6)), Some(&6));
        map.put(key(20, 192), 20);
        map.put(key(21, 192), 21);
        // Observed on the JVM: the third put moves the root first. A plain bin would append.
        assert_eq!(order(&map)[48..], [20, 6, 21]);
    }

    /// Java: the 13 puts of the doubling test, remove id 9, `new HashMap<>(map)`, then put id 9
    /// (hash 3) into the copy. And a map holding id 0 (hash 1), copied, then given ids 1, 2, 3
    /// with hashes 3, 5, 7.
    #[test]
    fn copy_is_sized_for_its_entries_and_refilled_in_source_order() {
        const HASHES: [i32; 13] = [1, 17, 33, 49, 0, 16, 2, 18, 34, 3, 35, 4, 20];
        let mut map = JavaHashMap::default();
        for id in [5, 1, 4, 0, 7, 3, 6, 2, 12, 8, 10, 9, 11] {
            map.put(key(id, HASHES[id as usize]), id);
        }
        // 13 entries need 32 buckets, as the source has: the copy keeps its order.
        assert_eq!(order(&map.copied()), order(&map));
        map.remove(&key(9, 3));
        assert_eq!(order(&map), [4, 0, 2, 6, 8, 10, 11, 5, 1, 3, 7, 12]);
        // 12 entries fit 16 buckets, where the source has 32.
        for mut copy in [map.copied(), map.into_copy()] {
            assert_eq!(order(&copy), [4, 5, 0, 2, 1, 3, 6, 8, 7, 10, 11, 12]);
            copy.put(key(9, 3), 9);
            assert_eq!(order(&copy), [4, 0, 2, 6, 8, 10, 9, 11, 5, 1, 3, 7, 12]);
        }

        let mut one = JavaHashMap::default();
        one.put(key(0, 1), 0);
        // Two buckets (threshold 1), then four (threshold 3), then eight.
        let mut copy = one.copied();
        let expected: [&[u32]; 3] = [&[0, 1], &[0, 2, 1], &[0, 1, 2, 3]];
        for (id, after) in (1..=3).zip(expected) {
            copy.put(key(id, id as i32 * 2 + 1), id);
            assert_eq!(order(&copy), after, "after putting {id}");
        }
        assert_eq!(JavaHashMap::<Key, u32>::default().copied().len(), 0);
    }

    /// Java: put ids 0 and 1 (hashes 1, 2); iterator A: remove(); put id 2 (hash 3); A: remove(),
    /// next(), hasNext(). Iterator B: next(); put id 1 with value 10; remove a missing key; next(),
    /// remove(), remove(), next(), hasNext(), next(), remove(). Iterator C: next(); put id 3
    /// (hash 4); remove(). An empty map's iterator: hasNext(), clear(), next(), remove().
    #[test]
    fn cursor_errors_come_in_java_order() {
        use CursorError::{ConcurrentModification, NoCurrentEntry, WrongMap};
        let mut map = JavaHashMap::default();
        map.put(key(0, 1), 0);
        map.put(key(1, 2), 1);
        let mut a = map.cursor();
        assert_eq!(map.remove_current(&mut a), Err(NoCurrentEntry));
        map.put(key(2, 3), 2);
        assert_eq!(map.remove_current(&mut a), Err(NoCurrentEntry));
        assert_eq!(next(&map, &mut a), Err(ConcurrentModification));
        assert!(a.has_next());

        let mut b = map.cursor();
        assert_eq!(next(&map, &mut b), Ok(Some((0, 0))));
        assert_eq!(map.put(key(1, 2), 10), Some(1));
        assert_eq!(map.remove(&key(9, 9)), None);
        assert_eq!(next(&map, &mut b), Ok(Some((1, 10))));
        assert_eq!(map.remove_current(&mut b), Ok(()));
        assert_eq!(map.remove_current(&mut b), Err(NoCurrentEntry));
        assert_eq!(next(&map, &mut b), Ok(Some((2, 2))));
        assert!(!b.has_next());
        assert_eq!(next(&map, &mut b), Ok(None));
        assert_eq!(map.remove_current(&mut b), Ok(()));
        assert_eq!(order(&map), [0]);

        let mut c = map.cursor();
        assert_eq!(next(&map, &mut c), Ok(Some((0, 0))));
        map.put(key(3, 4), 3);
        assert_eq!(map.remove_current(&mut c), Err(ConcurrentModification));

        let mut empty = JavaHashMap::default();
        let mut d = empty.cursor();
        assert!(!d.has_next());
        empty.clear();
        assert_eq!(next(&empty, &mut d), Err(ConcurrentModification));
        assert_eq!(empty.remove_current(&mut d), Err(NoCurrentEntry));
        // Not a Java case: a cursor handed to another map.
        assert_eq!(next(&map, &mut d), Err(WrongMap));
        assert_eq!(map.remove_current(&mut d), Err(WrongMap));
    }
}

#[cfg(test)]
mod slab_tests {
    use super::*;
    use std::rc::Rc;

    #[derive(Clone, PartialEq, Eq)]
    struct Owned(Rc<(u32, i32)>);
    impl JavaKey for Owned {
        fn java_hash(&self) -> i32 {
            self.0.1
        }
        fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
            Some(self.0.0.cmp(&other.0.0))
        }
        fn tie_break(&self, other: &Self) -> Ordering {
            self.0.0.cmp(&other.0.0)
        }
    }
    fn owned(id: u32, hash: i32) -> Owned {
        Owned(Rc::new((id, hash)))
    }
    fn ids(map: &JavaHashMap<Owned, Rc<u32>>) -> Vec<u32> {
        map.entries().map(|(k, _)| k.0.0).collect()
    }

    /// Keys and values that own heap data are dropped exactly once on remove, clear, replace and drop.
    #[test]
    fn owned_entries_drop_once_through_every_path() {
        let keys: Vec<Owned> = (0..200).map(|id| owned(id, (id as i32 % 7) * 64)).collect();
        let values: Vec<Rc<u32>> = (0..200).map(Rc::new).collect();
        {
            let mut map = JavaHashMap::default();
            for (k, v) in keys.iter().zip(&values) {
                assert!(map.put(k.clone(), v.clone()).is_none());
            }
            assert_eq!(
                map.put(keys[3].clone(), values[4].clone()),
                Some(values[3].clone())
            );
            for k in keys.iter().step_by(3) {
                assert!(map.remove(k).is_some());
            }
            let copy = map.copied();
            let clone = map.clone_with(Rc::clone);
            assert_eq!(ids(&copy).len(), map.len());
            assert_eq!(ids(&clone), ids(&map));
            assert_eq!(
                clone
                    .values_in_storage_order()
                    .map(|v| **v)
                    .collect::<Vec<_>>(),
                map.values_in_storage_order()
                    .map(|v| **v)
                    .collect::<Vec<_>>()
            );
            map.clear();
            assert!(map.is_empty() && map.entries().next().is_none());
            for (k, v) in keys.iter().zip(&values).take(20) {
                map.put(k.clone(), v.clone());
            }
            let moved: Vec<u32> = clone.into_iter().map(|(k, _)| k.0.0).collect();
            assert_eq!(moved.len(), 200 - 67);
            let mut partial = copy.into_iter();
            partial.next();
            drop(partial);
        }
        assert!(keys.iter().all(|k| Rc::strong_count(&k.0) == 1));
        assert!(values.iter().all(|v| Rc::strong_count(v) == 1));
    }

    /// Entry, for_each_mut and retain agree with put/get/remove on a map with tree bins.
    #[test]
    fn entry_retain_and_mutation_follow_traversal() {
        let mut map = JavaHashMap::<Owned, Rc<u32>>::default();
        for id in 0..80u32 {
            *map.entry(owned(id, (id as i32 % 3) * 128))
                .or_insert(Rc::new(0)) = Rc::new(id);
        }
        *map.entry(owned(5, 2 * 128)).or_insert_with(|| Rc::new(0)) = Rc::new(500);
        assert_eq!(map.get(&owned(5, 2 * 128)).map(|v| **v), Some(500));
        let mut seen = Vec::new();
        map.for_each_mut(|k, v| {
            seen.push(k.0.0);
            *v = Rc::new(k.0.0 * 2);
        });
        assert_eq!(seen, ids(&map));
        map.retain(|k, _| k.0.0 % 2 == 0);
        assert_eq!(map.len(), 40);
        assert!(ids(&map).iter().all(|id| id % 2 == 0));
        for id in (0..80u32).step_by(2) {
            assert_eq!(
                map.get_mut(&owned(id, (id as i32 % 3) * 128)).map(|v| **v),
                Some(id * 2)
            );
        }
    }

    #[test]
    fn exhausted_generation_never_revalidates_a_cursor() {
        let mut map = JavaHashMap::default();
        map.put(owned(1, 1), Rc::new(1));
        map.mod_count = u64::MAX - 1;
        let mut pending = map.cursor();
        let mut current = map.cursor();
        assert!(map.next_entry(&mut current).unwrap().is_some());
        map.clear();
        map.clear();
        map.put(owned(2, 2), Rc::new(2));

        assert_eq!(
            map.next_entry(&mut pending).err(),
            Some(CursorError::ConcurrentModification)
        );
        assert_eq!(
            map.remove_current(&mut current),
            Err(CursorError::ConcurrentModification)
        );
        let mut exhausted = map.cursor();
        assert_eq!(
            map.next_entry(&mut exhausted).err(),
            Some(CursorError::ConcurrentModification)
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn map_id_blocks_run_out_instead_of_repeating() {
        let blocks = AtomicU64::new(ID_BLOCKS - 2);
        let first = reserve_id_block(&blocks).expect("second-to-last block");
        let last = reserve_id_block(&blocks).expect("last block");
        assert_eq!(last - first, 1 << ID_BLOCK_BITS);
        assert!(
            last.checked_add(1 << ID_BLOCK_BITS).is_some(),
            "the last block's end fits in u64"
        );
        for _ in 0..3 {
            assert_eq!(reserve_id_block(&blocks), None);
        }
        assert_eq!(
            blocks.load(atomic::Ordering::Relaxed),
            ID_BLOCKS,
            "refusals leave the counter where it was"
        );
        let fresh = AtomicU64::new(0);
        assert_eq!(reserve_id_block(&fresh), Some(0));
        assert_eq!(reserve_id_block(&fresh), Some(1 << ID_BLOCK_BITS));
    }

    #[test]
    fn reserved_and_empty_maps_answer_without_a_table() {
        let map = JavaHashMap::<Owned, Rc<u32>>::with_reserved_nodes(64);
        assert_eq!(map.get(&owned(1, 0)), None);
        assert!(!map.cursor().has_next());
        assert_eq!(map.entries().len(), 0);
        let huge = JavaHashMap::<Owned, Rc<u32>>::with_reserved_nodes(usize::MAX);
        assert_eq!(huge.storage_bytes(), 0);
    }
}

#[cfg(test)]
mod unwind_tests {
    use super::*;
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    thread_local! {
        static COMPARES_LEFT: Cell<i64> = const { Cell::new(-1) };
    }
    fn arm(compares: i64) {
        COMPARES_LEFT.with(|c| c.set(compares));
    }
    fn disarm() {
        COMPARES_LEFT.with(|c| c.set(-1));
    }

    /// Equal-hash keys force `comparable_compare`, which panics once its budget runs out.
    #[derive(Clone, PartialEq, Eq)]
    struct Touchy(Rc<(u32, i32)>);
    impl JavaKey for Touchy {
        fn java_hash(&self) -> i32 {
            self.0.1
        }
        fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
            COMPARES_LEFT.with(|c| {
                let left = c.get();
                if left == 0 {
                    panic!("comparator panic for the unwind test");
                }
                if left > 0 {
                    c.set(left - 1);
                }
            });
            Some(self.0.0.cmp(&other.0.0))
        }
        fn tie_break(&self, other: &Self) -> Ordering {
            self.0.0.cmp(&other.0.0)
        }
    }
    fn touchy(id: u32, hash: i32) -> Touchy {
        Touchy(Rc::new((id, hash)))
    }

    /// Everything safe code can observe must stay consistent: lookups, traversal, cursors, growth, drops.
    fn check_consistent(map: &mut JavaHashMap<Touchy, Rc<u32>>, keys: &[Touchy]) {
        disarm();
        assert!(
            map.links.iter().all(|l| !l.is_pending()),
            "no rehash mark outlives grow"
        );
        let live: Vec<&Touchy> = keys.iter().filter(|k| map.get(k).is_some()).collect();
        assert_eq!(
            live.len(),
            map.len(),
            "every counted entry is reachable by lookup"
        );
        assert_eq!(map.entries().count(), map.len());
        let mut cursor = map.cursor();
        let mut seen = 0;
        while cursor.has_next() {
            map.next_entry(&mut cursor).unwrap().unwrap();
            seen += 1;
        }
        assert_eq!(seen, map.len());
        for id in 0..40u32 {
            map.put(touchy(10_000 + id, 0), Rc::new(id));
        }
        assert_eq!(map.entries().count(), map.len());
        let mut cursor = map.cursor();
        while cursor.has_next() {
            map.next_entry(&mut cursor).unwrap().unwrap();
            map.remove_current(&mut cursor).unwrap();
        }
        assert!(map.is_empty() && map.entries().next().is_none());
    }

    fn sweep(
        build: impl Fn() -> (JavaHashMap<Touchy, Rc<u32>>, Vec<Touchy>),
        trigger: impl Fn(&mut JavaHashMap<Touchy, Rc<u32>>, &mut Vec<Touchy>),
    ) {
        let mut panicked_once = false;
        for budget in 0.. {
            let (mut map, mut keys) = build();
            let value = Rc::new(7);
            arm(budget);
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                trigger(&mut map, &mut keys);
                map.put(touchy(99_999, 0), value.clone());
            }));
            disarm();
            if outcome.is_ok() {
                assert!(panicked_once, "the trigger never reached a comparison");
                break;
            }
            panicked_once = true;
            keys.push(touchy(99_999, 0));
            check_consistent(&mut map, &keys);
            drop(map);
            assert!(
                keys.iter().all(|k| Rc::strong_count(&k.0) == 1),
                "no key leaked or dropped twice"
            );
            assert_eq!(
                Rc::strong_count(&value),
                1,
                "no value leaked or dropped twice"
            );
        }
    }

    /// 16 equal-hash keys in bucket 0 of a 64-bucket table split 8/8 at 128 buckets and both halves re-treeify.
    fn split_tree() -> (JavaHashMap<Touchy, Rc<u32>>, Vec<Touchy>) {
        let built = tree_map(32);
        assert_eq!(built.0.len(), 48);
        built
    }
    /// The 16 equal-hash keys of `split_tree` as one tree bin of a 64-bucket table, plus `fillers` singleton keys.
    fn tree_map(fillers: u32) -> (JavaHashMap<Touchy, Rc<u32>>, Vec<Touchy>) {
        disarm();
        let mut map = JavaHashMap::default();
        let mut keys = Vec::new();
        for id in 0..16u32 {
            let key = touchy(id, if id % 2 == 0 { 0 } else { 64 });
            map.put(key.clone(), Rc::new(id));
            keys.push(key);
        }
        for id in 0..fillers {
            let key = touchy(100 + id, 1 + id as i32);
            map.put(key.clone(), Rc::new(id));
            keys.push(key);
        }
        (map, keys)
    }

    #[test]
    fn comparator_panic_during_rehash_treeify_leaves_a_sound_map() {
        sweep(split_tree, |map, keys| {
            let key = touchy(200, 33);
            keys.push(key.clone());
            map.put(key, Rc::new(200));
        });
    }

    #[test]
    fn comparator_panic_during_put_treeify_leaves_a_sound_map() {
        let build = || {
            disarm();
            let mut map = JavaHashMap::with_reserved_nodes(4);
            let mut keys = Vec::new();
            for id in 0..48u32 {
                let key = touchy(id, if id < 8 { 0 } else { id as i32 });
                map.put(key.clone(), Rc::new(id));
                keys.push(key);
            }
            (map, keys)
        };
        sweep(build, |map, keys| {
            let key = touchy(500, 0);
            keys.push(key.clone());
            map.put(key, Rc::new(500));
        });
    }

    /// Comparisons a call makes, counted with an unreachable budget.
    fn compares_used(call: impl FnOnce()) -> i64 {
        const BUDGET: i64 = 1 << 40;
        arm(BUDGET);
        call();
        let left = COMPARES_LEFT.with(|c| c.get());
        disarm();
        BUDGET - left
    }

    /// Every comparison of a tree insert may panic, including those after the bin search, where the old code had
    /// already moved the key and value into a fresh slot: the map must be left exactly as it was.
    #[test]
    fn comparator_panic_during_tree_insert_leaves_the_map_unchanged() {
        // 36 entries stay below the 64-bucket threshold of 48, so the insert never rehashes.
        let search = {
            let (map, _) = tree_map(20);
            compares_used(|| assert!(map.get(&touchy(300, 0)).is_none()))
        };
        let insert = {
            let (mut map, _) = tree_map(20);
            compares_used(|| assert!(map.put(touchy(300, 0), Rc::new(9)).is_none()))
        };
        assert!(
            insert > search + 1,
            "the insert compares after its search ({search} vs {insert})"
        );
        for budget in 0..insert {
            let (mut map, keys) = tree_map(20);
            let before: Vec<u32> = map.entries().map(|(k, _)| k.0.0).collect();
            let value = Rc::new(9);
            let key = touchy(300, 0);
            arm(budget);
            let outcome = catch_unwind(AssertUnwindSafe(|| map.put(key.clone(), value.clone())));
            disarm();
            assert!(outcome.is_err(), "budget {budget} of {insert} panics");
            assert_eq!(
                map.entries().map(|(k, _)| k.0.0).collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                map.values_in_storage_order().count(),
                map.len(),
                "budget {budget}: no stray live slot"
            );
            assert_eq!(
                (Rc::strong_count(&key.0), Rc::strong_count(&value)),
                (1, 1),
                "budget {budget}: the map holds no copy of the failed key or value"
            );
            check_consistent(&mut map, &keys);
        }
    }
}
