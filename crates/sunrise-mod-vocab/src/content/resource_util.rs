//! Stock resource handlers and transfer helpers, following NeoForge's `ResourceHandlerUtil`.
//! Writes and probes take `Some(tx)` for the current transaction or `None` to open a root.
//! They refuse without calling handlers if the token does not name the current transaction,
//! or if a transaction is open but no token was supplied. Probes always roll back.
use crate::content::resource::{checked_transfer, in_turn, try_in_turn};
use crate::{Capability, HostTransaction, Overanswered, ResourceHandler, ServerHost, Transaction};
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
    fn locate(&self, mut index: usize) -> Option<(&dyn ResourceHandler<R>, usize)> {
        if index >= self.size {
            return None;
        }
        for (handler, size) in &self.handlers {
            if index < *size {
                return Some((handler.as_ref(), index));
            }
            index -= size;
        }
        None
    }
}

impl<R> ResourceHandler<R> for CombinedResourceHandler<R> {
    /// The handlers' original sizes added up; `usize::MAX` at most.
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
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
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.locate(index).map_or(0, |(handler, at)| {
            handler.insert_at(host, tx, at, resource, amount)
        })
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.locate(index).map_or(0, |(handler, at)| {
            handler.extract_at(host, tx, at, resource, amount)
        })
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> Result<u32, Overanswered> {
        in_turn(
            host,
            tx,
            amount,
            &self.handlers,
            |host, tx, (handler, _), left| handler.insert(host, tx, resource, left),
        )
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        amount: u32,
    ) -> Result<u32, Overanswered> {
        in_turn(
            host,
            tx,
            amount,
            &self.handlers,
            |host, tx, (handler, _), left| handler.extract(host, tx, resource, left),
        )
    }
}

/// A checked range of another handler's indices, numbered from 0. A machine can expose only
/// its input slots on one side this way. To delegate without changing the visible range, implement
/// [`ResourceHandler`] on a struct holding the handler and forward to it:
///
/// ```
/// use mod_api::{Overanswered, ResourceHandler, ServerHost, Transaction};
///
/// struct Delegating<R>(Box<dyn ResourceHandler<R>>);
///
/// impl<R> ResourceHandler<R> for Delegating<R> {
///     fn size(&self, host: &mut dyn ServerHost) -> usize {
///         self.0.size(host)
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
///     fn insert_at(
///         &self,
///         host: &mut dyn ServerHost,
///         tx: &Transaction,
///         index: usize,
///         resource: &R,
///         amount: u32,
///     ) -> u32 {
///         self.0.insert_at(host, tx, index, resource, amount)
///     }
///     fn extract_at(
///         &self,
///         host: &mut dyn ServerHost,
///         tx: &Transaction,
///         index: usize,
///         resource: &R,
///         amount: u32,
///     ) -> u32 {
///         self.0.extract_at(host, tx, index, resource, amount)
///     }
///     fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, r: &R, n: u32) -> Result<u32, Overanswered> {
///         self.0.insert(host, tx, r, n)
///     }
///     fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, r: &R, n: u32) -> Result<u32, Overanswered> {
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
}

