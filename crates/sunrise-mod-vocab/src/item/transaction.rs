//! `host.transaction` (#41 §7): NeoForge 26.3's `Transaction`. Changes made through the
//! capability helpers inside one are kept by the game and made final only when the outermost
//! transaction commits; an abort puts back everything its closure changed.
use crate::{ItemPlace, ItemStack, ServerHost, StackWriteError};
use std::{fmt, marker::PhantomData, rc::Rc};

// no wire: only the host can lend this thread-bound authority over a live journal.
/// A loan of write authority in one open transaction, as NeoForge's `TransactionContext`.
/// Pass this loan and the accompanying server host to handler, item-place and world fluid
/// writes. It cannot close the transaction, be encoded as payload data, or outlive the
/// callback it was lent to. A live loan from another host or an enclosing ancestor frame is
/// not authority for the innermost open transaction on this host.
// exhaustive: its fields are private, so mods cannot build or destructure it.
pub struct TransactionContext {
    frame: TransactionFrame,
    _thread: PhantomData<Rc<()>>,
}

impl TransactionContext {
    /// The nesting depth at which this context was lent: zero at the root, plus one per
    /// enclosing transaction. Depth alone cannot identify a frame or journal.
    pub fn depth(&self) -> usize {
        self.frame.depth
    }
}

impl fmt::Debug for TransactionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionContext")
            .field("depth", &self.depth())
            .finish_non_exhaustive()
    }
}

// no wire: process-local identity of a journal frame.
/// A key for the host that owns a journal frame, not a write capability or close handle.
// hidden: the game's side of TransactionContext, re-exported through `__private`.
// exhaustive: opaque hidden handle with private fields, constructed only via TransactionFrame::new.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionFrame {
    journal: usize,
    sequence: u64,
    depth: usize,
}

impl TransactionFrame {
    /// Mints a frame identity scoped to `journal` (a live non-ZST address unique to the
    /// owning journal), monotonically increasing `sequence`, and nesting `depth`.
    pub fn new(journal: usize, sequence: u64, depth: usize) -> Self {
        Self {
            journal,
            sequence,
            depth,
        }
    }

    /// The depth at which this frame was installed.
    pub fn depth(self) -> usize {
        self.depth
    }

    /// Whether the loan came from this journal frame.
    pub fn matches(self, context: &TransactionContext) -> bool {
        self == context.frame
    }

    /// Lends write authority without handing out an owned context.
    pub fn lend<T>(self, f: impl for<'loan> FnOnce(&'loan TransactionContext) -> T) -> T {
        f(&TransactionContext {
            frame: self,
            _thread: PhantomData,
        })
    }
}

// no wire: observes the process-local transaction journal.
/// A read-only observation of the host's journal phase (`Transaction.Lifecycle`), returned by
/// [`transaction_lifecycle`](crate::prelude::ModStateServerHost::transaction_lifecycle).
/// It is not a write or close capability or a Wire payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransactionLifecycle {
    /// No transaction is open.
    None,
    /// A transaction is open and none is closing.
    Open,
    /// A nested transaction is closing: committing into the one around it, or aborting.
    Closing,
    /// The outermost transaction is closing. On commit, effects such as `contents_changed`
    /// and block updates run after its frame has been removed.
    RootClosing,
}

// no wire: local admission refusal, not a mod boundary payload.
// exhaustive: unit error marker carrying no fields.
/// There is no open transaction this caller can borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionUnavailable;

impl fmt::Display for TransactionUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no current transaction is available")
    }
}

impl std::error::Error for TransactionUnavailable {}

// no wire: local scope settlement, not a mod boundary payload.
// exhaustive: all scoped transaction failure outcomes (failed to open, or rolled back on commit).
/// A host could not open a transaction, or refused it during commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionScopeError {
    /// The callback did not run.
    Unavailable,
    /// The callback requested commit, but its writes were rolled back.
    CommitRefused,
}

impl fmt::Display for TransactionScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "transaction unavailable in the current phase",
            Self::CommitRefused => "transaction commit refused; changes rolled back",
        })
    }
}

impl std::error::Error for TransactionScopeError {}

// no wire: the error belongs to a local closure and can contain any Rust type.
// exhaustive: all transaction refusal and callback failure outcomes.
/// Why [`HostTransaction::transaction`] did not commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionError<E> {
    /// The supplied context does not identify the innermost open transaction on this host.
    InvalidContext,
    /// The host could not open a transaction. The closure was not called.
    Unavailable,
    /// The closure returned `Ok`, but the host refused the commit and rolled back its writes.
    CommitRefused,
    /// The closure returned this error. Its writes were rolled back.
    Aborted(E),
}

