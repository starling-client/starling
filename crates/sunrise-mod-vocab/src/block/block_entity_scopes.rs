//! Scoped views and sequential host loans for block-entity lifecycle and sync hooks.
use crate::{
    BlockEntityRef, BlockEntityScope, BlockEntityTypeKey, BlockState, ClientHost, Compound,
    HostSide, ItemKey, ItemStack, LevelHost, LevelRef,
};

/// Read-only view of a resident mod block entity. Its storage stays in the level;
/// keep a [`BlockEntityRef`] from [`Self::handle`] if you need to find it after the call.
// exhaustive: only the engine constructs a view.
// no wire: a borrowed view, not a payload.
pub struct BlockEntityReadView<'a> {
    subject: BlockEntityRef,
    state: BlockState,
    data: BlockEntityDataView<'a>,
}

impl<'a> BlockEntityReadView<'a> {
    // hidden: the engine builds the view after checking the subject and borrowing its resident row.
    #[doc(hidden)]
    pub fn new(
        subject: BlockEntityRef,
        state: BlockState,
        data: &'a dyn ResidentBlockEntityReadOnly,
    ) -> Self {
        Self {
            subject,
            state,
            data: BlockEntityDataView::new(data),
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
    /// The entity's live block state.
    pub fn state(&self) -> BlockState {
        self.state
    }
    /// The entity's immutable type id.
    pub fn id(&self) -> &str {
        self.data.id()
    }
    /// The entity's resolved block-entity type key, if registered.
    pub fn kind(&self) -> Option<BlockEntityTypeKey> {
        self.data.kind()
    }
    /// A generation-bound handle to keep after this call.
    pub fn handle(&self) -> BlockEntityRef {
        self.subject
    }

    /// The live, transient integer slots its menu reads.
    pub fn data_slots(&self) -> &[i32] {
        self.data.data_slots()
    }
    /// The resident saved or synchronized compound. Modeled item slots are separate;
    /// `Items`, when present, holds opaque saved data that has not been decoded into item slots.
    pub fn nbt(&self) -> &Compound {
        self.data.nbt()
    }

    /// The declared internal slot count, independent of its external automation policy.
    pub fn item_slots(&self) -> usize {
        self.data.item_slots()
    }
    /// Borrow a modeled stack. `None` for an empty, opaque or undeclared slot.
    pub fn item_slot(&self, slot: u32) -> Option<BlockEntityReadStackView<'_>> {
        self.data.item_slot(slot)
    }
    /// Materialize one modeled stack, including components. `None` for an empty,
    /// opaque or undeclared slot, or one that cannot be represented in the mod API.
    pub fn copy_item_slot(&self, slot: u32) -> Option<ItemStack> {
        self.data.copy_item_slot(slot)
    }
}

/// Borrowed data of the actual mod block entity used by a render, sync or lifecycle callback.
/// It may be detached (for example, supplied by a screen scene or evicted with a chunk),
/// so the view does not imply that there is a live world handle to it. A mod lends its own
/// saved data as a view through [`DetachedBlockEntityData::view`].
// exhaustive: the engine or a DetachedBlockEntityData constructs a view.
// no wire: borrowed subject data is local to the callback.
#[derive(Clone, Copy)]
pub struct BlockEntityDataView<'a>(&'a dyn ResidentBlockEntityReadOnly);

/// A block entity's saved data that a mod holds itself, not in any level: the type id and the
/// compound, as a structure template or a moving contraption carries them (vanilla builds such a
/// block entity with `EntityBlock.newBlockEntity` and `BlockEntity.loadWithComponents` to draw
/// it). [`view`](Self::view) lends it to code that takes a [`BlockEntityDataView`], the type
/// the engine's block-entity callbacks receive.
///
/// The compound is read as it is: [`BlockEntityDataView::nbt`] is it, `Items` included, and
/// the view models no item slots or menu data slots (`item_slots` is 0, `data_slots` empty).
// exhaustive: its fields are private, so mods build it with new.
// no wire: a borrowed view of the mod's own data, not a payload.
#[derive(Clone, Copy)]
pub struct DetachedBlockEntityData<'a> {
    id: &'a str,
    nbt: &'a Compound,
}

impl<'a> DetachedBlockEntityData<'a> {
    /// Lend `nbt` as the saved data of a block entity of type `id` (`mymod:press`).
    pub fn new(id: &'a str, nbt: &'a Compound) -> Self {
        Self { id, nbt }
    }
    /// This data as a [`BlockEntityDataView`].
    pub fn view(&self) -> BlockEntityDataView<'_> {
        BlockEntityDataView(self)
    }
}

impl ResidentBlockEntityReadOnly for DetachedBlockEntityData<'_> {
    fn id(&self) -> &str {
        self.id
    }
    fn data_slots(&self) -> &[i32] {
        &[]
    }
    fn nbt(&self) -> &Compound {
        self.nbt
    }
    fn item_slots(&self) -> usize {
        0
    }
    fn item_slot(&self, _: u32) -> Option<(&str, u32)> {
        None
    }
    fn copy_item_slot(&self, _: u32) -> Option<ItemStack> {
        None
    }
}

