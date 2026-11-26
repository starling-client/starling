// Element-checked access to the list at a native callback's write boundary.

/// A list lent by a hook. Reads project one entry; each write checks only the submitted entry.
/// Later listeners see accepted edits, in order. If a listener panics, its edits are undone.
/// A fixed-size list (such as a brewing inventory) cannot insert or remove entries.
// no wire: this type borrows the engine's list for the duration of a callback.
// exhaustive: the fields are private; only the engine constructs a list.
pub struct HookList<'a, T, E, const FIXED: bool = false> {
    backend: &'a mut dyn HookListBackend<T, E>,
    hook: &'static str,
    field: &'static str,
    listener: &'static str,
}

impl<'a, T, E: core::fmt::Debug, const FIXED: bool> HookList<'a, T, E, FIXED> {
    // hidden: the engine lends the list; mods receive it in a hook's event.
    #[doc(hidden)]
    pub fn new(
        backend: &'a mut dyn HookListBackend<T, E>,
        hook: &'static str,
        field: &'static str,
    ) -> Self {
        Self {
            backend,
            hook,
            field,
            listener: "",
        }
    }

    pub(crate) fn enter(&mut self, listener: &'static str) {
        self.listener = listener;
    }

    pub(crate) fn finish(&mut self, accepted: bool) {
        self.backend.finish(accepted);
    }

    /// Number of entries, including empty slots when `T` is optional.
    pub fn len(&self) -> usize {
        self.backend.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read one entry. An absent index or an entry that cannot be projected yields `None`.
    pub fn get(&self, index: usize) -> Option<T> {
        self.backend.get(index)
    }

    /// Visit projectable entries in index order. Use [`Self::get`] when indices matter.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        (0..self.len()).filter_map(|index| self.get(index))
    }

    /// Replace an entry, or return `Ok(false)` if the index is absent. An invalid value leaves
    /// that entry unchanged and returns the boundary error.
    pub fn set(&mut self, index: usize, value: T) -> Result<bool, E> {
        let result = self.backend.set(index, value);
        self.report(result)
    }

    /// Exchange two entries in place without projecting them. Returns `false` for an absent index.
    pub fn swap(&mut self, first: usize, second: usize) -> bool {
        self.backend.swap(first, second)
    }

    fn report(&self, result: Result<bool, E>) -> Result<bool, E> {
        if let Err(error) = &result {
            log::warn!(
                "mod {} hook {} output {} refused: {error:?}",
                self.listener,
                self.hook,
                self.field
            );
        }
        result
    }
}

impl<T, E: core::fmt::Debug> HookList<'_, T, E> {
    /// Insert before `index`, or at the end when `index == len()`. A larger index yields
    /// `Ok(false)`; an invalid value returns its boundary error without changing the list.
    pub fn insert(&mut self, index: usize, value: T) -> Result<bool, E> {
        let result = self.backend.insert(index, value);
        self.report(result)
    }

    /// Append one checked entry. A refusal does not change other entries.
    pub fn push(&mut self, value: T) -> Result<(), E> {
        self.insert(self.len(), value).map(|_| ())
    }

    /// Remove an entry without projecting it. Returns `false` if the index is absent.
    pub fn remove(&mut self, index: usize) -> bool {
        self.backend.remove(index)
    }

    /// Remove every entry, including any that cannot be projected.
    pub fn clear(&mut self) {
        for index in (0..self.len()).rev() {
            self.remove(index);
        }
    }
}

// hidden: the engine's side of a lent list.
#[doc(hidden)]
#[allow(clippy::len_without_is_empty)]
pub trait HookListBackend<T, E> {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<T>;
    fn set(&mut self, index: usize, value: T) -> Result<bool, E>;
    fn insert(&mut self, index: usize, value: T) -> Result<bool, E>;
    fn remove(&mut self, index: usize) -> bool;
    fn swap(&mut self, first: usize, second: usize) -> bool;
    fn finish(&mut self, accepted: bool);
}

// hidden: the engine's storage behind a lent list.
// exhaustive: a list is resizable or fixed, as `HookList`'s `FIXED` says.
#[doc(hidden)]
pub enum HookListStorage<'a, T> {
    Resizable(&'a mut Vec<T>),
    Fixed(&'a mut [T]),
}

// hidden: the engine's undo journal behind a lent list.
// exhaustive: the fields are private.
#[doc(hidden)]
pub struct HookListJournal<'a, T, R, W> {
    storage: HookListStorage<'a, T>,
    undo: Vec<Undo<T>>,
    read: R,
    write: W,
}

