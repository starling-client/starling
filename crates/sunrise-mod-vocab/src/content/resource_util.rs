//! Stock resource handlers and transfer helpers, following NeoForge's `ResourceHandlerUtil`.
//! Writes and probes take `Some(tx)` for the innermost open transaction or `None` to open a root.
//! Passing a context that does not match the host's innermost open transaction (including `None`
//! while a transaction is open) returns `TransactionError::InvalidContext` before consulting
//! handlers. Probes always roll back. A scope that cannot open is `Unavailable`; a rollback at
//! requested commit is `CommitRefused`. Neither is a zero transfer or a failed match.
use crate::content::resource::{
    advance_transfer_index, checked_transfer, in_turn, insert_candidate, uniform_slots_end,
};
use crate::{
    Capability, HostTransaction, Overanswered, ResourceHandler, ServerHost, StorageSlot,
    TransactionContext, TransactionError,
};
use std::ops::Range;

/// Several handlers as one (`CombinedResourceHandler`): their indices one after another, the
/// first handler's from 0. An insert or extract without an index asks each handler in turn, so
/// each places it its own way.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[non_exhaustive]
pub struct CombinedResourceHandler<R> {
    handlers: Vec<(Box<dyn ResourceHandler<R>>, usize)>,
    size: usize,
}

impl<R> CombinedResourceHandler<R> {
    /// `handlers`, numbered in this order. Each handler's size is sampled once, so indices
    /// stay assigned to the same handler for the lifetime of the wrapper.
    pub fn new(host: &mut dyn ServerHost, handlers: Vec<Box<dyn ResourceHandler<R>>>) -> Self {
        let mut size = 0usize;
        let handlers = handlers
            .into_iter()
            .map(|handler| {
                let slots = handler.size(host);
                size = size.saturating_add(slots);
                (handler, slots)
            })
            .collect();
        Self { handlers, size }
    }

    /// The handler `index` falls in, and the index within it.
    fn locate(&self, index: usize) -> Option<(&dyn ResourceHandler<R>, usize)> {
        let (handler, range) = self.locate_range(index..self.size)?;
        Some((handler, range.start))
    }

    /// The prefix of `indices` inside its first handler, in that handler's coordinates.
    fn locate_range(
        &self,
        indices: Range<usize>,
    ) -> Option<(&dyn ResourceHandler<R>, Range<usize>)> {
        let end = indices.end.min(self.size);
        let mut start = indices.start;
        if start >= end {
            return None;
        }
        for (handler, size) in &self.handlers {
            if start < *size {
                let offset = indices.start - start;
                return Some((handler.as_ref(), start..(end - offset).min(*size)));
            }
            start -= size;
        }
        None
    }
}