impl<'a> BlockEntityDataView<'a> {
    // hidden: the engine lends the resident row or the actual detached data.
    #[doc(hidden)]
    pub fn new(data: &'a dyn ResidentBlockEntityReadOnly) -> Self {
        Self(data)
    }

    /// The entity's immutable type id.
    pub fn id(&self) -> &str {
        self.0.id()
    }
    /// The entity's resolved block-entity type key, if registered.
    pub fn kind(&self) -> Option<BlockEntityTypeKey> {
        crate::content::registry_keys::block_entity_type_key(self.id())
    }
    /// The transient integer slots written for menus.
    pub fn data_slots(&self) -> &[i32] {
        self.0.data_slots()
    }
    /// The saved or synchronized compound. Modeled item slots are separate.
    pub fn nbt(&self) -> &Compound {
        self.0.nbt()
    }
    /// The number of item slots exposed by this view, including empty slots.
    pub fn item_slots(&self) -> usize {
        self.0.item_slots()
    }
    /// Borrow one modeled stack without projecting its components.
    pub fn item_slot(&self, slot: u32) -> Option<BlockEntityReadStackView<'_>> {
        let (item, count) = self.0.item_slot(slot)?;
        Some(BlockEntityReadStackView {
            data: self.0,
            slot,
            item,
            count,
        })
    }
    /// Project one slot into a complete owned stack. A failure affects only this slot.
    pub fn copy_item_slot(&self, slot: u32) -> Option<ItemStack> {
        self.0.copy_item_slot(slot)
    }
}

/// Read a resident stack without copying its components. Copy explicitly to retain it.
// exhaustive: the borrowed backend cannot be constructed by a mod.
// no wire: borrowed stack storage.
pub struct BlockEntityReadStackView<'a> {
    data: &'a dyn ResidentBlockEntityReadOnly,
    slot: u32,
    item: &'a str,
    count: u32,
}

impl BlockEntityReadStackView<'_> {
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

/// Sequential access to a client-side resident block entity and the mutable client Host.
///
/// ```compile_fail,E0501
/// use mod_vocab::BlockEntityClientTickEvent;
/// fn tick(event: &mut BlockEntityClientTickEvent<'_>) {
///     event.subject.with_view(|machine| {
///         let _host = event.subject.host();
///         let _nbt = machine.nbt();
///     });
/// }
/// ```
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the sequential loans cannot escape their call.
pub struct BlockEntityClientScope<'a>(&'a mut dyn BlockEntityClientAccess);

impl<'a> BlockEntityClientScope<'a> {
    // hidden: the engine supplies the loan backend for this client dispatch.
    #[doc(hidden)]
    pub fn new(access: &'a mut dyn BlockEntityClientAccess) -> Self {
        Self(access)
    }

    // hidden: dispatch routes block-entity parts from the live subject without a separate Host.
    #[doc(hidden)]
    pub fn route_kind(&self) -> Option<&str> {
        self.0.route_kind()
    }

    /// Lend the client copy for `f`. Once `f` returns, you can use the Host
    /// and then ask for the view again. Each loan checks the client level,
    /// chunk residency and generation; removal or replacement returns `None`
    /// without running `f`.
    pub fn with_view<R>(&mut self, f: impl FnOnce(BlockEntityReadView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the mutable client Host (for particles, sounds and client queries).
    /// No view is live while this borrow is held; reborrow the view afterwards to re-check it.
    pub fn host(&mut self) -> &mut dyn ClientHost {
        self.0.host()
    }
}

/// Read-only access to a client-side resident block entity and the read-only client Host.
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the borrowed loans cannot escape their call.
#[derive(Clone, Copy)]
pub struct BlockEntityClientReadScope<'a>(&'a dyn BlockEntityClientReadAccess);

impl<'a> BlockEntityClientReadScope<'a> {
    // hidden: the engine supplies the read backend for this client notice.
    #[doc(hidden)]
    pub fn new(access: &'a dyn BlockEntityClientReadAccess) -> Self {
        Self(access)
    }

    // hidden: dispatch routes block-entity parts from the live subject without a separate Host.
    #[doc(hidden)]
    pub fn route_kind(&self) -> Option<&str> {
        self.0.route_kind()
    }

    /// Lend the client copy for `f`. Returns `None` if the copy is no longer resident.
    pub fn with_view<R>(&self, f: impl FnOnce(BlockEntityReadView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the read-only client Host.
    pub fn host(&self) -> &dyn ClientHost {
        self.0.host()
    }
}

/// Read-only access to a server-side resident block entity and the read-only level Host.
// exhaustive: the engine constructs this scope around a single dispatch.
// no wire: the borrowed loans cannot escape their call.
#[derive(Clone, Copy)]
pub struct BlockEntityLevelReadScope<'a>(&'a dyn BlockEntityLevelReadAccess);

impl<'a> BlockEntityLevelReadScope<'a> {
    // hidden: the engine supplies the read backend for this level notice.
    #[doc(hidden)]
    pub fn new(access: &'a dyn BlockEntityLevelReadAccess) -> Self {
        Self(access)
    }