impl<R> ResourceHandler<R> for RangedResourceHandler<R> {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.range.len()
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
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.inner(index).map_or(0, |at| {
            self.handler.insert_at(host, tx, at, resource, amount)
        })
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        self.inner(index).map_or(0, |at| {
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
    fn insert_at(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, _: u32) -> u32 {
        0
    }
    fn extract_at(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, _: u32) -> u32 {
        0
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
        _: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        if self.is_valid(host, index, resource) {
            amount
        } else {
            0
        }
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> u32 {
        if self.is_valid(host, index, resource) {
            amount
        } else {
            0
        }
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
        _: &mut dyn ServerHost,
        _: &Transaction,
        index: usize,
        _: &R,
        amount: u32,
    ) -> u32 {
        if index == 0 { amount } else { 0 }
    }
    fn extract_at(&self, _: &mut dyn ServerHost, _: &Transaction, _: usize, _: &R, _: u32) -> u32 {
        0
    }
}

/// The first index holding `resource` (`indexOf`).
pub fn index_of<R: PartialEq>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
) -> Option<usize> {
    (0..handler.size(host)).find(|&index| {
        handler.amount(host, index) > 0 && handler.resource(host, index).as_ref() == Some(resource)
    })
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
    (0..handler.size(host)).all(|index| handler.amount(host, index) == 0)
}

/// Whether every index has reached capacity (`isFull`); true for no indices.
/// An empty index uses its general capacity, so a zero-capacity index is already full.
pub fn is_full<R>(host: &mut dyn ServerHost, handler: &dyn ResourceHandler<R>) -> bool {
    (0..handler.size(host)).all(|index| {
        let resource = handler.resource(host, index);
        handler.amount(host, index) >= handler.capacity(host, index, resource.as_ref())
    })
}

/// Whether any index accepts `resource` (`ResourceHandlerUtil.isValid`), regardless of space.
pub fn is_valid<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
) -> bool {
    (0..handler.size(host)).any(|index| handler.is_valid(host, index, resource))
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
    for index in 0..size {
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

/// Inserts up to `amount` of `resource`, topping up matching indices before empty ones.
/// An overanswer rolls back the insertion and returns [`Overanswered`]. Pass `Some(tx)` for
/// the current transaction or `None` to open a root. A mismatched token, including a missing
/// token when a transaction is open, returns `Ok(0)` without calling the handler.
pub fn insert_stacking<R: PartialEq>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    resource: &R,
    amount: u32,
    tx: Option<&Transaction>,
) -> Result<u32, Overanswered> {
    if !can_open(host, tx) || amount == 0 {
        return Ok(0);
    }
    host.transaction(|host, tx| try_insert_stacking(host, &handler.into(), resource, amount, tx))
}

fn try_insert_stacking<R: PartialEq>(
    host: &mut dyn ServerHost,
    endpoint: &Attributed<'_, R>,
    resource: &R,
    amount: u32,
    tx: &Transaction,
) -> Result<u32, Overanswered> {
    let handler = endpoint.handler;
    let size = handler.size(host);
    let indices = [true, false]
        .into_iter()
        .flat_map(|holding| (0..size).map(move |index| (holding, index)));
    try_in_turn(
        host,
        tx,
        amount,
        indices,
        |host, tx, &(holding, index), left| {
            let matches = if holding {
                handler.amount(host, index) > 0
                    && handler.resource(host, index).as_ref() == Some(resource)
            } else {
                handler.amount(host, index) == 0
            };
            Ok(if matches {
                handler.insert_at(host, tx, index, resource, left)
            } else {
                0
            })
        },
    )
    .map_err(|error| endpoint.attribute(error))
}

/// Takes up to `amount` of the first extractable resource `filter` accepts. An overanswer
/// aborts the extraction. Pass `Some(tx)` for the current transaction or `None` to open a root.
/// A mismatched token, including a missing token when a transaction is open, returns `None`
/// without calling the handler.
pub fn extract_first<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
    amount: u32,
    tx: Option<&Transaction>,
) -> Option<(R, u32)> {
    if !can_open(host, tx) || amount == 0 {
        return None;
    }
    host.transaction(|host, tx| {
        let resource = probe_extractable(host, handler, filter).ok_or(())?;
        let taken = handler
            .extract(host, tx, &resource, amount)
            .map_err(|_| ())?;
        let taken = checked_transfer(taken, amount).map_err(|_| ())?;
        if taken == 0 {
            Err(())
        } else {
            Ok((resource, taken))
        }
    })
    .ok()
}

/// The first resource `filter` accepts that can be extracted. Probes with the slot's whole
/// int amount in a transaction that always rolls back. Pass `Some(tx)` for the current
/// transaction or `None` to probe at the root. A mismatched token, including a missing token
/// when a transaction is open, returns `None` without calling the handler.
pub fn find_extractable_resource<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
    tx: Option<&Transaction>,
) -> Option<R> {
    if !can_open(host, tx) {
        return None;
    }
    probe_extractable(host, handler, filter)
}

fn probe_extractable<R>(
    host: &mut dyn ServerHost,
    handler: &dyn ResourceHandler<R>,
    filter: impl Fn(&R) -> bool,
) -> Option<R> {
    (0..handler.size(host)).find_map(|index| {
        let amount = u32::try_from(handler.amount_int(host, index)).ok()?;
        let resource = handler
            .resource(host, index)
            .filter(|r| amount != 0 && filter(r))?;
        let probe: Result<std::convert::Infallible, _> = host
            .transaction(|host, tx| Err(handler.extract_at(host, tx, index, &resource, amount)));
        let answer = probe.unwrap_err();
        (answer > 0 && answer <= amount).then_some(resource)
    })
}

/// Validates an optional enclosing transaction before opening any new journal.
fn can_open(host: &dyn ServerHost, tx: Option<&Transaction>) -> bool {
    host.transaction_current().as_ref() == tx
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
    fn attribute(&self, mut error: Overanswered) -> Overanswered {
        if error.owner.is_none() {
            error.owner = self.owner.map(str::to_owned);
        }
        error
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &R,
        asked: u32,
    ) -> Result<u32, Overanswered> {
        self.handler
            .insert(host, tx, resource, asked)
            .and_then(|answer| checked_transfer(answer, asked))
            .map_err(|error| self.attribute(error))
    }
}

