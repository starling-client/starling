//! Run retained native code through the same panic boundary as generated hooks.
use super::{contain_callback, installed, log_and_queue_panic};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type PoisonedKeys = HashMap<(usize, &'static str), HashSet<Arc<str>>>;

#[derive(Default)]
pub(super) struct RetainedPoisons {
    any: AtomicBool,
    entries: Mutex<PoisonedKeys>,
}

// no wire: the game binds a native callback's owner and session-local disablement key.
// exhaustive: private fields keep this an engine-only dispatch token.
/// Identity of retained native code, stable when its recipes or command tree are rebuilt.
#[derive(Clone, Debug)]
pub struct RetainedCallback {
    owner: &'static str,
    index: usize,
    site: &'static str,
    key: Arc<str>,
}

// no wire: an internal outcome of invoking process-local native code.
// exhaustive: engine dispatch matches every failure to its call site's existing error behavior.
/// Why a retained callback did not produce an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedFailure {
    /// The callback or its owner was already disabled.
    Disabled,
    /// The callback panicked.
    Panicked,
}

impl RetainedCallback {
    /// Bind the known owner slot and the callback's registered key at load time.
    pub fn new(owner: &'static str, index: usize, site: &'static str, key: &str) -> Self {
        Self {
            owner,
            index,
            site,
            key: Arc::from(key),
        }
    }

    /// Check the owner and this callback before starting work on its behalf.
    pub fn enabled(&self) -> bool {
        installed().is_none_or(|installed| {
            installed
                .disabled
                .get(self.index)
                .is_none_or(|disabled| !disabled.load(Ordering::Relaxed))
                && !(installed.retained_poisons.any.load(Ordering::Acquire)
                    && installed
                        .retained_poisons
                        .entries
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&(self.index, self.site))
                        .is_some_and(|keys| keys.contains(self.key.as_ref())))
        })
    }

    /// Invoke only this callback. A panic is reported once and suppresses the callback
    /// even if it is later rebound to the same key.
    pub fn call<T>(&self, call: impl FnOnce() -> T) -> Result<T, RetainedFailure> {
        if !self.enabled() {
            return Err(RetainedFailure::Disabled);
        }
        let _owner = super::mark_mod(self.owner, self.site);
        // The retained call's own caller handles this outcome. Do not flag an enclosing
        // generated listener as failed when it called retained code and handled the result.
        contain_callback(call, |payload| {
            let first = installed().is_none_or(|installed| {
                let poisons = &installed.retained_poisons;
                let mut entries = poisons.entries.lock().unwrap_or_else(|e| e.into_inner());
                let first = entries
                    .entry((self.index, self.site))
                    .or_default()
                    .insert(self.key.clone());
                poisons.any.store(true, Ordering::Release);
                first
            });
            if first {
                log_and_queue_panic(self.site, self.owner, payload);
            }
        })
        .ok_or(RetainedFailure::Panicked)
    }
}