impl<R> ResourceHandler<R> for CombinedResourceHandler<R> {
    /// The handlers' original sizes added up; `usize::MAX` at most.
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
    }
    fn next_transfer_index(
        &self,
        host: &mut dyn ServerHost,
        indices: Range<usize>,
        inserting: Option<&R>,
    ) -> Option<usize> {
        let mut offset: usize = 0;
        for (handler, size) in &self.handlers {
            let end = offset.saturating_add(*size).min(indices.end);
            let start = offset.max(indices.start);
            if start < end {
                let mut relative = (start - offset)..(end - offset);
                if let Some(index) =
                    advance_transfer_index(handler.as_ref(), host, &mut relative, inserting)
                {
                    return Some(offset + index);
                }
            }
            if end == indices.end {
                break;
            }
            offset = end;
        }
        None
    }
    fn uniform_slots_end(&self, host: &mut dyn ServerHost, indices: Range<usize>) -> usize {
        let Some((handler, relative)) = self.locate_range(indices.clone()) else {
            return indices.end;
        };
        let offset = indices.start - relative.start;
        offset + uniform_slots_end(handler, host, relative)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        let (handler, at) = self.locate(index)?;
        handler.resource(host, at)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.locate(index)
            .map_or(0, |(handler, at)| handler.amount(host, at))
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
        self.locate(index)
            .map_or(0, |(handler, at)| handler.capacity(host, at, resource))
    }
    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        self.locate(index)
            .is_some_and(|(handler, at)| handler.is_valid(host, at, resource))
    }
    fn backing(&self, host: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
        let (handler, at) = self.locate(index)?;
        handler.backing(host, at)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.locate(index).map_or(Ok(0), |(handler, at)| {
            handler.insert_at(host, tx, at, resource, amount)
        })
    }
    fn insert_at_in_range(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        indices: Range<usize>,
        resource: &R,
        amount: u32,
    ) -> Result<(u32, usize), TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let Some((handler, relative)) = self.locate_range(indices.clone()) else {
            return Ok((0, indices.end));
        };
        let offset = indices.start - relative.start;
        let (wrote, next) = insert_candidate(handler, host, tx, relative, resource, amount)?;
        Ok((wrote, offset + next))
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.locate(index).map_or(Ok(0), |(handler, at)| {
            handler.extract_at(host, tx, at, resource, amount)
        })
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let mut handlers = self.handlers.iter();
        in_turn(
            host,
            amount,
            |_| handlers.next(),
            |host, tx, (handler, _), left| handler.insert(host, tx, resource, left),
        )
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let mut handlers = self.handlers.iter();
        in_turn(
            host,
            amount,
            |_| handlers.next(),
            |host, tx, (handler, _), left| handler.extract(host, tx, resource, left),
        )
    }
}

/// A checked range of another handler's indices, numbered from 0. A machine can expose only
/// its input slots on one side this way. To delegate without changing the visible range, implement
/// [`ResourceHandler`] on a struct holding the handler and forward to it:
///
/// ```
/// use mod_api::{
///     Overanswered, ResourceHandler, ServerHost, StorageSlot, TransactionContext, TransactionError,
/// };
/// use std::ops::Range;
///
/// struct Delegating<R>(Box<dyn ResourceHandler<R>>);
///
/// impl<R> ResourceHandler<R> for Delegating<R> {
///     fn size(&self, host: &mut dyn ServerHost) -> usize {
///         self.0.size(host)
///     }
///     fn next_transfer_index(&self, host: &mut dyn ServerHost, indices: Range<usize>, inserting: Option<&R>) -> Option<usize> {
///         self.0.next_transfer_index(host, indices, inserting)
///     }
///     fn uniform_slots_end(&self, host: &mut dyn ServerHost, indices: Range<usize>) -> usize {
///         self.0.uniform_slots_end(host, indices)
///     }
///     fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
///         self.0.resource(host, index)
///     }
///     fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
///         self.0.amount(host, index)
///     }
///     fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
///         self.0.capacity(host, index, resource)
///     }
///     fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
///         self.0.is_valid(host, index, resource)
///     }
///     fn backing(&self, host: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
///         self.0.backing(host, index)
///     }
///     fn insert_at(
///         &self,
///         host: &mut dyn ServerHost,
///         tx: &TransactionContext,
///         index: usize,
///         resource: &R,
///         amount: u32,
///     ) -> Result<u32, TransactionError<Overanswered>> {
///         self.0.insert_at(host, tx, index, resource, amount)
///     }
///     fn insert_at_in_range(
///         &self,
///         host: &mut dyn ServerHost,
///         tx: &TransactionContext,
///         indices: Range<usize>,
///         resource: &R,
///         amount: u32,
///     ) -> Result<(u32, usize), TransactionError<Overanswered>> {
///         self.0.insert_at_in_range(host, tx, indices, resource, amount)
///     }
///     fn extract_at(
///         &self,
///         host: &mut dyn ServerHost,
///         tx: &TransactionContext,
///         index: usize,
///         resource: &R,
///         amount: u32,
///     ) -> Result<u32, TransactionError<Overanswered>> {
///         self.0.extract_at(host, tx, index, resource, amount)
///     }
///     fn insert(&self, host: &mut dyn ServerHost, tx: &TransactionContext, r: &R, n: u32) -> Result<u32, TransactionError<Overanswered>> {
///         self.0.insert(host, tx, r, n)
///     }
///     fn extract(&self, host: &mut dyn ServerHost, tx: &TransactionContext, r: &R, n: u32) -> Result<u32, TransactionError<Overanswered>> {
///         self.0.extract(host, tx, r, n)
///     }
/// }
/// ```
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[non_exhaustive]
pub struct RangedResourceHandler<R> {
    handler: Box<dyn ResourceHandler<R>>,
    range: Range<usize>,
}

