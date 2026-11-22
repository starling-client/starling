//! A list of `&'static` values registered before the game runs and fixed when it starts: what
//! [`crate::register`], [`crate::settings::register`] and the other registries keep. Each one is
//! a `static`.
use std::sync::{Mutex, OnceLock};

use crate::TooLate;

pub(crate) struct Registry<T: ?Sized + 'static> {
    /// Registered before the list is sealed.
    pending: Mutex<Vec<&'static T>>,
    /// The list the engine reads, fixed the first time it reads it.
    sealed: OnceLock<Box<[&'static T]>>,
}

impl<T: ?Sized + 'static> Registry<T> {
    pub(crate) const fn new() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            sealed: OnceLock::new(),
        }
    }

    pub(crate) fn register(&self, value: &'static T) -> Result<(), TooLate> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the lock: `all` seals under it too, so a value is never lost in between.
        if self.sealed.get().is_some() {
            return Err(TooLate);
        }
        pending.push(value);
        Ok(())
    }

    /// The registered values in registration order, sealing the list on the first call. Once
    /// sealed, one atomic load.
    pub(crate) fn all(&'static self) -> &'static [&'static T] {
        self.sealed.get_or_init(|| {
            std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()))
                .into_boxed_slice()
        })
    }
}

/// Why a registry with a bound refused a value.
pub(crate) enum Beyond {
    /// The list had sealed: the value registered after the game runs.
    TooLate,
    /// The bound holds every place the registry's entries take.
    Full,
}

impl<T: ?Sized + 'static> Registry<T> {
    /// Registers `value` while fewer than `max` wait, then refuses with [`Beyond`]. The count
    /// and the seal are checked under the lock, so a registry that hands its entries ids or
    /// places can never hand out one past its bound.
    pub(crate) fn register_max(&self, value: &'static T, max: usize) -> Result<(), Beyond> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the lock: `all` seals under it too, so a value is never lost in between.
        if self.sealed.get().is_some() {
            return Err(Beyond::TooLate);
        }
        if pending.len() >= max {
            return Err(Beyond::Full);
        }
        pending.push(value);
        Ok(())
    }
}
