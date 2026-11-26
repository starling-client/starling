//! Sequential loans of a resident block entity and its server Host.
use crate::{
    BlockEntityItemError, BlockEntityRef, BlockState, Compound, ItemKey, ItemStack, JavaString,
    LevelRef, ServerHost, Tag, TransactionContext, TransactionError, TransactionScopeError,
    TransactionUnavailable,
};

/// A resident block entity lent by a server callback. Its storage stays in the level.
/// Keep a [`BlockEntityRef`] from [`Self::handle`] if you need to find it after the call.
// exhaustive: only the engine constructs a view.
// no wire: an exclusive loan, not a payload.
pub struct BlockEntityView<'a> {
    subject: BlockEntityRef,
    state: BlockState,
    data: &'a mut dyn ResidentBlockEntity,
}

impl<'a> BlockEntityView<'a> {
    // hidden: the engine builds the view after checking the subject and borrowing its resident row.
    #[doc(hidden)]
    pub fn new(
        subject: BlockEntityRef,
        state: BlockState,
        data: &'a mut dyn ResidentBlockEntity,
    ) -> Self {
        Self {
            subject,
            state,
            data,
        }
    }

    /// The entity's block position.
    pub fn position(&self) -> [i32; 3] {
        self.subject.position
    }
    /// The level the entity resides in.
    pub fn level(&self) -> LevelRef {
        self.subject.level
    }
    /// The block's live [`BlockState`] at the time of this [`BlockEntityScope::with_view`]
    /// loan, updating when the block changes in place to another valid state for this block
    /// entity type.
    pub fn state(&self) -> BlockState {
        self.state
    }
    /// The entity's immutable type id.
    pub fn id(&self) -> &str {
        self.data.id()
    }
    /// A generation-bound handle to keep after this callback.
    pub fn handle(&self) -> BlockEntityRef {
        self.subject
    }

    /// The live, transient integer slots its menu reads.
    pub fn data_slots(&self) -> &[i32] {
        self.data.data_slots()
    }
    /// Set one existing transient menu data slot without automatically marking the chunk
    /// unsaved. Returns `false` for an absent slot or an equal value.
    pub fn set_data_slot(&mut self, slot: u32, value: i32) -> bool {
        self.data.set_data_slot(slot, value)
    }
    /// Set and, if necessary, resize the transient menu data, without a block update
    /// packet, 256-slot cap, or automatic chunk dirtying. Returns `true` if the values
    /// changed and `false` on a no-op.
    pub fn set_data_slots(&mut self, data: &[i32]) -> bool {
        self.data.set_data_slots(data)
    }

    /// The resident saved compound. Modeled item slots are separate. On a slotless
    /// holder, `Items` is mod-owned NBT, even if it looks like an inventory list.
    pub fn nbt(&self) -> &Compound {
        self.data.nbt()
    }
    /// Set or remove a mod-owned saved field, without reloading the block entity or
    /// automatically marking the chunk unsaved. Returns `false` for an equal field,
    /// excessive nesting, engine-owned identity or shape fields, or `"Items"` when the
    /// block entity's block has engine-managed container slots (`item_slots() > 0`).
    /// For slotless block entities (`item_slots() == 0`), `"Items"` writes the NBT field
    /// directly and clears any disk-decoded item slots. Chunk saves preserve the
    /// field as written, including list order.
    pub fn set_nbt(&mut self, key: JavaString, value: Option<Tag>) -> bool {
        self.data.set_nbt(key, value)
    }

    /// The declared internal slot count, independent of its external automation policy.
    pub fn item_slots(&self) -> usize {
        self.data.item_slots()
    }
    /// Borrow a modeled stack. `None` for an empty, opaque or undeclared slot.
    pub fn item_slot(&self, slot: u32) -> Option<BlockEntityStackView<'_>> {
        let (item, count) = self.data.item_slot(slot)?;
        Some(BlockEntityStackView {
            data: &*self.data,
            slot,
            item,
            count,
        })
    }
    /// Replace a declared internal slot. An explicit write claims any opaque saved row at
    /// that index. Rejected stacks leave storage alone; untouched slots are not normalized.
    /// Call [`BlockEntityRef::set_changed`] explicitly to mark the chunk unsaved.
    pub fn set_item_slot(
        &mut self,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        self.data.set_item_slot(slot, stack)
    }

    /// Queue one-shot fields for the next block-entity sync to its current viewers.
    /// An empty compound clears the queued fields. Nothing saves or sends them from this call.
    /// Aborting a transaction restores the batch that was waiting before its first edit.
    pub fn set_client_update_fields(&mut self, fields: Compound) {
        self.data.set_client_update_fields(fields);
    }
}

/// Read a resident stack without copying its components. Copy explicitly to retain or edit it.
// exhaustive: the borrowed backend cannot be constructed by a mod.
// no wire: borrowed stack storage.
pub struct BlockEntityStackView<'a> {
    data: &'a dyn ResidentBlockEntity,
    slot: u32,
    item: &'a str,
    count: u32,
}

impl BlockEntityStackView<'_> {
    /// The stack's registered item, or `None` if not visible in the mod item registry.
    pub fn item(&self) -> Option<ItemKey> {
        crate::content::registry_keys::item_key(self.item)
    }
    /// The resident item count.
    pub fn count(&self) -> u32 {
        self.count
    }
    /// Materialize the complete stack, including components. `None` if it cannot be
    /// represented in the mod API.
    pub fn to_owned(&self) -> Option<ItemStack> {
        self.data.copy_item_slot(self.slot)
    }
}