impl<R> RangedResourceHandler<R> {
    /// Wraps a non-empty range, or returns `None` if it is reversed, empty or outside `handler`.
    pub fn of(
        host: &mut dyn ServerHost,
        handler: Box<dyn ResourceHandler<R>>,
        range: Range<usize>,
    ) -> Option<Self> {
        (range.start < range.end && range.end <= handler.size(host))
            .then_some(Self { handler, range })
    }

    /// Wraps one index, or returns `None` if it is outside `handler`.
    pub fn of_single_index(
        host: &mut dyn ServerHost,
        handler: Box<dyn ResourceHandler<R>>,
        index: usize,
    ) -> Option<Self> {
        Self::of(host, handler, index..index.checked_add(1)?)
    }

    /// `index`'s index in the wrapped handler, or none past this range.
    fn inner(&self, index: usize) -> Option<usize> {
        (index < self.range.len()).then(|| self.range.start + index)
    }

    fn inner_range(&self, indices: Range<usize>) -> Range<usize> {
        let len = self.range.len();
        (self.range.start + indices.start.min(len))..(self.range.start + indices.end.min(len))
    }
}

impl<R> ResourceHandler<R> for RangedResourceHandler<R> {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.range.len()
    }
    fn next_transfer_index(
        &self,
        host: &mut dyn ServerHost,
        indices: Range<usize>,
        inserting: Option<&R>,
    ) -> Option<usize> {
        let mut relative = self.inner_range(indices);
        advance_transfer_index(self.handler.as_ref(), host, &mut relative, inserting)
            .map(|index| index - self.range.start)
    }
    fn uniform_slots_end(&self, host: &mut dyn ServerHost, indices: Range<usize>) -> usize {
        let relative = self.inner_range(indices.clone());
        if relative.is_empty() {
            return indices.end;
        }
        uniform_slots_end(self.handler.as_ref(), host, relative) - self.range.start
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        let at = self.inner(index)?;
        self.handler.resource(host, at)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.inner(index)
            .map_or(0, |at| self.handler.amount(host, at))
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
        self.inner(index)
            .map_or(0, |at| self.handler.capacity(host, at, resource))
    }
    fn is_valid(&self, host: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        self.inner(index)
            .is_some_and(|at| self.handler.is_valid(host, at, resource))
    }
    fn backing(&self, host: &mut dyn ServerHost, index: usize) -> Option<StorageSlot> {
        let at = self.inner(index)?;
        self.handler.backing(host, at)
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.inner(index).map_or(Ok(0), |at| {
            self.handler.insert_at(host, tx, at, resource, amount)
        })
    }
    fn insert_at_in_range(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        indices: Range<usize>,
        resource: &R,
        amount: u32,
    ) -> Result<(u32, usize), TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let relative = self.inner_range(indices.clone());
        if relative.is_empty() {
            return Ok((0, indices.end));
        }
        let (wrote, next) =
            insert_candidate(self.handler.as_ref(), host, tx, relative, resource, amount)?;
        Ok((wrote, next - self.range.start))
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        self.inner(index).map_or(Ok(0), |at| {
            self.handler.extract_at(host, tx, at, resource, amount)
        })
    }
}

/// No indices, refusing everything (`EmptyResourceHandler`): what a provider answers for a side
/// that reaches nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[non_exhaustive]
pub struct EmptyResourceHandler;