impl<E> From<TransactionScopeError> for TransactionError<E> {
    fn from(error: TransactionScopeError) -> Self {
        match error {
            TransactionScopeError::Unavailable => Self::Unavailable,
            TransactionScopeError::CommitRefused => Self::CommitRefused,
        }
    }
}

impl<E> TransactionError<TransactionError<E>> {
    pub(crate) fn flatten(self) -> TransactionError<E> {
        match self {
            Self::InvalidContext => TransactionError::InvalidContext,
            Self::Unavailable => TransactionError::Unavailable,
            Self::CommitRefused => TransactionError::CommitRefused,
            Self::Aborted(inner) => inner,
        }
    }
}

impl<E: fmt::Display> fmt::Display for TransactionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContext => f.write_str("invalid transaction context"),
            Self::Unavailable => TransactionScopeError::Unavailable.fmt(f),
            Self::CommitRefused => TransactionScopeError::CommitRefused.fmt(f),
            Self::Aborted(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for TransactionError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidContext | Self::Unavailable | Self::CommitRefused => None,
            Self::Aborted(error) => Some(error),
        }
    }
}

impl<E> crate::NoBackend for TransactionError<E> {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

// no wire: a local scratch transaction refusal, not a mod boundary payload.
// exhaustive: transaction refusal or rejected initial stack.
/// Why this host refused a scratch item-place scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScratchScopeError {
    /// The host could not open or commit the transaction.
    Transaction(TransactionScopeError),
    /// The initial stack did not pass the game's write validation.
    Stack(StackWriteError),
}

impl fmt::Display for ScratchScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transaction(error) => error.fmt(f),
            Self::Stack(error) => write!(f, "invalid scratch stack: {error:?}"),
        }
    }
}

impl std::error::Error for ScratchScopeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transaction(error) => Some(error),
            Self::Stack(_) => None,
        }
    }
}

// no wire: the error belongs to a local closure and can contain any Rust type.
// exhaustive: all scratch loan refusal and callback failure outcomes.
/// Why [`HostTransaction::scratch_stack`] or [`HostTransaction::scratch_slots`] did not commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScratchError<E> {
    /// The host could not open a transaction. The closure was not called.
    Unavailable,
    /// The initial stack did not pass the game's write validation. The closure was not called.
    Stack(StackWriteError),
    /// The closure returned `Ok`, but the host refused the commit and rolled back its writes.
    CommitRefused,
    /// The closure returned this error. Its writes were rolled back.
    Aborted(E),
}

impl<E: fmt::Display> fmt::Display for ScratchError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => TransactionScopeError::Unavailable.fmt(f),
            Self::CommitRefused => TransactionScopeError::CommitRefused.fmt(f),
            Self::Stack(error) => write!(f, "invalid scratch stack: {error:?}"),
            Self::Aborted(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ScratchError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable | Self::CommitRefused | Self::Stack(_) => None,
            Self::Aborted(error) => Some(error),
        }
    }
}

/// Combine the host's actual settlement with the callback's typed result.
pub(crate) fn finish_scope<T, E>(
    settled: Result<bool, TransactionScopeError>,
    outcome: Option<Result<T, E>>,
) -> Result<T, TransactionError<E>> {
    let committed = settled.map_err(TransactionError::from)?;
    match outcome {
        Some(Ok(value)) if committed => Ok(value),
        Some(Ok(_)) => Err(TransactionError::CommitRefused),
        Some(Err(error)) => Err(TransactionError::Aborted(error)),
        None => Err(TransactionError::Unavailable),
    }
}

