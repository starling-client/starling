//! What a thread is working on, for the sampler: context stores on every platform (no-ops where it does not run) and the names behind its ids.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering::*};

#[cfg(target_os = "linux")]
mod cx {
    pub use crate::sampler::{
        replace_span, set_block, set_chunk, set_dim, set_entity_type, set_span, set_tick, tick_end,
    };
}

#[cfg(not(target_os = "linux"))]
mod cx {
    pub fn set_block(_: u16) {}
    pub fn set_chunk(_: i32, _: i32) {}
    pub fn set_dim(_: u8) {}
    pub fn set_entity_type(_: u16) {}
    pub fn set_span(_: u16) {}
    pub fn set_tick(_: u64) {}
    pub fn replace_span(_: u16) -> u16 {
        0
    }
    pub fn tick_end(_: u64) {}
}

const ON: bool = cfg!(target_os = "linux");

/// What an id names; 0 is always "none".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Span = 0,
    Entity = 1,
}

const MAX_IDS: usize = 4096;
const SLOTS: usize = 1024;

struct Names {
    ptr: [AtomicPtr<u8>; MAX_IDS],
    len: [AtomicU32; MAX_IDS],
    interned: Mutex<BTreeMap<&'static str, u16>>,
}

static NAMES: [Names; 2] = [const { Names::new() }; 2];

impl Names {
    const fn new() -> Self {
        Self {
            ptr: [const { AtomicPtr::new(std::ptr::null_mut()) }; MAX_IDS],
            len: [const { AtomicU32::new(0) }; MAX_IDS],
            interned: Mutex::new(BTreeMap::new()),
        }
    }

    fn name(&self, id: u16) -> Option<&'static str> {
        let ptr = self.ptr.get(id as usize)?.load(Acquire);
        // SAFETY: a published slot holds a leaked, immutable UTF-8 string of that length.
        (!ptr.is_null()).then(|| unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                ptr,
                self.len[id as usize].load(Relaxed) as usize,
            ))
        })
    }

    fn intern(&self, name: &str) -> u16 {
        let mut interned = self.interned.lock().unwrap();
        if let Some(&id) = interned.get(name) {
            return id;
        }
        let id = interned.len() + 1;
        if id >= MAX_IDS {
            return 0;
        }
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        self.len[id].store(leaked.len() as u32, Relaxed);
        self.ptr[id].store(leaked.as_ptr().cast_mut(), Release);
        interned.insert(leaked, id as u16);
        id as u16
    }
}

thread_local! {
    static CACHE: [[Cell<(usize, usize, u16)>; SLOTS]; 2] = const { [const { [const { Cell::new((0, 0, 0)) }; SLOTS] }; 2] };
    static TICK: Cell<u64> = const { Cell::new(0) };
}

/// Fibonacci hashing's multiplier at the pointer's width (Starling's armv7 and x86 APKs are
/// 32-bit).
#[cfg(target_pointer_width = "64")]
const GOLDEN: usize = 0x9E37_79B9_7F4A_7C15;
#[cfg(not(target_pointer_width = "64"))]
const GOLDEN: usize = 0x9E37_79B9;

/// The id of `name`, found by the address and length of its bytes; `verify` re-checks the text, for strings that can be freed and reused.
///
/// The cache is two-way: each address may sit in its slot or the slot's pair, so two hot names that hash
/// to the same slot don't evict each other on every call. A miss moves the slot's entry to its pair.
#[inline]
fn id_of(space: Space, name: &str, verify: bool) -> u16 {
    if !ON {
        return 0;
    }
    let (names, key) = (&NAMES[space as usize], name.as_ptr() as usize);
    let slot = (key >> 3).wrapping_mul(GOLDEN) >> (usize::BITS - 10);
    let found = |hit: (usize, usize, u16)| {
        (hit.0 == key
            && hit.1 == name.len()
            && hit.2 != 0
            && (!verify || names.name(hit.2) == Some(name)))
        .then_some(hit.2)
    };
    let (first, second) = CACHE.with(|c| {
        (
            c[space as usize][slot].get(),
            c[space as usize][slot ^ 1].get(),
        )
    });
    if let Some(id) = found(first).or_else(|| found(second)) {
        return id;
    }
    let id = names.intern(name);
    CACHE.with(|c| {
        c[space as usize][slot ^ 1].set(first);
        c[space as usize][slot].set((key, name.len(), id));
    });
    id
}

/// What `id` names, for the report that reads the samples.
pub fn name(space: Space, id: u16) -> Option<&'static str> {
    NAMES[space as usize].name(id)
}

/// Makes `name` the innermost span, returning the one it replaces for [`leave_span`].
#[inline]
pub fn enter_span(name: &'static str) -> u16 {
    cx::replace_span(id_of(Space::Span, name, false))
}

#[inline]
pub fn leave_span(previous: u16) {
    cx::set_span(previous);
}

/// The id [`set_entity`] stores for an entity or block entity type.
#[inline]
pub fn entity_id(name: &str) -> u16 {
    id_of(Space::Entity, name, true)
}

#[inline]
pub fn set_entity(id: u16) {
    cx::set_entity_type(id);
}

#[inline]
pub fn set_chunk(x: i32, z: i32) {
    cx::set_chunk(x, z);
}

#[inline]
pub fn set_block(id: u16) {
    cx::set_block(id);
}

#[inline]
pub fn set_dim(id: u8) {
    cx::set_dim(id);
}

/// The work done with the thread's entity type, chunk and block is over.
#[inline]
pub fn clear_work() {
    cx::set_entity_type(0);
    cx::set_block(0);
    cx::set_chunk(0, 0);
}

/// Starts the next tick or frame of this thread.
#[inline]
pub fn tick_start() {
    let tick = TICK.get() + 1;
    TICK.set(tick);
    cx::set_tick(tick);
}

/// Ends the tick or frame [`tick_start`] began, `duration_ns` long.
#[inline]
pub fn tick_end(duration_ns: u64) {
    cx::tick_end(duration_ns);
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