impl<R> ResourceHandler<R> for EmptyResourceHandler {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        0
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<R> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&R>) -> u32 {
        0
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        _: usize,
        _: &R,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(0)
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        _: usize,
        _: &R,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(0)
    }
}

/// One index of an endless amount of one resource (`InfiniteResourceHandler`): every insert and
/// extract of it is taken whole, and anything else is refused. A creative tank.
#[derive(Clone, Debug, PartialEq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[non_exhaustive]
pub struct InfiniteResourceHandler<R>(R);

impl<R> InfiniteResourceHandler<R> {
    /// An endless amount of `resource`.
    pub fn new(resource: R) -> Self {
        Self(resource)
    }
}

impl<R: Clone + PartialEq> ResourceHandler<R> for InfiniteResourceHandler<R> {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, index: usize) -> Option<R> {
        (index == 0).then(|| self.0.clone())
    }
    fn amount(&self, _: &mut dyn ServerHost, index: usize) -> u32 {
        if index == 0 { u32::MAX } else { 0 }
    }
    fn capacity(&self, _: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
        if index == 0 && resource.is_none_or(|resource| *resource == self.0) {
            u32::MAX
        } else {
            0
        }
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(if self.is_valid(host, index, resource) {
            amount
        } else {
            0
        })
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(if self.is_valid(host, index, resource) {
            amount
        } else {
            0
        })
    }
}

/// One index that destroys whatever goes in and holds nothing (`VoidingResourceHandler`): a
/// trash can.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
#[non_exhaustive]
pub struct VoidingResourceHandler;

impl<R> ResourceHandler<R> for VoidingResourceHandler {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<R> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, index: usize, _: Option<&R>) -> u32 {
        if index == 0 { u32::MAX } else { 0 }
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        index: usize,
        _: &R,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(if index == 0 { amount } else { 0 })
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        _: usize,
        _: &R,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        Ok(0)
    }
}

/// The first index holding `resource` (`indexOf`).
pub fn index_of<R: PartialEq>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
) -> Option<usize> {
    let mut indices = 0..handler.size(host);
    while let Some(index) = advance_transfer_index(handler, host, &mut indices, None) {
        if handler.amount(host, index) > 0
            && handler.resource(host, index).as_ref() == Some(resource)
        {
            return Some(index);
        }
    }
    None
}

/// Whether an index holds `resource` (`contains`).
pub fn contains<R: PartialEq>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
) -> bool {
    index_of(host, handler, resource).is_some()
}

/// Whether all indices hold nothing (`isEmpty`); a handler with no indices is empty.
pub fn is_empty<R>(host: &mut dyn ServerHost, handler: &dyn ResourceHandler<R>) -> bool {
    let mut indices = 0..handler.size(host);
    while let Some(index) = advance_transfer_index(handler, host, &mut indices, None) {
        if handler.amount(host, index) > 0 {
            return false;
        }
    }
    true
}

/// Visit occupied candidates individually and known-empty gaps in runs with equal slot rules.
pub(crate) fn visit_slots<R, E>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    mut visit: impl FnMut(&mut dyn ServerHost, Range<usize>, Option<R>) -> Result<(), E>,
) -> Result<(), E> {
    let mut indices = 0..handler.size(host);
    while !indices.is_empty() {
        let end = uniform_slots_end(handler, host, indices.clone());
        let mut run = indices.start..end;
        let mut start = run.start;
        while let Some(index) = advance_transfer_index(handler, host, &mut run, None) {
            if start < index {
                visit(host, start..index, None)?;
            }
            let resource = handler.resource(host, index);
            visit(host, index..run.start, resource)?;
            start = run.start;
        }
        if start < end {
            visit(host, start..end, None)?;
        }
        indices.start = end;
    }
    Ok(())
}