/// Transactions are available only on the logical server. Both the host and the context are
/// lent to the callback; neither can escape it.
///
/// ```
/// use mod_api::{
///     BlockEntityError, BlockEntityRef, HostTransaction, ServerHost, TransactionError,
/// };
/// fn write(host: &mut dyn ServerHost, block: BlockEntityRef) {
///     let _: Result<(), TransactionError<BlockEntityError>> = host.transaction(|host, _tx| {
///         block.set_changed(host)?;
///         Ok(())
///     });
/// }
/// ```
/// ```compile_fail
/// use mod_api::{BlockEntityError, BlockEntityRef, Host, HostTransaction, TransactionError};
/// fn write(host: &mut dyn Host, block: BlockEntityRef) {
///     let _: Result<(), TransactionError<BlockEntityError>> = host.transaction(|host, _tx| {
///         block.set_changed(host)?;
///         Ok(())
///     });
/// }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, HostTransaction, TransactionError};
/// fn client(host: &mut dyn ClientHost) {
///     let _: Result<(), TransactionError<()>> = host.transaction(|_, _| Ok(()));
/// }
/// ```
/// ```compile_fail
/// use mod_api::{HostTransaction, ServerHost, TransactionContext, TransactionError};
/// fn escape(host: &mut dyn ServerHost) {
///     let _: Result<&TransactionContext, TransactionError<()>> = host.transaction(|_, tx| Ok(tx));
/// }
/// ```
/// ```compile_fail
/// use mod_api::{HostTransaction, ServerHost, TransactionError};
/// fn close(host: &mut dyn ServerHost) {
///     let _: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
///         host.transaction_close(tx, true);
///         Ok(())
///     });
/// }
/// ```
pub trait HostTransaction: ServerHost {
    /// Borrows the current transaction without opening or closing a scope. Returns `None`
    /// without calling `f` when no transaction is open. Writes settle with the existing frame,
    /// even if `f` unwinds and the enclosing action catches it.
    ///
    /// ```compile_fail
    /// use mod_api::{HostTransaction, ServerHost, TransactionContext};
    /// fn escape(host: &mut dyn ServerHost) {
    ///     let _: Option<&TransactionContext> = host.current_transaction(|_, tx| tx);
    /// }
    /// ```
    /// ```compile_fail
    /// use mod_api::{HostTransaction, ServerHost};
    /// fn close(host: &mut dyn ServerHost) {
    ///     host.current_transaction(|host, tx| host.transaction_close(tx, true));
    /// }
    /// ```
    fn current_transaction<T>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut dyn ServerHost, &'loan TransactionContext) -> T,
    ) -> Option<T> {
        let mut f = Some(f);
        let mut value = None;
        self.with_current_transaction_scope(&mut |host, tx| {
            if let Some(f) = f.take() {
                value = Some(f(host, tx));
            }
        })
        .ok()?;
        value
    }

    /// Runs `f` in a transaction, nested in the open one if any (`Transaction.openRoot` and
    /// `open`). `Ok` commits into the transaction around it or, at the outermost, for good,
    /// with the world told of each change once. `Err` aborts, restoring every storage it wrote.
    /// An unwind out of `f` aborts this transaction before reaching the caller; the transaction
    /// around it stays open.
    ///
    /// An error returned by `f` becomes [`TransactionError::Aborted`]. If a transaction cannot
    /// be opened, [`TransactionError::Unavailable`] means `f` did not run.
    /// [`TransactionError::CommitRefused`] means `f` returned `Ok` but its writes were rolled back.
    ///
    /// `f` gets a loan of `&mut dyn ServerHost`, even on a concrete receiver, and a
    /// [`TransactionContext`] to write in. Only the
    /// host's owning scope ends the transaction.
    ///
    /// Covered: block entities (item slots, saved compound, custom name and attachments), cauldrons,
    /// players' inventories, cursors, ender chests and persistent data, entities' inventories
    /// ([`ItemPlace::EntitySlot`](crate::ItemPlace::EntitySlot)) and item entities' stacks.
    /// Aborting restores a player's recorded storage even if the player dies, is in another
    /// level, or is no longer the acting player when the transaction closes.
    fn transaction<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut dyn ServerHost, &'loan TransactionContext) -> Result<T, E>,
    ) -> Result<T, TransactionError<E>> {
        let mut f = Some(f);
        let mut result = None;
        let settled = self.with_transaction_scope(&mut |host, tx| {
            let Some(f) = f.take() else { return false };
            let outcome = f(host, tx);
            let commit = outcome.is_ok();
            result = Some(outcome);
            commit
        });
        finish_scope(settled, result)
    }

    /// Runs `f` in a transaction with a lent single-slot scratch [`ItemPlace`] initialised from
    /// `stack` (`ItemAccess.forStack`), nested in the open transaction if any. Writes through the
    /// lent place never change its item, only its count and components, refusing what doesn't fit
    /// or is another item.
    ///
    /// On `Ok`, the transaction commits into its enclosing transaction (or for good at the root)
    /// and returns `(value, final_stack)`. On `Err` or unwind, the transaction aborts and the
    /// scratch slot is dropped without publishing. If no transaction can open or `stack` fails
    /// write validation, `f` does not run.
    fn scratch_stack<T, E>(
        &mut self,
        stack: ItemStack,
        f: impl for<'loan> FnOnce(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> Result<T, E>,
    ) -> Result<(T, Option<ItemStack>), ScratchError<E>> {
        let mut f = Some(f);
        let mut outcome = None;
        let slots = self
            .with_scratch_stack_scope(stack, &mut |host, tx, place| {
                let Some(f) = f.take() else { return false };
                let result = f(host, tx, place);
                let commit = result.is_ok();
                outcome = Some(result);
                commit
            })
            .map_err(|error| match error {
                ScratchScopeError::Transaction(TransactionScopeError::Unavailable) => {
                    ScratchError::Unavailable
                }
                ScratchScopeError::Transaction(TransactionScopeError::CommitRefused) => {
                    ScratchError::CommitRefused
                }
                ScratchScopeError::Stack(error) => ScratchError::Stack(error),
            })?;
        match (slots, outcome) {
            (Some(slots), Some(Ok(value))) => Ok((value, slots)),
            (_, Some(Err(error))) => Err(ScratchError::Aborted(error)),
            (None, Some(Ok(_))) => Err(ScratchError::CommitRefused),
            (_, None) => Err(ScratchError::Unavailable),
        }
    }

    /// Runs `f` in a transaction with a lent two-slot scratch [`ItemPlace`] whose first slot is
    /// initialised from `stack` (`DispenseFluidContainer`), nested in the open transaction if any.
    /// Reads and extracts use the first slot; inserts fill the first slot when it is empty or
    /// holds the same item and components, and what doesn't fit goes to the second slot.
    ///
    /// On `Ok`, the transaction commits into its enclosing transaction (or for good at the root)
    /// and returns `(value, (first, second))`. On `Err` or unwind, the transaction aborts and the
    /// scratch slots are dropped without publishing. If no transaction can open or `stack` fails
    /// write validation, `f` does not run.
    fn scratch_slots<T, E>(
        &mut self,
        stack: ItemStack,
        f: impl for<'loan> FnOnce(
            &'loan mut dyn ServerHost,
            &'loan TransactionContext,
            &'loan ItemPlace,
        ) -> Result<T, E>,
    ) -> Result<(T, (Option<ItemStack>, Option<ItemStack>)), ScratchError<E>> {
        let mut f = Some(f);
        let mut outcome = None;
        let slots = self
            .with_scratch_slots_scope(stack, &mut |host, tx, place| {
                let Some(f) = f.take() else { return false };
                let result = f(host, tx, place);
                let commit = result.is_ok();
                outcome = Some(result);
                commit
            })
            .map_err(|error| match error {
                ScratchScopeError::Transaction(TransactionScopeError::Unavailable) => {
                    ScratchError::Unavailable
                }
                ScratchScopeError::Transaction(TransactionScopeError::CommitRefused) => {
                    ScratchError::CommitRefused
                }
                ScratchScopeError::Stack(error) => ScratchError::Stack(error),
            })?;
        match (slots, outcome) {
            (Some(slots), Some(Ok(value))) => Ok((value, slots)),
            (_, Some(Err(error))) => Err(ScratchError::Aborted(error)),
            (None, Some(Ok(_))) => Err(ScratchError::CommitRefused),
            (_, None) => Err(ScratchError::Unavailable),
        }
    }
}