/// Moves up to `max` of the resources `filter` accepts, in index order. The helper probes what
/// it can extract, rolls the probe back, inserts, then extracts exactly what went in. A step
/// that cannot extract that exact amount rolls back. [`Overanswered`] rolls back the entire
/// move, even if the caller catches the error and commits the enclosing transaction. Pass a
/// capability reference to retain its provider in the error, or a `&dyn ResourceHandler<R>`
/// for a directly composed handler.
///
/// Pass `Some(tx)` to run under the current transaction, or `None` to open a root. A token
/// that does not name the current transaction, or `None` while one is open, moves nothing.
pub fn move_resources<'a, R: 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&Transaction>,
) -> Result<u32, Overanswered> {
    let (from, to) = (from.into(), to.into());
    move_resources_by(host, from, filter, max, tx, |host, tx, resource, out| {
        to.insert(host, tx, resource, out)
    })
}

/// As [`move_resources`], topping up matching destination indices before filling empty ones.
pub fn move_resources_stacking<'a, R: PartialEq + 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&Transaction>,
) -> Result<u32, Overanswered> {
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
    tx: Option<&Transaction>,
    insert: impl Fn(&mut dyn ServerHost, &Transaction, &R, u32) -> Result<u32, Overanswered>,
) -> Result<u32, Overanswered> {
    with_move_transaction(host, tx, |host| {
        let mut moved = 0;
        for index in 0..from.handler.size(host) {
            if moved == max {
                break;
            }
            while moved < max {
                let Some(resource) = from.handler.resource(host, index).filter(|r| filter(r))
                else {
                    break;
                };
                let went = move_slot(host, &from, index, &resource, max - moved, &insert)?;
                if went == 0 {
                    break;
                }
                moved += went;
            }
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
    tx: Option<&Transaction>,
) -> Result<Option<(R, u32)>, Overanswered> {
    let (from, to) = (from.into(), to.into());
    move_first_by(host, from, filter, max, tx, |host, tx, resource, out| {
        to.insert(host, tx, resource, out)
    })
}

/// As [`move_first`], topping up matching destination indices before filling empty ones.
pub fn move_first_stacking<'a, R: PartialEq + 'a>(
    host: &mut dyn ServerHost,
    from: impl Into<Attributed<'a, R>>,
    to: impl Into<Attributed<'a, R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: Option<&Transaction>,
) -> Result<Option<(R, u32)>, Overanswered> {
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
    tx: Option<&Transaction>,
    insert: impl Fn(&mut dyn ServerHost, &Transaction, &R, u32) -> Result<u32, Overanswered>,
) -> Result<Option<(R, u32)>, Overanswered> {
    with_move_transaction(host, tx, |host| {
        let mut selected = None;
        let mut moved = 0;
        for index in 0..from.handler.size(host) {
            if moved == max {
                break;
            }
            let Some(resource) = from.handler.resource(host, index) else {
                continue;
            };
            if !selected
                .as_ref()
                .map_or_else(|| filter(&resource), |r| r == &resource)
            {
                continue;
            }
            let went = move_slot(host, &from, index, &resource, max - moved, &insert)?;
            if went == 0 {
                continue;
            }
            moved += went;
            selected = Some(resource);
        }
        Ok(selected.map(|r| (r, moved)))
    })
}

fn with_move_transaction<T: Default>(
    host: &mut dyn ServerHost,
    tx: Option<&Transaction>,
    run: impl FnOnce(&mut dyn ServerHost) -> Result<T, Overanswered>,
) -> Result<T, Overanswered> {
    if !can_open(host, tx) {
        return Ok(T::default());
    }
    host.transaction(|host, _| run(host))
}

fn move_slot<R>(
    host: &mut dyn ServerHost,
    from: &Attributed<'_, R>,
    index: usize,
    resource: &R,
    max: u32,
    insert: &impl Fn(&mut dyn ServerHost, &Transaction, &R, u32) -> Result<u32, Overanswered>,
) -> Result<u32, Overanswered> {
    let want = max.min(from.handler.amount(host, index));
    if want == 0 {
        return Ok(0);
    }
    // The destination must see the source before extraction, even though the probe may write.
    let probe: Result<std::convert::Infallible, _> =
        host.transaction(|host, tx| Err(from.handler.extract_at(host, tx, index, resource, want)));
    let available = checked_transfer(probe.unwrap_err(), want).map_err(|e| from.attribute(e))?;
    if available == 0 {
        return Ok(0);
    }
    host.transaction(|host, tx| {
        let accepted = insert(host, tx, resource, available).map_err(Some)?;
        if accepted == 0 {
            return Err(None);
        }
        let taken = from.handler.extract_at(host, tx, index, resource, accepted);
        let taken = checked_transfer(taken, accepted).map_err(|e| Some(from.attribute(e)))?;
        if taken != accepted {
            return Err(None);
        }
        Ok(accepted)
    })
    .or_else(|error| error.map_or(Ok(0), Err))
}

#[cfg(test)]
#[path = "resource_util_tests.rs"]
mod tests;