/// Whether every index has reached capacity (`isFull`); true for no indices.
/// An empty index uses its general capacity, so a zero-capacity index is already full.
pub fn is_full<R>(host: &mut dyn ServerHost, handler: &dyn ResourceHandler<R>) -> bool {
    visit_slots(host, handler, |host, slots, resource| {
        let index = slots.start;
        (handler.amount(host, index) >= handler.capacity(host, index, resource.as_ref()))
            .then_some(())
            .ok_or(())
    })
    .is_ok()
}

/// Whether any index accepts `resource` (`ResourceHandlerUtil.isValid`), regardless of space.
pub fn is_valid<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
) -> bool {
    let mut indices = 0..handler.size(host);
    while !indices.is_empty() {
        if handler.is_valid(host, indices.start, resource) {
            return true;
        }
        indices.start = uniform_slots_end(handler, host, indices.clone());
    }
    false
}

/// The vanilla container comparator scale (0..=15), using all indices as the denominator.
/// Empty and zero-capacity indices contribute 0; each overfilled index contributes at most 1.
/// With no indices the signal is 0.
pub fn comparator_signal<R>(host: &mut dyn ServerHost, handler: &dyn ResourceHandler<R>) -> u8 {
    let size = handler.size(host);
    if size == 0 {
        return 0;
    }
    let mut fill = 0f32;
    let mut indices = 0..size;
    while let Some(index) = advance_transfer_index(handler, host, &mut indices, None) {
        let Some(resource) = handler.resource(host, index) else {
            continue;
        };
        let amount = handler.amount(host, index);
        let capacity = handler.capacity(host, index, Some(&resource));
        if amount > 0 && capacity > 0 {
            fill += (amount as f32 / capacity as f32).min(1.0);
        }
    }
    let fill = fill / size as f32;
    // `Mth.lerpDiscrete(fill, 0, 15)`.
    (fill * 14.0).floor() as u8 + u8::from(fill > 0.0)
}

/// Inserts up to `amount` of `resource`, trying occupied indices before empty ones.
/// The handler decides whether each index accepts the offered resource.
/// An overanswer rolls back the insertion and returns `TransactionError::Aborted(Overanswered)`.
/// Pass `Some(tx)` for the innermost open transaction or `None` to open a root.
/// A mismatch returns `TransactionError::InvalidContext` without calling the handler.
pub fn insert_stacking<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
    amount: u32,
    tx: Option<&TransactionContext>,
) -> Result<u32, TransactionError<Overanswered>> {
    if !host.transaction_matches(tx) {
        return Err(TransactionError::InvalidContext);
    }
    if amount == 0 {
        return Ok(0);
    }
    host.transaction(|host, tx| try_insert_stacking(host, &handler.into(), resource, amount, tx))
        .map_err(TransactionError::flatten)
}

fn try_insert_stacking<R>(
    host: &mut dyn ServerHost,
    endpoint: &Attributed<'_, R>,
    resource: &R,
    amount: u32,
    tx: &TransactionContext,
) -> Result<u32, TransactionError<Overanswered>> {
    let handler = endpoint.handler;
    let size = handler.size(host);
    let mut done = 0;
    for inserting in [None, Some(resource)] {
        let mut indices = 0..size;
        while done < amount {
            let Some(index) = advance_transfer_index(handler, host, &mut indices, inserting) else {
                break;
            };
            let matches = if inserting.is_none() {
                handler.resource(host, index).is_some()
            } else {
                handler.resource(host, index).is_none()
            };
            if matches {
                let left = amount - done;
                let (wrote, next) =
                    insert_candidate(handler, host, tx, index..indices.end, resource, left)
                        .map_err(|error| endpoint.attribute(error))?;
                indices.start = next;
                done += checked_transfer(wrote, left)
                    .map_err(|error| endpoint.attribute(TransactionError::Aborted(error)))?;
            }
        }
    }
    Ok(done)
}