    /// Lend the resident block entity for `f`. Returns `None` if it is no longer resident.
    pub fn with_view<R>(&self, f: impl FnOnce(BlockEntityReadView<'_>) -> R) -> Option<R> {
        let mut f = Some(f);
        let mut result = None;
        self.0.with_view(&mut |view| {
            if let Some(f) = f.take() {
                result = Some(f(view));
            }
        });
        result
    }

    /// Borrow the read-only level Host.
    pub fn host(&self) -> &dyn LevelHost {
        self.0.host()
    }
}

/// Side-tagged sequential access to a resident block entity on a both-side hook
/// (`block_entity_on_load`).
///
/// ```compile_fail,E0501
/// use mod_vocab::{BlockEntityOnLoadEvent, BlockEntitySideScope};
/// fn on_load(event: &mut BlockEntityOnLoadEvent<'_>) {
///     if let BlockEntitySideScope::Server(scope) = &mut event.subject {
///         scope.with_view(|machine| {
///             let _host = scope.host();
///             let _nbt = machine.nbt();
///         });
///     }
/// }
/// ```
// exhaustive: Minecraft has only a server side and a client side.
// no wire: the sequential loans cannot escape their call.
pub enum BlockEntitySideScope<'a> {
    /// The logical server's mutable block-entity scope (view, `ServerHost` and `transaction`).
    Server(BlockEntityScope<'a>),
    /// The client's read-only block-entity scope (read view and mutable `ClientHost`).
    Client(BlockEntityClientScope<'a>),
}

impl BlockEntitySideScope<'_> {
    /// Borrow the logical side's Host while no view is live.
    pub fn host(&mut self) -> HostSide<'_> {
        match self {
            Self::Server(scope) => HostSide::Server(scope.host()),
            Self::Client(scope) => HostSide::Client(scope.host()),
        }
    }
}

/// Engine backing for a read-only mod block entity view.
// hidden: the engine owns row reads; listeners use BlockEntityReadView or BlockEntityDataView.
#[doc(hidden)]
pub trait ResidentBlockEntityReadOnly {
    fn id(&self) -> &str;
    fn data_slots(&self) -> &[i32];
    fn nbt(&self) -> &Compound;
    fn item_slots(&self) -> usize;
    fn item_slot(&self, slot: u32) -> Option<(&str, u32)>;
    fn copy_item_slot(&self, slot: u32) -> Option<ItemStack>;
}

/// Engine backing for sequential client block-entity view and mutable client Host loans.
// hidden: the engine owns the alternating view and Host loans; listeners receive BlockEntityClientScope.
#[doc(hidden)]
pub trait BlockEntityClientAccess {
    fn route_kind(&self) -> Option<&str>;
    fn with_view(&mut self, f: &mut dyn FnMut(BlockEntityReadView<'_>));
    fn host(&mut self) -> &mut dyn ClientHost;
}

/// Engine backing for read-only client block-entity view and Host loans.
// hidden: the engine owns the read loans; listeners receive BlockEntityClientReadScope.
#[doc(hidden)]
pub trait BlockEntityClientReadAccess {
    fn route_kind(&self) -> Option<&str>;
    fn with_view(&self, f: &mut dyn FnMut(BlockEntityReadView<'_>));
    fn host(&self) -> &dyn ClientHost;
}

/// Engine backing for read-only server level block-entity view and Host loans.
// hidden: the engine owns the read loans; listeners receive BlockEntityLevelReadScope.
#[doc(hidden)]
pub trait BlockEntityLevelReadAccess {
    fn with_view(&self, f: &mut dyn FnMut(BlockEntityReadView<'_>));
    fn host(&self) -> &dyn LevelHost;
}

#[cfg(test)]
mod detached_data_tests {
    use super::*;
    use sunrise_core::nbt::Tag;

    /// A mod's own saved data lent as a view reads back its id and compound, `Items` and all,
    /// with no modeled item or data slots.
    #[test]
    fn a_detached_compound_reads_as_a_block_entity_data_view() {
        let mut nbt = Compound::new();
        nbt.insert("Progress".into(), Tag::Int(7));
        nbt.insert("Items".into(), Tag::List(Vec::new()));
        let data = DetachedBlockEntityData::new("mymod:press", &nbt);
        let view = data.view();
        assert_eq!(view.id(), "mymod:press");
        assert!(std::ptr::eq(view.nbt(), &nbt));
        assert_eq!(view.nbt().get(&"Progress".into()), Some(&Tag::Int(7)));
        assert!(view.nbt().contains_key(&"Items".into()));
        assert_eq!(view.item_slots(), 0);
        assert!(view.item_slot(0).is_none() && view.copy_item_slot(0).is_none());
        assert!(view.data_slots().is_empty());
    }
}