/// Sequential access to the resident block entity and the server Host.
///
/// ```compile_fail,E0501
/// use mod_vocab::BlockEntityTickEvent;
/// fn tick(event: &mut BlockEntityTickEvent<'_>) {
///     event.subject.with_view(|machine| {
///         let _host = event.subject.host();
///         let _slots = machine.data_slots();
///     });
/// }
/// ```
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the sequential loans cannot escape their call.
pub struct BlockEntityScope<'a>(&'a mut dyn BlockEntityAccess);

impl<'a> BlockEntityScope<'a> {
    // hidden: the engine supplies the loan backend for this dispatch.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn BlockEntityAccess) -> Self {
        Self(access)
    }

    // hidden: dispatch shortens the scope to the same lifetime as the other event fields.
    #[doc(hidden)]
    pub fn reborrow(&mut self) -> BlockEntityScope<'_> {
        BlockEntityScope(&mut *self.0)
    }

    /// Borrows an already-open innermost transaction without opening or closing a frame.
    /// Returns `None` without calling `f` if there is no current transaction to borrow.
    pub fn current_transaction<T>(
        &mut self,
        f: impl for<'loan> FnOnce(&'loan mut BlockEntityScope<'_>, &'loan TransactionContext) -> T,
    ) -> Option<T> {
        let mut f = Some(f);
        let mut value = None;
        self.0
            .with_current_transaction_scope(&mut |access, tx| {
                if let Some(f) = f.take() {
                    value = Some(f(&mut BlockEntityScope::new(access), tx));
                }
            })
            .ok()?;
        value
    }

    /// Lend the original subject for `f`. Once `f` returns, you can use the Host
    /// and then ask for the view again. A blockless menu or a removal or replacement
    /// of the original block entity returns `None` without calling `f`. A ticker's
    /// view tracks in-place states valid for the same entity type; a menu only lends
    /// the block state admitted for its callback.
    ///
    /// Successful writes are visible to subsequent loans and listeners. Menu data uses
    /// normal menu sync. Saved storage edits in the menu hooks mark the chunk unsaved;
    /// a ticker or on-load callback calls [`BlockEntityRef::set_changed`] explicitly.
    pub fn with_view<R>(&mut self, f: impl FnOnce(BlockEntityView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the Host for other objects or world operations. No view is live while
    /// this borrow is held. Reborrow the view afterwards to re-check the subject.
    pub fn host(&mut self) -> &mut dyn ServerHost {
        self.0.host()
    }

    /// Run an atomic operation spanning the subject and other world storage. The
    /// callback can alternate between view and Host, under one explicit transaction.
    /// A closure error becomes [`TransactionError::Aborted`] and, like an unwind, rolls back
    /// its journaled edits. Returns [`TransactionError::Unavailable`] without calling `f` if a
    /// transaction cannot be opened. Ordinary loans join an already open transaction and never
    /// open a savepoint by themselves.
    pub fn transaction<T, E>(
        &mut self,
        f: impl for<'loan> FnOnce(
            &'loan mut BlockEntityScope<'_>,
            &'loan TransactionContext,
        ) -> Result<T, E>,
    ) -> Result<T, TransactionError<E>> {
        let mut f = Some(f);
        let mut result = None;
        let settled = self.0.with_transaction_scope(&mut |access, tx| {
            let Some(f) = f.take() else { return false };
            let mut scope = BlockEntityScope::new(access);
            let outcome = f(&mut scope, tx);
            let commit = outcome.is_ok();
            result = Some(outcome);
            commit
        });
        crate::item::transaction::finish_scope(settled, result)
    }
}

/// Engine backing for the sequential loans.
// hidden: the engine owns the alternating view and Host loans; listeners receive BlockEntityScope.
#[doc(hidden)]
pub trait BlockEntityAccess {
    fn with_view(&mut self, f: &mut dyn FnMut(BlockEntityView<'_>));
    fn host(&mut self) -> &mut dyn ServerHost;
    /// Lends the current scope without opening or settling a transaction.
    fn with_current_transaction_scope(
        &mut self,
        _callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn BlockEntityAccess,
            &'loan TransactionContext,
        ),
    ) -> Result<(), TransactionUnavailable> {
        Err(TransactionUnavailable)
    }

    fn with_transaction_scope(
        &mut self,
        _callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn BlockEntityAccess,
            &'loan TransactionContext,
        ) -> bool,
    ) -> Result<bool, TransactionScopeError> {
        Err(TransactionScopeError::Unavailable)
    }
}

/// Engine backing for a checked, journaled row loan.
// hidden: the engine owns row access and journaling; listeners use the checked BlockEntityView methods.
#[doc(hidden)]
pub trait ResidentBlockEntity {
    fn id(&self) -> &str;
    fn data_slots(&self) -> &[i32];
    fn set_data_slot(&mut self, slot: u32, value: i32) -> bool;
    fn set_data_slots(&mut self, data: &[i32]) -> bool;
    fn nbt(&self) -> &Compound;
    fn set_nbt(&mut self, key: JavaString, value: Option<Tag>) -> bool;
    fn item_slots(&self) -> usize;
    fn item_slot(&self, slot: u32) -> Option<(&str, u32)>;
    fn copy_item_slot(&self, slot: u32) -> Option<ItemStack>;
    fn set_item_slot(
        &mut self,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError>;
    fn set_client_update_fields(&mut self, fields: Compound);
}