/// Takes up to `amount` of the first extractable resource `filter` accepts. A positive amount
/// opens a rollback probe before scanning, even if nothing matches. An overanswer aborts the
/// extraction. Pass `Some(tx)` for the innermost open transaction or `None` to open a root.
/// A scope refusal is an error, not `Ok(None)`. Passing a context that does not match the host's
/// innermost transaction (including `None` while one is open) returns `InvalidContext`.
pub fn extract_first<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
    amount: u32,
    tx: Option<&TransactionContext>,
) -> Result<Option<(R, u32)>, TransactionError<Overanswered>> {
    if !host.transaction_matches(tx) {
        return Err(TransactionError::InvalidContext);
    }
    if amount == 0 {
        return Ok(None);
    }
    let Some(resource) = probe_extractable(host, handler, filter)? else {
        return Ok(None);
    };
    match host.transaction(|host, tx| {
        let taken = handler.extract(host, tx, &resource, amount).map_err(Some)?;
        let taken = checked_transfer(taken, amount)
            .map_err(|error| Some(TransactionError::Aborted(error)))?;
        if taken == 0 {
            Err(None)
        } else {
            Ok((resource, taken))
        }
    }) {
        Ok(taken) => Ok(Some(taken)),
        Err(TransactionError::Aborted(None)) => Ok(None),
        Err(TransactionError::Aborted(Some(error))) => Err(error),
        Err(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
        Err(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
        Err(TransactionError::CommitRefused) => Err(TransactionError::CommitRefused),
    }
}

/// The first resource `filter` accepts that can be extracted. One rollback scope covers
/// the whole scan, including all writes made by earlier attempts. Pass `Some(tx)` for the
/// innermost open transaction or `None` to probe at the root. A mismatch returns
/// `TransactionError::InvalidContext` without opening the scan.
/// Returns `Ok(None)` if nothing matched, or `Err` for refusal or an overanswer.
pub fn find_extractable_resource<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
    tx: Option<&TransactionContext>,
) -> Result<Option<R>, TransactionError<Overanswered>> {
    if !host.transaction_matches(tx) {
        return Err(TransactionError::InvalidContext);
    }
    probe_extractable(host, handler, filter)
}

fn probe_extractable<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
) -> Result<Option<R>, TransactionError<Overanswered>> {
    let probe: Result<std::convert::Infallible, _> = host.transaction(|host, tx| {
        let mut indices = 0..handler.size(host);
        while let Some(index) = advance_transfer_index(handler, host, &mut indices, None) {
            let Some(resource) = handler.resource(host, index).filter(|r| filter(r)) else {
                continue;
            };
            let Ok(amount) = u32::try_from(handler.amount_int(host, index)) else {
                continue;
            };
            let answer = handler
                .extract_at(host, tx, index, &resource, amount)
                .map_err(Err)?;
            if checked_transfer(answer, amount)
                .map_err(|error| Err(TransactionError::Aborted(error)))?
                > 0
            {
                return Err(Ok(Some(resource)));
            }
        }
        Err(Ok(None))
    });
    match probe {
        Ok(never) => match never {},
        Err(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
        Err(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
        Err(TransactionError::CommitRefused) => Err(TransactionError::CommitRefused),
        Err(TransactionError::Aborted(result)) => result,
    }
}

/// A borrowed move endpoint. Pass a capability reference to retain its provider's name in
/// [`Overanswered`], or a `&dyn ResourceHandler<R>` for a directly composed handler.
// no wire: borrows a process-local trait object.
#[non_exhaustive]
pub struct Attributed<'a, R> {
    handler: &'a (dyn ResourceHandler<R> + 'a),
    owner: Option<&'static str>,
}

impl<'a, R> From<&'a Capability<dyn ResourceHandler<R>>> for Attributed<'a, R> {
    fn from(capability: &'a Capability<dyn ResourceHandler<R>>) -> Self {
        Self {
            handler: capability,
            owner: Some(capability.owner.unwrap_or("the game")),
        }
    }
}

impl<'a, R> From<&'a (dyn ResourceHandler<R> + 'a)> for Attributed<'a, R> {
    fn from(handler: &'a (dyn ResourceHandler<R> + 'a)) -> Self {
        Self {
            handler,
            owner: None,
        }
    }
}