impl<H: ServerHost + ?Sized> HostTransaction for H {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeHost;
    use crate::{
        Host, ITEMS, Identifier, ItemResource, ItemStack, ResourceHandler,
        prelude::{BlockEntitiesHost, ItemsHost},
    };

    const AT: [i32; 3] = [0, 64, 0];

    fn fake_with_stone() -> (FakeHost, ItemResource) {
        let mut fake = FakeHost::new();
        let stone = Identifier::parse("minecraft:stone").unwrap();
        fake.registries.resolve_items([stone.clone()]).unwrap();
        let stone = ItemStack::new(fake.item_key(&stone).unwrap(), 1).unwrap();
        let resource = ItemResource::of(&stone);
        fake.level
            .block_entity(AT)
            .with_items(1)
            .items
            .as_mut()
            .unwrap()
            .slots[0] = Some(stone);
        (fake, resource)
    }

    #[test]
    fn borrowing_the_current_transaction_leaves_settlement_to_its_opener() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let (mut fake, resource) = fake_with_stone();
        let level = fake.level.level();
        let items = level.capability(&mut fake, &ITEMS, AT, None).unwrap();
        assert_eq!(
            fake.current_transaction::<()>(|_, _| panic!("no frame")),
            None
        );
        for commit in [false, true] {
            let result = fake.transaction(|host, outer| {
                let panicked = catch_unwind(AssertUnwindSafe(|| {
                    host.current_transaction::<()>(|_, _| panic!("borrow unwound"));
                }));
                assert!(panicked.is_err());
                assert!(host.transaction_matches(Some(outer)));
                let moved = host.current_transaction(|host, joined| {
                    assert_eq!(joined.depth(), outer.depth());
                    assert!(host.transaction_matches(Some(joined)));
                    assert!(host.transaction_matches(Some(outer)));
                    items.extract_at(host, joined, 0, &resource, 1)
                });
                assert_eq!(moved, Some(Ok(1)));
                assert!(host.transaction_matches(Some(outer)));
                assert_eq!(items.amount(host, 0), 0);
                commit.then_some(()).ok_or(())
            });
            assert_eq!(
                result,
                if commit {
                    Ok(())
                } else {
                    Err(TransactionError::Aborted(()))
                }
            );
            assert!(fake.transaction_matches(None));
            assert_eq!(items.amount(&mut fake, 0), if commit { 0 } else { 1 });
        }
    }

    struct NoTx;
    impl Host for NoTx {}
    crate::impl_host_areas!(NoTx);
    impl ServerHost for NoTx {
        fn backend_mut(&mut self) -> Option<crate::BackendMutRef<'_>> {
            panic!("an invalid context must be refused before backend lookup");
        }
    }
    crate::impl_server_host_areas!(NoTx);

    #[test]
    fn generated_writes_refuse_a_foreign_context_before_backend_lookup() {
        let (mut owner, _) = fake_with_stone();
        let level = owner.level.level();
        let block = owner.block_entity(level, AT).unwrap();
        let water = crate::FluidKey(1);
        owner
            .transaction(|_, tx| {
                assert_eq!(
                    level.pickup_fluid(&mut NoTx, AT, None, tx),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(
                    level.place_fluid(&mut NoTx, AT, water, None, false, tx),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(
                    level.play_fluid_effects(&mut NoTx, AT, water, false, None, tx),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(
                    block.set_item_slot(&mut NoTx, tx, 0, None),
                    Err(crate::BlockEntityItemError::InvalidContext)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
    }

    #[test]
    fn a_host_without_transactions_returns_unavailable_without_calling_closure() {
        let mut host = NoTx;
        assert!(host.transaction_matches(None));
        let outcome = host.transaction(|_, _| -> Result<(), ()> {
            panic!("closure must not run when transactions are unavailable");
        });
        assert_eq!(outcome, Err(TransactionError::Unavailable));
        let (_, resource) = fake_with_stone();
        let stack = resource.to_stack(1).unwrap();
        let stack_outcome = host.scratch_stack(stack.clone(), |_, _, _| -> Result<(), ()> {
            panic!("closure must not run when transactions are unavailable");
        });
        assert_eq!(stack_outcome, Err(ScratchError::Unavailable));
        let slots_outcome = host.scratch_slots(stack, |_, _, _| -> Result<(), ()> {
            panic!("closure must not run when transactions are unavailable");
        });
        assert_eq!(slots_outcome, Err(ScratchError::Unavailable));
    }

    #[test]
    fn a_live_context_from_another_host_is_refused_and_leaves_state_unchanged() {
        use crate::{
            ComponentId, EnergyHandler, ItemComponentEnergy, ItemComponentHandler, ItemPlace,
            STORAGE, StorageError,
        };
        let (mut owner, resource) = fake_with_stone();
        let mut other = owner.clone();
        let level = owner.level.level();
        let items = level.capability(&mut owner, &ITEMS, AT, None).unwrap();
        let storage = level.capability(&mut owner, &STORAGE, AT, None).unwrap();
        let place = ItemPlace::BlockEntity {
            level,
            pos: AT,
            side: None,
            slot: 0,
        };
        let component = ComponentId::new("test:charge").unwrap();
        let component_handler = ItemComponentHandler::new(place.clone(), component.clone());
        let energy = ItemComponentEnergy::new(place.clone(), component, 100, 100, 100);

        other
            .transaction(|other, foreign| {
                owner
                    .transaction(|host, own| {
                        assert_eq!((foreign.depth(), own.depth()), (0, 0));
                        assert!(!host.transaction_matches(Some(foreign)));
                        assert!(host.transaction_matches(Some(own)));
                        assert_eq!(
                            items.extract_at(host, foreign, 0, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            items.insert_at(host, foreign, 0, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            items.set_max_stack_size(host, foreign, 32),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            items.extract(host, foreign, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            storage.insert(host, foreign, &resource, 1),
                            Err(StorageError::InvalidContext)
                        );
                        assert_eq!(
                            storage.extract(host, foreign, &resource, 1),
                            Err(StorageError::InvalidContext)
                        );
                        assert_eq!(
                            place.insert(host, foreign, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            place.extract(host, foreign, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            place.exchange(host, foreign, &resource, 1),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            component_handler.write(host, foreign, None),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            energy.insert(host, foreign, 10),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(
                            energy.extract(host, foreign, 10),
                            Err(TransactionError::InvalidContext)
                        );
                        assert_eq!(items.amount(host, 0), 1);
                        assert_eq!(items.extract_at(host, own, 0, &resource, 1), Ok(1));
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                assert!(other.transaction_matches(Some(foreign)));
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(items.amount(&mut owner, 0), 0);
        assert_eq!(items.amount(&mut other, 0), 1);
        assert_eq!(owner.journal.closed(), [(1, true)]);
        assert_eq!(other.journal.closed(), [(1, true)]);
    }

    #[test]
    fn an_ancestor_or_closed_context_is_refused_in_a_new_scope() {
        let (mut fake, resource) = fake_with_stone();
        let level = fake.level.level();
        let items = level.capability(&mut fake, &ITEMS, AT, None).unwrap();

        let closed = fake
            .transaction(|host, outer| {
                host.transaction(|host, inner| {
                    assert_eq!((outer.depth(), inner.depth()), (0, 1));
                    assert_eq!(
                        items.extract(host, outer, &resource, 1),
                        Err(TransactionError::InvalidContext)
                    );
                    Ok::<_, ()>(())
                })
                .unwrap();
                Ok::<_, ()>(outer.frame)
            })
            .unwrap();
        closed.lend(|stale| {
            assert_eq!(
                items.extract(&mut fake, stale, &resource, 1),
                Err(TransactionError::InvalidContext)
            );
            fake.transaction(|host, own| {
                assert_eq!(own.depth(), stale.depth());
                assert_eq!(
                    items.extract(host, stale, &resource, 1),
                    Err(TransactionError::InvalidContext)
                );
                assert_eq!(items.amount(host, 0), 1);
                Ok::<_, ()>(())
            })
            .unwrap();
        });
    }

    #[test]
    fn a_transaction_commits_on_ok_and_aborts_on_err_and_nests() {
        let mut fake = FakeHost::new();
        let host: &mut dyn ServerHost = &mut fake;
        let outer = host.transaction(|host, _| {
            let inner = host.transaction(|_, _| Err::<(), _>("refused"));
            assert_eq!(inner, Err(TransactionError::Aborted("refused")));
            let committed = host.transaction(|_, _| Ok::<_, ()>(7)).unwrap();
            Ok::<_, ()>(committed)
        });
        assert_eq!(outer, Ok(7));
        assert_eq!(fake.journal.closed(), [(2, false), (2, true), (1, true)]);
        assert_eq!(fake.journal.depth(), 0);
    }

    #[test]
    fn a_panic_aborts_exactly_the_transaction_it_leaves() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let mut fake = FakeHost::new();
        let host: &mut dyn ServerHost = &mut fake;
        // A child's panic caught inside the root: the child aborts, the root still commits.
        let root = host.transaction(|host, _| {
            let child = catch_unwind(AssertUnwindSafe(|| {
                host.transaction(|_, _| -> Result<(), ()> { panic!("a broken handler") })
            }));
            assert!(child.is_err());
            Ok::<_, ()>(())
        });
        assert_eq!(root, Ok(()));
        // A root's panic caught outside it: the root aborts before the catch sees anything.
        let host: &mut dyn ServerHost = &mut fake;
        let caught = catch_unwind(AssertUnwindSafe(|| {
            host.transaction(|_, _| -> Result<(), ()> { panic!("a broken handler") })
        }));
        assert!(caught.is_err());
        assert_eq!(fake.journal.closed(), [(2, false), (1, true), (1, false)]);
        assert_eq!(fake.journal.depth(), 0);
    }
}