enum Undo<T> {
    Set(usize, T),
    Insert(usize),
    Remove(usize, T),
    Swap(usize, usize),
}

impl<'a, T, R, W> HookListJournal<'a, T, R, W> {
    pub fn new<A, E>(storage: HookListStorage<'a, T>, read: R, write: W) -> Self
    where
        R: Fn(&T) -> Option<A>,
        W: Fn(&A) -> Result<T, E>,
    {
        Self {
            storage,
            undo: Vec::new(),
            read,
            write,
        }
    }
}

impl<T> HookListStorage<'_, T> {
    fn as_slice(&self) -> &[T] {
        match self {
            Self::Resizable(list) => list,
            Self::Fixed(list) => list,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            Self::Resizable(list) => list,
            Self::Fixed(list) => list,
        }
    }
}

impl<T, A, E, R: Fn(&T) -> Option<A>, W: Fn(&A) -> Result<T, E>> HookListBackend<A, E>
    for HookListJournal<'_, T, R, W>
{
    fn len(&self) -> usize {
        self.storage.as_slice().len()
    }

    fn get(&self, index: usize) -> Option<A> {
        self.storage.as_slice().get(index).and_then(&self.read)
    }

    fn set(&mut self, index: usize, value: A) -> Result<bool, E> {
        let Some(slot) = self.storage.as_mut_slice().get_mut(index) else {
            return Ok(false);
        };
        let value = (self.write)(&value)?;
        self.undo
            .push(Undo::Set(index, core::mem::replace(slot, value)));
        Ok(true)
    }

    fn insert(&mut self, index: usize, value: A) -> Result<bool, E> {
        if let HookListStorage::Resizable(list) = &mut self.storage
            && index <= list.len()
        {
            list.insert(index, (self.write)(&value)?);
            self.undo.push(Undo::Insert(index));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn remove(&mut self, index: usize) -> bool {
        if let HookListStorage::Resizable(list) = &mut self.storage
            && index < list.len()
        {
            self.undo.push(Undo::Remove(index, list.remove(index)));
            true
        } else {
            false
        }
    }

    fn swap(&mut self, first: usize, second: usize) -> bool {
        let list = self.storage.as_mut_slice();
        if first >= list.len() || second >= list.len() {
            return false;
        }
        list.swap(first, second);
        self.undo.push(Undo::Swap(first, second));
        true
    }

    fn finish(&mut self, accepted: bool) {
        if accepted {
            self.undo.clear();
            return;
        }
        while let Some(edit) = self.undo.pop() {
            match edit {
                Undo::Set(index, old) => self.storage.as_mut_slice()[index] = old,
                Undo::Insert(index) => {
                    if let HookListStorage::Resizable(list) = &mut self.storage {
                        list.remove(index);
                    }
                }
                Undo::Remove(index, old) => {
                    if let HookListStorage::Resizable(list) = &mut self.storage {
                        list.insert(index, old);
                    }
                }
                Undo::Swap(first, second) => self.storage.as_mut_slice().swap(first, second),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    #[test]
    fn list_validates_only_submitted_elements_and_unwinds_only_its_own_edits() {
        let mut values = vec![-1, 1]; // The first engine entry has no public projection.
        let reads = Cell::new(0);
        let writes = Cell::new(0);
        let mut journal = HookListJournal::new(
            HookListStorage::Resizable(&mut values),
            |value: &i32| {
                reads.set(reads.get() + 1);
                (*value >= 0).then_some(*value)
            },
            |value: &i32| {
                writes.set(writes.get() + 1);
                (*value >= 0).then_some(*value).ok_or("negative")
            },
        );
        let mut list = HookList::new(&mut journal, "test", "values");
        list.enter("first");
        assert_eq!(list.push(2), Ok(()));
        assert_eq!(list.set(1, -2), Err("negative"));
        list.finish(true);
        list.enter("second");
        assert_eq!(list.get(2), Some(2));
        assert!(list.remove(0));
        assert!(list.swap(0, 1));
        assert_eq!(list.set(0, 3), Ok(true));
        assert_eq!(list.insert(1, 4), Ok(true));
        list.finish(false);
        assert_eq!(values, [-1, 1, 2]);
        assert_eq!(reads.get(), 1);
        assert_eq!(writes.get(), 4);
    }
}