impl<R> Attributed<'_, R> {
    fn attribute(
        &self,
        mut error: TransactionError<Overanswered>,
    ) -> TransactionError<Overanswered> {
        if let TransactionError::Aborted(over) = &mut error
            && over.owner.is_none()
        {
            over.owner = self.owner.map(str::to_owned);
        }
        error
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &R,
        asked: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        self.handler
            .insert(host, tx, resource, asked)
            .and_then(|answer| checked_transfer(answer, asked).map_err(TransactionError::Aborted))
            .map_err(|error| self.attribute(error))
    }
}

/// Moves up to `max` of the resources `filter` accepts, in index order. The helper probes what
/// it can extract, rolls the probe back, inserts, then extracts exactly what went in. A step
/// that cannot extract that exact amount rolls back. `TransactionError::Aborted(Overanswered)` rolls back the entire
/// move, even if the caller catches the error and commits the enclosing transaction. Pass a
/// capability reference to retain its provider in the error, or a `&dyn ResourceHandler<R>`
/// for a directly composed handler.
///
/// Pass `Some(tx)` to run under the innermost open transaction, or `None` to open a root.
/// A mismatch returns `TransactionError::InvalidContext`. With a valid context, a zero
/// request returns zero without opening any scope. Each source index is visited once,
/// probing up to the still-requested amount.
pub fn move_resources<'a, R: 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
) -> Result<u32, TransactionError<Overanswered>> {
    let (from, to) = (from.into(), to.into());
    move_resources_by(host, from, filter, max, tx, |host, tx, resource, out| {
        to.insert(host, tx, resource, out)
    })
}

/// As [`move_resources`], trying occupied destination indices before empty ones.
pub fn move_resources_stacking<'a, R: 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
) -> Result<u32, TransactionError<Overanswered>> {
    let (from, to) = (from.into(), to.into());
    move_resources_by(host, from, filter, max, tx, |host, tx, resource, out| {
        try_insert_stacking(host, &to, resource, out, tx)
    })
}

fn move_resources_by<R>(
    host: &mut dyn ServerHost,
    from: Attributed<'_, R>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
    insert: impl Fn(
        &mut dyn ServerHost,
        &TransactionContext,
        &R,
        u32,
    ) -> Result<u32, TransactionError<Overanswered>>,
) -> Result<u32, TransactionError<Overanswered>> {
    with_move_transaction(host, tx, max, 0, |host| {
        let mut moved = 0;
        let mut indices = 0..from.handler.size(host);
        while moved < max {
            let Some(index) = advance_transfer_index(from.handler, host, &mut indices, None) else {
                break;
            };
            let Some(resource) = from.handler.resource(host, index).filter(|r| filter(r)) else {
                continue;
            };
            moved += move_slot(host, &from, (index, &resource), max - moved, false, &insert)?;
        }
        Ok(moved)
    })
}

/// As [`move_resources`], but only moves the first matching resource the destination accepts.
/// Once selected, that same resource can also be moved from later source indices.
pub fn move_first<'a, R: PartialEq + 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
) -> Result<Option<(R, u32)>, TransactionError<Overanswered>> {
    let (from, to) = (from.into(), to.into());
    move_first_by(host, from, filter, max, tx, |host, tx, resource, out| {
        to.insert(host, tx, resource, out)
    })
}

/// As [`move_first`], trying occupied destination indices before empty ones.
pub fn move_first_stacking<'a, R: PartialEq + 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
) -> Result<Option<(R, u32)>, TransactionError<Overanswered>> {
    let (from, to) = (from.into(), to.into());
    move_first_by(host, from, filter, max, tx, |host, tx, resource, out| {
        try_insert_stacking(host, &to, resource, out, tx)
    })
}

fn move_first_by<R: PartialEq>(
    host: &mut dyn ServerHost,
    from: Attributed<'_, R>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&TransactionContext>,
    insert: impl Fn(
        &mut dyn ServerHost,
        &TransactionContext,
        &R,
        u32,
    ) -> Result<u32, TransactionError<Overanswered>>,
) -> Result<Option<(R, u32)>, TransactionError<Overanswered>> {
    with_move_transaction(host, tx, max, None, |host| {
        let mut selected = None;
        let mut moved = 0;
        let mut indices = 0..from.handler.size(host);
        while moved < max {
            let Some(index) = advance_transfer_index(from.handler, host, &mut indices, None) else {
                break;
            };
            let Some(resource) = from.handler.resource(host, index) else {
                continue;
            };
            if !selected
                .as_ref()
                .map_or_else(|| filter(&resource), |r| r == &resource)
            {
                continue;
            }
            let went = move_slot(host, &from, (index, &resource), max - moved, true, &insert)?;
            if went == 0 {
                continue;
            }
            moved += went;
            selected = Some(resource);
        }
        Ok(selected.map(|r| (r, moved)))
    })
}

fn with_move_transaction<T>(
    host: &mut dyn ServerHost,
    tx: Option<&TransactionContext>,
    max: u32,
    empty: T,
    run: impl FnOnce(&mut dyn ServerHost) -> Result<T, TransactionError<Overanswered>>,
) -> Result<T, TransactionError<Overanswered>> {
    if !host.transaction_matches(tx) {
        return Err(TransactionError::InvalidContext);
    }
    if max == 0 {
        return Ok(empty);
    }
    // Keep the checked move atomic if a later index gives a malformed answer.
    host.transaction(|host, _| run(host))
        .map_err(TransactionError::flatten)
}

fn move_slot<R>(
    host: &mut dyn ServerHost,
    from: &Attributed<'_, R>,
    (index, resource): (usize, &R),
    want: u32,
    skip_rejected: bool,
    insert: &impl Fn(
        &mut dyn ServerHost,
        &TransactionContext,
        &R,
        u32,
    ) -> Result<u32, TransactionError<Overanswered>>,
) -> Result<u32, TransactionError<Overanswered>> {
    // The destination must see the source before extraction, even though the probe may write.
    let probe: Result<
        std::convert::Infallible,
        TransactionError<Result<u32, TransactionError<Overanswered>>>,
    > = host.transaction(|host, tx| Err(from.handler.extract_at(host, tx, index, resource, want)));
    let extracted = match probe {
        Ok(never) => match never {},
        Err(TransactionError::InvalidContext) => return Err(TransactionError::InvalidContext),
        Err(TransactionError::Unavailable) => return Err(TransactionError::Unavailable),
        Err(TransactionError::CommitRefused) => return Err(TransactionError::CommitRefused),
        Err(TransactionError::Aborted(Ok(extracted))) => extracted,
        Err(TransactionError::Aborted(Err(error))) => return Err(from.attribute(error)),
    };
    let available = checked_transfer(extracted, want)
        .map_err(|e| from.attribute(TransactionError::Aborted(e)))?;
    if available == 0 {
        return Ok(0);
    }
    match host.transaction(|host, tx| {
        let accepted = insert(host, tx, resource, available).map_err(Some)?;
        // `moveFirst` does not extract from a rejected resource or select it.
        if skip_rejected && accepted == 0 {
            return Err(None);
        }
        let taken = from
            .handler
            .extract_at(host, tx, index, resource, accepted)
            .map_err(|e| Some(from.attribute(e)))?;
        let taken = checked_transfer(taken, accepted)
            .map_err(|e| Some(from.attribute(TransactionError::Aborted(e))))?;
        if taken != accepted {
            return Err(None);
        }
        Ok(accepted)
    }) {
        Ok(accepted) => Ok(accepted),
        Err(TransactionError::Aborted(Some(over))) => Err(over),
        Err(TransactionError::Aborted(None)) => Ok(0),
        Err(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
        Err(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
        Err(TransactionError::CommitRefused) => Err(TransactionError::CommitRefused),
    }
}

#[cfg(test)]
#[path = "resource_util_tests.rs"]
mod tests;
