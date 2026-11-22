//! Owned and borrowed item values. Registry-dependent validation belongs to the game's write boundary.
use crate::ItemKey;
use component_edits::{ComponentEdit, ItemComponentEdits};
use std::num::NonZeroU32;
use std::ops::Range;
use std::sync::Arc;
use sunrise_core::nbt::{Compound, JavaString, Tag};

/// A typed descriptor for one component in an item's component patch.
/// Implementations own the conversion between their value and Minecraft NBT.
pub trait Component {
    /// The value returned by this descriptor.
    type Value;
    /// Canonical component name, including its namespace.
    fn id(&self) -> &str;
    /// Decode a component value, refusing an incompatible NBT representation.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// Encode a component value.
    fn encode(&self, value: Self::Value) -> Tag;
}

/// An owned component patch, independent of the engine's component representation.
#[derive(sunrise_mod_api_macros::Wire, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Components {
    patch: Compound,
}

impl Components {
    /// Borrow the raw component patch at an NBT boundary.
    pub fn as_nbt(&self) -> &Compound {
        &self.patch
    }

    pub(crate) fn into_nbt(self) -> Compound {
        self.patch
    }
}
impl From<Compound> for Components {
    fn from(patch: Compound) -> Self {
        Self { patch }
    }
}
impl From<&Components> for Compound {
    fn from(value: &Components) -> Self {
        value.patch.clone()
    }
}

/// A nonempty, owned item value. Maxima are checked when the game accepts a write,
/// not by this record: item definitions and effective component overrides belong to the game.
/// The component patch is shared copy-on-write: `Clone` counts a reference instead of
/// copying it, the first write through a shared stack copies the patch once, and equality
/// of two stacks that still share it is a pointer comparison. An absent patch allocates nothing.
///
/// An untouched mutable stack hook argument retains its engine value without
/// write validation, even when the engine value exceeds effective maxima. Equality
/// with the snapshot passed to the listener determines whether it changed; a changed
/// argument is validated in full before committing any hook outputs.
/// A refused hook output retains its captured engine value. The clicked-slot bridge also
/// stages the slot, cursor and cancellation flags together. Independent host effects,
/// including storage writes, commands and messages, are not undone by output validation.
// no wire: decoding rejects zero counts through new and validates component patches through set_components; damage is applied through set_damage.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemStack {
    item: ItemKey,
    count: NonZeroU32,
    damage: Option<u32>,
    components: Option<Arc<Components>>,
}

type StackMetadata = (ItemKey, NonZeroU32, Option<u32>);
type StackProjector<'a> = dyn Fn(StackMetadata) -> Option<ItemStack> + 'a;
type SlotMetadata<'a> = dyn Fn(u32) -> Option<StackMetadata> + 'a;
type SlotProjector<'a> = dyn Fn(u32, StackMetadata) -> Option<ItemStack> + 'a;

/// A read-only loan of a nonempty stack. Reading its scalars does not copy the stack or
/// encode its components. Use [`Self::project`] to retain an owned value or read its patch.
// no wire: borrows a call-scoped stack or projection callable.
#[non_exhaustive]
pub struct ItemStackView<'a> {
    metadata: StackMetadata,
    projection: StackProjection<'a>,
}

enum StackProjection<'a> {
    Borrowed(&'a ItemStack),
    Resident(&'a StackProjector<'a>),
    Slot(u32, &'a SlotProjector<'a>),
    PreparedDrop(
        usize,
        &'a dyn crate::block::prepared_drops::PreparedBlockDropsAccess,
    ),
}

impl<'a> ItemStackView<'a> {
    /// Lend an existing API value, including one synthesized for the current call.
    pub fn from_stack(stack: &'a ItemStack) -> Self {
        Self {
            metadata: stack.read_metadata(),
            projection: StackProjection::Borrowed(stack),
        }
    }

    /// Lend the resident stack's checked `(item, count, damage override)` without encoding it.
    /// The projector receives the view's scalars when an owned stack is requested.
    // hidden: the engine lends resident storage using metadata it has checked.
    #[doc(hidden)]
    pub fn resident(metadata: StackMetadata, project: &'a StackProjector<'a>) -> Self {
        Self {
            metadata,
            projection: StackProjection::Resident(project),
        }
    }

    /// Lend the same item and components with a different visible count.
    pub(crate) fn with_count(mut self, count: NonZeroU32) -> Self {
        self.metadata.1 = count;
        self
    }

    pub(crate) fn prepared_drop(
        index: usize,
        metadata: StackMetadata,
        access: &'a dyn crate::block::prepared_drops::PreparedBlockDropsAccess,
    ) -> Self {
        Self {
            metadata,
            projection: StackProjection::PreparedDrop(index, access),
        }
    }

    /// The process-local identity of the item.
    pub fn item(&self) -> ItemKey {
        self.metadata.0
    }

    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.metadata.1.get()
    }

    /// Stored damage, or zero when no numeric damage override is present.
    pub fn damage(&self) -> u32 {
        self.damage_component().unwrap_or(0)
    }

    /// The numeric damage override. `Some(0)` is explicit zero; `None` means the
    /// component is inherited or explicitly removed, not a numeric override.
    pub fn damage_component(&self) -> Option<u32> {
        self.metadata.2
    }

    /// Copy this stack into an owned API value, including its normalized component patch.
    /// Returns `None` if the source cannot be represented as an owned [`ItemStack`].
    pub fn project(&self) -> Option<ItemStack> {
        let mut stack = match self.projection {
            StackProjection::Borrowed(stack) => stack.clone(),
            StackProjection::Resident(project) => project(self.metadata)?,
            StackProjection::Slot(slot, project) => project(slot, self.metadata)?,
            StackProjection::PreparedDrop(index, access) => {
                access.project_stack(index, self.metadata)?
            }
        };
        stack.count = self.metadata.1;
        Some(stack)
    }
}

/// A scoped editor for the component overrides of a stack at an [`ItemPlace`](crate::ItemPlace).
/// [`ItemPlace::edit`](crate::ItemPlace::edit) lends this editor without lending the host.
/// [`get`](Self::get) and the damage getters see earlier edits in the same callback.
/// [`original`](Self::original) reads the pre-edit stack; project that view explicitly to
/// keep an owned copy. After the callback returns, the game validates the batch before
/// changing the resolved slot. A successful write is still subject to transaction rollback.
///
/// The editor cannot leave the callback:
///
/// ```compile_fail
/// use mod_api::{ItemComponentEditor, ItemPlace, ServerHost, Transaction};
/// fn keep_editor(place: &ItemPlace, host: &mut dyn ServerHost, tx: &Transaction) {
///     let mut saved: Option<&mut ItemComponentEditor<'_>> = None;
///     let _ = place.edit(host, tx, |editor| { saved = Some(editor); });
///     let _ = saved;
/// }
/// ```
///
/// The callback cannot use the host already borrowed by the edit:
///
/// ```compile_fail
/// use mod_api::{ItemPlace, ServerHost, Transaction};
/// fn reread(place: &ItemPlace, host: &mut dyn ServerHost, tx: &Transaction) {
///     let _ = place.edit(host, tx, |_| { let _ = place.with_stack(host, |s| s.count()); });
/// }
/// ```
///
/// Changing the item or count requires a slot write or transfer, not a component edit:
///
/// ```compile_fail
/// use mod_api::ItemComponentEditor;
/// fn shrink(editor: &mut ItemComponentEditor<'_>) {
///     let _ = editor.set_count(1);
/// }
/// ```
// no wire: the editor borrows the addressed stack for one callback.
// exhaustive: its private fields prevent construction outside the game bridge.
pub struct ItemComponentEditor<'a> {
    original: ItemStackView<'a>,
    read_override: &'a dyn Fn(&str) -> Option<Tag>,
    edits: ItemComponentEdits,
}

impl<'a> ItemComponentEditor<'a> {
    /// The process-local identity of the item.
    pub fn item(&self) -> ItemKey {
        self.original.item()
    }

    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.original.count()
    }

    /// Stored damage after staged edits, or zero when no numeric override is present.
    pub fn damage(&self) -> u32 {
        self.damage_component().unwrap_or(0)
    }

    /// The staged or original numeric damage override. `Some(0)` is explicit zero;
    /// `None` means no numeric override, whether inherited or explicitly removed.
    pub fn damage_component(&self) -> Option<u32> {
        match self.edits.damage {
            Some(ComponentEdit::Value(damage)) => Some(damage),
            Some(ComponentEdit::Remove | ComponentEdit::Inherit) => None,
            None => self.original.damage_component(),
        }
    }

    /// Read the stack before any edits. Call [`ItemStackView::project`] on this view to
    /// request an owned snapshot; that projection never includes staged changes.
    pub fn original(&self) -> &ItemStackView<'a> {
        &self.original
    }

    /// Read a staged or original override with its typed descriptor, without consulting
    /// item defaults or projecting the entire stack. An inherited or removed component
    /// has no ordinary override to decode.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        let id = component.id();
        if id == "minecraft:damage" {
            return component.decode(&unsigned_tag(self.damage_component()?));
        }
        match self.edits.components.get(id) {
            Some(ComponentEdit::Value(value)) => component.decode(value),
            Some(ComponentEdit::Remove | ComponentEdit::Inherit) => None,
            None => component.decode(&(self.read_override)(id)?),
        }
    }

    /// Stage a component value. Raw `minecraft:damage` uses vanilla's numeric-to-int
    /// codec; a negative result or nonnumeric value leaves earlier edits intact.
    pub fn set<C: Component>(&mut self, component: &C, value: C::Value) -> Result<(), StackError> {
        let id = component.id();
        let value = component.encode(value);
        if id == "minecraft:damage" {
            self.set_damage(damage_value(&value)?);
        } else if let Some(id) = id.strip_prefix('!') {
            if id == "minecraft:damage" {
                validate_damage_removal(&value)?;
                self.clear_damage();
            } else {
                self.edits
                    .components
                    .insert(id.into(), ComponentEdit::Remove);
            }
        } else {
            self.edits
                .components
                .insert(id.into(), ComponentEdit::Value(value));
        }
        Ok(())
    }

    /// Clear a component override so the stack inherits its item default.
    pub fn remove<C: Component>(&mut self, component: &C) {
        let id = component.id();
        if id == "minecraft:damage" {
            self.edits.damage = Some(ComponentEdit::Inherit);
        } else {
            self.edits
                .components
                .insert(id.into(), ComponentEdit::Inherit);
        }
    }

    /// Stage an explicit numeric damage override. The game validates that damage fits `0..=i32::MAX`.
    pub fn set_damage(&mut self, damage: u32) {
        self.edits.damage = Some(ComponentEdit::Value(damage));
    }

    /// Stage explicit removal of the damage component, including an inherited default.
    pub fn clear_damage(&mut self) {
        self.edits.damage = Some(ComponentEdit::Remove);
    }

    /// Merge values and explicit `!id` removals into the staged edits, leaving other
    /// components alone. Use full component keys: the write does not add a namespace.
    /// A removal wins over a value in the same patch, including for damage.
    /// Raw `minecraft:damage` uses the same codec as [Self::set]; invalid damage
    /// refuses the entire patch without disturbing earlier edits.
    pub fn apply_components(&mut self, patch: &Compound) -> Result<(), StackError> {
        visit_component_patch(patch, |edit| match edit {
            PatchEdit::Component(id, edit) => {
                self.edits
                    .components
                    .insert(id.into(), edit.map(Tag::clone));
            }
            PatchEdit::Damage(edit) => self.edits.damage = Some(edit),
        })
    }

    /// Stage the source's own value or explicit removal, or inherit when it has neither.
    /// An id that is not a namespaced component id changes nothing.
    pub fn copy_component_from(&mut self, other: &ItemStack, id: &str) {
        if id == "minecraft:damage" {
            self.edits.damage = Some(other.damage_override());
        } else if crate::ComponentId::new(id).is_some() {
            self.edits
                .components
                .insert(id.into(), other.component_override(id).map(Tag::clone));
        }
    }
}

pub(crate) mod component_edits {
    use super::{ItemComponentEditor, ItemStack, ItemStackView, Tag};
    use std::collections::BTreeMap;

    /// One staged operation on an override; removal and inheritance are distinct.
    // exhaustive: the engine applies these three distinct override operations exhaustively.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum ComponentEdit<T> {
        /// Replace the override with this value.
        Value(T),
        /// Explicitly remove the component, even if the item supplies a default.
        Remove,
        /// Erase the override and let the item's default apply.
        Inherit,
    }

    impl<T> ComponentEdit<T> {
        pub(super) fn map<U>(self, f: impl FnOnce(T) -> U) -> ComponentEdit<U> {
            match self {
                Self::Value(value) => ComponentEdit::Value(f(value)),
                Self::Remove => ComponentEdit::Remove,
                Self::Inherit => ComponentEdit::Inherit,
            }
        }
    }

    /// A callback's normalized writes: one operation per bare component name, with damage inline.
    // exhaustive: the editor alone normalizes the named operations and damage into this private game input.
    pub struct ItemComponentEdits {
        pub(super) components: BTreeMap<String, ComponentEdit<Tag>>,
        pub(super) damage: Option<ComponentEdit<u32>>,
    }

    impl ItemComponentEdits {
        /// The damage operation, or `None` when the editor left damage alone.
        pub fn damage(&self) -> Option<ComponentEdit<u32>> {
            self.damage
        }

        /// Read one staged ordinary operation by its bare component name.
        pub fn component(&self, id: &str) -> Option<&ComponentEdit<Tag>> {
            self.components.get(id)
        }

        /// Borrow the ordinary operations for admission.
        pub fn components(&self) -> impl Iterator<Item = (&str, &ComponentEdit<Tag>)> {
            self.components.iter().map(|(id, edit)| (id.as_str(), edit))
        }

        /// Remove unchanged operations, including damage, before validating a write.
        pub fn retain(
            &mut self,
            mut component: impl FnMut(&str, &ComponentEdit<Tag>) -> bool,
            damage: impl FnOnce(ComponentEdit<u32>) -> bool,
        ) {
            self.components.retain(|id, edit| component(id, edit));
            self.damage = self.damage.filter(|&edit| damage(edit));
        }

        /// Borrow the names to apply, including damage only when the batch changes it.
        pub fn names(&self) -> impl Iterator<Item = &str> + Clone {
            self.components
                .keys()
                .map(String::as_str)
                .chain(self.damage.map(|_| "minecraft:damage"))
        }

        /// Apply the batch to an API-owned value, as used for exchange and by the test host.
        pub(crate) fn apply_to(self, stack: &mut ItemStack) {
            for (id, edit) in self.components {
                stack.apply_component_edit(&id, edit);
            }
            if let Some(edit) = self.damage {
                stack.apply_damage_edit(edit);
            }
        }
    }

    /// Lend the resolved stack to an edit callback and take its staged changes. The override
    /// reader returns one original ordinary value at its bare id; damage comes from the view.
    /// The game owns validation and publication of the returned batch after this scope ends.
    pub fn edit_item_components<'a>(
        view: ItemStackView<'a>,
        read_override: &'a dyn Fn(&str) -> Option<Tag>,
        edit: impl FnOnce(&mut ItemComponentEditor<'a>),
    ) -> ItemComponentEdits {
        let mut editor = ItemComponentEditor {
            original: view,
            read_override,
            edits: ItemComponentEdits {
                components: BTreeMap::new(),
                damage: None,
            },
        };
        edit(&mut editor);
        editor.edits
    }
}

pub(crate) fn read_stack_view<R>(
    read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    lend: impl FnOnce(&mut dyn for<'a> FnMut(ItemStackView<'a>)) -> Option<()>,
) -> Option<R> {
    let mut read = Some(read);
    let mut result = None;
    lend(&mut |view| {
        if let Some(read) = read.take() {
            result = Some(read(view));
        }
    })?;
    result
}

/// A read-only loan of numbered inventory slots, in their source's numbering.
/// Construction reads no slots. Looking up or iterating slots does not project their stacks.
// no wire: borrows the storage's call-scoped readers.
#[non_exhaustive]
pub struct InventoryView<'a> {
    slots: Range<u32>,
    metadata: &'a SlotMetadata<'a>,
    project: &'a SlotProjector<'a>,
}

impl<'a> InventoryView<'a> {
    /// Lend the given slot range. `metadata` reads only the addressed slot and returns
    /// `(item, count, damage override)` for a readable stack. `project` receives the
    /// original index and those checked scalars; it runs only on explicit projection.
    // hidden: the engine binds its inventory storage to the numbered slot readers.
    #[doc(hidden)]
    pub fn new(
        slots: Range<u32>,
        metadata: &'a SlotMetadata<'a>,
        project: &'a SlotProjector<'a>,
    ) -> Self {
        Self {
            slots,
            metadata,
            project,
        }
    }

    /// Read one slot. Out-of-range, empty and unreadable slots return `None`.
    pub fn get(&self, slot: u32) -> Option<ItemStackView<'a>> {
        if !self.slots.contains(&slot) {
            return None;
        }
        self.stack(slot)
    }

    /// Visit readable nonempty slots lazily, in ascending order, with their original indices.
    /// Empty and unreadable slots are skipped, not renumbered.
    pub fn iter(&self) -> impl Iterator<Item = (u32, ItemStackView<'a>)> + '_ {
        self.slots
            .clone()
            .filter_map(move |slot| self.stack(slot).map(|stack| (slot, stack)))
    }

    fn stack(&self, slot: u32) -> Option<ItemStackView<'a>> {
        Some(ItemStackView {
            metadata: (self.metadata)(slot)?,
            projection: StackProjection::Slot(slot, self.project),
        })
    }
}

/// A stack operation could not represent the requested count or component patch.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackError {
    /// Empty stacks are represented by `None`, not by a zero-count item.
    Empty,
    /// Adding items would overflow the record's count.
    CountOverflow,
    /// Damage must be a nonnegative integer representable by u32.
    InvalidDamage,
    /// The supplied value for a component is not admitted by its codec.
    InvalidComponent {
        /// The component whose value was refused.
        id: crate::ComponentId,
    },
}
impl std::fmt::Display for StackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("an item stack must be nonempty"),
            Self::CountOverflow => f.write_str("item stack count overflow"),
            Self::InvalidDamage => f.write_str("invalid item damage"),
            Self::InvalidComponent { id } => {
                write!(f, "invalid value for component {}", id.as_str())
            }
        }
    }
}
impl std::error::Error for StackError {}

/// Why the game refused to accept an owned stack at a write boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackWriteError {
    /// A text field failed component conversion before this write.
    Text(crate::text::TextFormError),
    /// The host has no game backend for this operation.
    Unavailable,
    /// The process-local key is not present in this game's item registry.
    UnknownItem,
    /// The count is above the effective stack limit.
    CountExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// A maximum is malformed or out of range (`max_stack_size` outside `1..=99`, `max_damage` outside `1..=i32::MAX`).
    InvalidMaximum,
    /// An engine result contained a zero-count stack instead of an empty slot.
    EmptyEngineStack,
    /// A stack's component patch was malformed or carried an out-of-range damage value.
    InvalidComponents {
        /// The record-level normalization refusal.
        reason: StackError,
    },
    /// A decoded `minecraft:consumable` or `minecraft:death_protection` component
    /// refused its opaque effect payload. `minecraft:use_remainder` and
    /// `minecraft:sulfur_cube_content` defer effect decoding until materialization.
    InvalidEffectPayload,
    /// A stack carries both maximum durability and a stack limit above one.
    StackableDamageable,
}

impl From<crate::text::TextFormError> for StackWriteError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for StackWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why an [`ItemPlace`](crate::ItemPlace) could not run the item's crafted-by processing.
///
/// This is a local refusal returned by [`ItemPlace::on_crafted_by`](crate::ItemPlace::on_crafted_by),
/// not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::CraftedByError>();
/// ```
// no wire: local refusal returned by ItemPlace::on_crafted_by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CraftedByError {
    /// The place is not a menu slot or the carried stack, or it no longer holds a stack.
    NotHeld,
    /// The host has no server game backend, or the transaction or place is no longer usable.
    Unavailable,
}

impl crate::NoBackend for CraftedByError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The two nonempty portions of a consuming split. Either portion can be absent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StackSplit {
    /// Up to the requested number of items.
    pub taken: Option<ItemStack>,
    /// Items left after taking that portion.
    pub remainder: Option<ItemStack>,
}

impl ItemStack {
    /// Construct a nonempty stack. Registry maxima are validated by the game on writes.
    pub fn new(item: ItemKey, count: u32) -> Option<Self> {
        Some(Self::from_scalars(item, NonZeroU32::new(count)?, None))
    }
    /// Construct a single item without a component patch.
    pub fn of(item: ItemKey) -> Self {
        Self::from_scalars(item, NonZeroU32::MIN, None)
    }
    /// Start a projection from checked scalar metadata before attaching any patch.
    // hidden: the engine starts an owned projection from checked nonempty metadata.
    #[doc(hidden)]
    pub fn from_scalars(item: ItemKey, count: NonZeroU32, damage: Option<u32>) -> Self {
        Self {
            item,
            count,
            damage,
            components: None,
        }
    }
    pub(crate) fn read_metadata(&self) -> StackMetadata {
        (self.item, self.count, self.damage)
    }
    /// The process-local identity of this item.
    pub fn item(&self) -> ItemKey {
        self.item
    }
    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.count.get()
    }
    /// Change the count without consulting the game. A refusal leaves the value unchanged.
    pub fn set_count(&mut self, count: u32) -> Result<(), StackError> {
        self.count = NonZeroU32::new(count).ok_or(StackError::Empty)?;
        Ok(())
    }
    /// Stored damage, or zero when no numeric damage override is present.
    /// A plain stack inherits its item's default. Some(0) is an explicit present zero;
    /// [`Self::clear_damage`] records removal of the default component instead.
    pub fn damage(&self) -> u32 {
        self.damage.unwrap_or(0)
    }
    /// The numeric damage override: None inherits the prototype unless the patch
    /// contains an explicit removal; Some(0) is a present zero-damage component.
    pub fn damage_component(&self) -> Option<u32> {
        self.damage
    }
    /// Set damage. The game validates that damage fits `0..=i32::MAX` when accepting this value.
    pub fn set_damage(&mut self, damage: u32) {
        self.damage = Some(damage);
        self.remove_patch_entries(&[JavaString::from("!minecraft:damage")]);
    }
    /// Remove the effective damage component, including an inherited default.
    /// The patch stores a real NBT removal, never a second numeric damage value.
    pub fn clear_damage(&mut self) {
        self.remove_component_value("minecraft:damage", true);
    }
    /// Whether a nonzero numeric damage override is stored on the stack.
    pub fn is_damaged(&self) -> bool {
        self.damage.is_some_and(|damage| damage != 0)
    }
    /// Add items. Overflow leaves the stack unchanged; registry maxima are checked on writes.
    pub fn grow(&mut self, count: u32) -> Result<(), StackError> {
        let count = self
            .count()
            .checked_add(count)
            .ok_or(StackError::CountOverflow)?;
        self.set_count(count)
    }
    /// Remove items, consuming the value so removing everything needs no empty sentinel.
    pub fn shrink(mut self, count: u32) -> Option<Self> {
        self.count = NonZeroU32::new(self.count().saturating_sub(count))?;
        Some(self)
    }
    /// Take up to `count` items. A full split moves the patch without cloning it.
    pub fn split(mut self, count: u32) -> StackSplit {
        if count == 0 {
            return StackSplit {
                taken: None,
                remainder: Some(self),
            };
        }
        if count >= self.count() {
            return StackSplit {
                taken: Some(self),
                remainder: None,
            };
        }
        let mut taken = self.clone();
        taken.count = NonZeroU32::new(count).expect("partial split is nonzero");
        self.count = NonZeroU32::new(self.count() - count).expect("partial split leaves items");
        StackSplit {
            taken: Some(taken),
            remainder: Some(self),
        }
    }
    /// Compare item identities, independent of count and components.
    pub fn same_item(&self, other: &Self) -> bool {
        self.item == other.item
    }
    /// Compare component patches and damage, independent of count and item identity.
    pub fn same_components(&self, other: &Self) -> bool {
        self.damage == other.damage && self.components == other.components
    }
    /// Whether these values have the same item and component patch.
    /// Capacity is a registry-dependent write-boundary check, not a promise of this predicate.
    pub fn is_stackable_with(&self, other: &Self) -> bool {
        self.same_item(other) && self.same_components(other)
    }
    /// Read a component with its typed descriptor.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        if component.id() == "minecraft:damage" {
            return self
                .damage
                .and_then(|damage| component.decode(&unsigned_tag(damage)));
        }
        component.decode(self.component_value(component.id())?)
    }
    pub(crate) fn component_value(&self, id: &str) -> Option<&Tag> {
        self.components()?.patch.get(&JavaString::from(id))
    }
    /// Replace a component value. Only stacks with a patch allocate component storage.
    /// Raw `minecraft:damage` uses vanilla's numeric-to-int codec and rejects negative
    /// results and nonnumeric values.
    pub fn set<C: Component>(&mut self, component: &C, value: C::Value) -> Result<(), StackError> {
        self.set_component_value(component.id(), component.encode(value))
    }
    pub(crate) fn set_component_value(&mut self, id: &str, value: Tag) -> Result<(), StackError> {
        if id == "minecraft:damage" {
            self.set_damage(damage_value(&value)?);
            return Ok(());
        }
        if id == "!minecraft:damage" {
            validate_damage_removal(&value)?;
            self.clear_damage();
            return Ok(());
        }
        self.put_component_value(id, value);
        Ok(())
    }
    fn put_component_value(&mut self, id: &str, value: Tag) {
        let patch = &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
        patch.remove(&JavaString::from(format!("!{id}").as_str()));
        patch.insert(JavaString::from(id), value);
    }
    fn apply_component_edit(&mut self, id: &str, edit: ComponentEdit<Tag>) {
        match edit {
            ComponentEdit::Value(value) => self.put_component_value(id, value),
            ComponentEdit::Remove => self.remove_component_value(id, true),
            ComponentEdit::Inherit => self.remove_component_value(id, false),
        }
    }
    fn apply_damage_edit(&mut self, edit: ComponentEdit<u32>) {
        match edit {
            ComponentEdit::Value(damage) => self.set_damage(damage),
            ComponentEdit::Remove => self.clear_damage(),
            ComponentEdit::Inherit => self.remove_component_value("minecraft:damage", false),
        }
    }
    /// Remove an override, releasing the patch allocation when its last entry is removed.
    pub fn remove<C: Component>(&mut self, component: &C) {
        self.remove_component_value(component.id(), false);
    }
    pub(crate) fn remove_component_value(&mut self, id: &str, removal: bool) {
        if id == "minecraft:damage" {
            self.damage = None;
        }
        let removed = JavaString::from(format!("!{id}").as_str());
        if removal {
            let patch =
                &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
            patch.remove(&JavaString::from(id));
            patch.insert(removed, Tag::Compound(Compound::new()));
            return;
        }
        self.remove_patch_entries(&[JavaString::from(id), removed]);
    }
    /// Remove entries from an existing patch. A shared patch is copied only when one of
    /// the keys is present; the patch is released when its last entry is removed.
    fn remove_patch_entries(&mut self, keys: &[JavaString]) {
        let Some(components) = &mut self.components else {
            return;
        };
        if !keys.iter().any(|key| components.patch.contains_key(key)) {
            return;
        }
        let patch = &mut Arc::make_mut(components).patch;
        for key in keys {
            patch.remove(key);
        }
        if patch.is_empty() {
            self.components = None;
        }
    }
    /// The component patch, if this stack has overrides.
    pub fn components(&self) -> Option<&Components> {
        self.components.as_deref()
    }
    /// `ItemStack.applyComponents`: merge a patch without discarding other overrides.
    /// Use full component keys, as in [Self::set_components]. Incoming values replace
    /// removals and incoming removals replace values. If both are supplied for one
    /// component, removal wins, including for damage. Raw `minecraft:damage` uses
    /// the same codec as [Self::set]; invalid damage leaves the stack unchanged.
    pub fn apply_components(&mut self, patch: &Compound) -> Result<(), StackError> {
        visit_component_patch(patch, |edit| match edit {
            PatchEdit::Component(id, edit) => self.apply_component_edit(id, edit.map(Tag::clone)),
            PatchEdit::Damage(edit) => self.apply_damage_edit(edit),
        })
    }
    /// Copy this component's own value or explicit removal from `other`, without
    /// consulting item defaults. If `other` inherits it, clear this stack's override too.
    /// An `id` that is not a namespaced component id (`Damage`, or a removal marker such as
    /// `!minecraft:damage`) changes nothing.
    pub fn copy_component_from(&mut self, other: &Self, id: &str) {
        if id == "minecraft:damage" {
            self.apply_damage_edit(other.damage_override());
        } else if crate::ComponentId::new(id).is_some() {
            self.apply_component_edit(id, other.component_override(id).map(Tag::clone));
        }
    }
    fn component_override(&self, id: &str) -> ComponentEdit<&Tag> {
        let Some(patch) = self.components().map(Components::as_nbt) else {
            return ComponentEdit::Inherit;
        };
        if patch.contains_key(&JavaString::from(format!("!{id}").as_str())) {
            ComponentEdit::Remove
        } else {
            patch
                .get(&JavaString::from(id))
                .map_or(ComponentEdit::Inherit, ComponentEdit::Value)
        }
    }
    fn damage_override(&self) -> ComponentEdit<u32> {
        if let Some(damage) = self.damage {
            ComponentEdit::Value(damage)
        } else if self.component_value("!minecraft:damage").is_some() {
            ComponentEdit::Remove
        } else {
            ComponentEdit::Inherit
        }
    }
    /// Replace the component patch at an NBT conversion boundary. Use full component keys:
    /// a bare `damage` entry stays opaque rather than becoming a damage override.
    /// `minecraft:damage` uses the same codec as [Self::set] and replaces prior damage.
    /// `!minecraft:damage` clears any prior or incoming damage value. Invalid damage
    /// refuses the entire replacement and leaves the stack unchanged.
    pub fn set_components(&mut self, mut patch: Compound) -> Result<(), StackError> {
        let damage = patch_damage(&patch)?;
        let removed = patch.contains_key(&JavaString::from("!minecraft:damage"));
        patch.remove(&JavaString::from("minecraft:damage"));
        self.damage = if removed {
            None
        } else {
            damage.or(self.damage)
        };
        self.components = (!patch.is_empty()).then(|| Arc::new(Components::from(patch)));
        Ok(())
    }
    /// Decode the item NBT escape using this host's load-resolved item ids.
    /// Unknown ids, zero counts and invalid damage return None. Raw damage uses the same
    /// codec as [Self::set]. Effective maxima remain the game's write-boundary responsibility.
    pub fn from_nbt(host: &dyn crate::Host, value: &Compound) -> Option<Self> {
        let Tag::String(id) = value.get(&JavaString::from("id"))? else {
            return None;
        };
        let id = crate::Identifier::parse(&id.to_string_lossy()).ok()?;
        let item = host.item_key(&id)?;
        let count = match value.get(&JavaString::from("count")) {
            None => 1,
            Some(value) => u32::try_from(match value {
                Tag::Byte(value) => i64::from(*value),
                Tag::Short(value) => i64::from(*value),
                Tag::Int(value) => i64::from(*value),
                Tag::Long(value) => *value,
                _ => return None,
            })
            .ok()?,
        };
        let mut stack = Self::new(item, count)?;
        match value.get(&JavaString::from("components")) {
            Some(Tag::Compound(patch)) => stack.set_components(patch.clone()).ok()?,
            None => {}
            _ => return None,
        }
        Some(stack)
    }
    /// Encode the item NBT escape using this host's canonical id, never a process-local key.
    /// Returns None when id metadata is unavailable. Only components present on the
    /// stack are written; effective defaults and maxima belong to the items host area.
    pub fn to_nbt(&self, host: &dyn crate::Host) -> Option<Compound> {
        let id = self.item.id(host)?;
        let mut value = Compound::new();
        value.insert(JavaString::from("id"), Tag::String(id.into()));
        value.insert(JavaString::from("count"), unsigned_tag(self.count()));
        let mut patch = self
            .components()
            .map(|patch| patch.as_nbt().clone())
            .unwrap_or_default();
        if let Some(damage) = self.damage {
            patch.insert(JavaString::from("minecraft:damage"), unsigned_tag(damage));
        }
        if !patch.is_empty() {
            value.insert(JavaString::from("components"), Tag::Compound(patch));
        }
        Some(value)
    }
}

pub(crate) fn unsigned_tag(value: u32) -> Tag {
    i32::try_from(value).map_or_else(|_| Tag::Long(i64::from(value)), Tag::Int)
}

fn damage_value(value: &Tag) -> Result<u32, StackError> {
    sunrise_core::nbt::numeric_int(value)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(StackError::InvalidDamage)
}

enum PatchEdit<'a> {
    Component(&'a str, ComponentEdit<&'a Tag>),
    Damage(ComponentEdit<u32>),
}

/// Walk normalized edits in patch order once damage has been admitted. Removals
/// win over values. Numeric damage lives outside the patch and is set at the end.
fn visit_component_patch(
    patch: &Compound,
    mut visit: impl FnMut(PatchEdit<'_>),
) -> Result<(), StackError> {
    let damage = patch_damage(patch)?;
    for (id, value) in patch {
        let id = id.to_string_lossy();
        match id.as_str() {
            "minecraft:damage" => (),
            "!minecraft:damage" => visit(PatchEdit::Damage(ComponentEdit::Remove)),
            id => {
                if let Some(id) = id.strip_prefix('!') {
                    visit(PatchEdit::Component(id, ComponentEdit::Remove));
                } else if !patch.contains_key(&JavaString::from(format!("!{id}").as_str())) {
                    visit(PatchEdit::Component(id, ComponentEdit::Value(value)));
                }
            }
        }
    }
    if let Some(value) = damage {
        visit(PatchEdit::Damage(ComponentEdit::Value(value)));
    }
    Ok(())
}

fn validate_damage_removal(value: &Tag) -> Result<(), StackError> {
    if matches!(value, Tag::Compound(value) if value.is_empty()) {
        Ok(())
    } else {
        Err(StackError::InvalidDamage)
    }
}

/// Validate the raw damage entries before either a replacement or a merge mutates a stack.
/// Explicit removal (`!minecraft:damage`) wins over a numeric `minecraft:damage` entry.
fn patch_damage(patch: &Compound) -> Result<Option<u32>, StackError> {
    let damage = patch.get(&JavaString::from("minecraft:damage"));
    let removal = patch.get(&JavaString::from("!minecraft:damage"));
    if let Some(removal) = removal {
        validate_damage_removal(removal)?;
    }
    let damage = damage.map(damage_value).transpose()?;
    if removal.is_some() {
        return Ok(None);
    }
    Ok(damage)
}
#[cfg(test)]
mod tests {
    use super::component_edits::edit_item_components;
    use super::*;

    include!("item_stack_damage_tests.rs");
    include!("item_stack_component_tests.rs");

    struct Number;
    impl Component for Number {
        type Value = i32;
        fn id(&self) -> &str {
            "test:number"
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(value) => Some(*value),
                _ => None,
            }
        }
        fn encode(&self, value: i32) -> Tag {
            Tag::Int(value)
        }
    }

    #[test]
    fn scoped_component_editor_reads_staged_overrides_without_projecting() {
        struct Field(&'static str);
        impl Component for Field {
            type Value = i32;
            fn id(&self) -> &str {
                self.0
            }
            fn decode(&self, value: &Tag) -> Option<i32> {
                Number.decode(value)
            }
            fn encode(&self, value: i32) -> Tag {
                Number.encode(value)
            }
        }

        let removed = Field("test:removed");
        let inherited = Field("test:inherited");
        let damage = Field("minecraft:damage");
        let mut before = ItemStack::new(ItemKey(3), 16).unwrap();
        before.set(&Number, 12).unwrap();
        before.set(&removed, 23).unwrap();
        before.set(&inherited, 34).unwrap();
        before.set(&Other, 45).unwrap();
        before.set_damage(7);
        let source = ItemStack::of(ItemKey(5));
        let reads = std::cell::RefCell::new(Vec::new());
        let read_override = |id: &str| {
            reads.borrow_mut().push(id.to_owned());
            before.component_value(id).cloned()
        };
        let view = ItemStackView::resident(before.read_metadata(), &|_| {
            panic!("component reads must not project the stack")
        });
        let edits = edit_item_components(view, &read_override, |stack| {
            assert_eq!(stack.item(), ItemKey(3));
            assert_eq!(stack.count(), 16);
            assert_eq!(stack.get(&Number), Some(12));
            assert_eq!(stack.get(&damage), Some(7));

            stack.remove(&Number);
            assert_eq!(stack.get(&Number), None);
            stack.set(&Number, 41).unwrap();
            assert_eq!(stack.get(&Number), Some(41));
            stack
                .apply_components(
                    &[
                        ("test:removed".into(), Tag::Int(42)),
                        ("!test:removed".into(), Tag::Compound(Compound::new())),
                    ]
                    .into(),
                )
                .unwrap();
            assert_eq!(stack.get(&removed), None);
            stack.copy_component_from(&source, "test:inherited");
            assert_eq!(stack.get(&inherited), None);
            assert_eq!(stack.get(&Other), Some(45));

            stack.set_damage(0);
            assert_eq!(stack.damage_component(), Some(0));
            assert_eq!(stack.get(&damage), Some(0));
            assert_eq!(stack.original().damage(), 7);
            let invalid = [
                ("test:number".into(), Tag::Int(99)),
                ("minecraft:damage".into(), Tag::Int(-1)),
            ]
            .into();
            assert_eq!(
                stack.apply_components(&invalid),
                Err(StackError::InvalidDamage)
            );
            assert_eq!(stack.get(&Number), Some(41));
            assert_eq!(stack.damage_component(), Some(0));

            stack.clear_damage();
            assert_eq!(stack.damage(), 0);
            assert_eq!(stack.damage_component(), None);
            assert_eq!(stack.get(&damage), None);
        });
        assert_eq!(*reads.borrow(), ["test:number", "test:other"]);

        edits.apply_to(&mut before);
        let expected: Compound = [
            ("test:number".into(), Tag::Int(41)),
            ("test:other".into(), Tag::Int(45)),
            ("!test:removed".into(), Tag::Compound(Compound::new())),
            ("!minecraft:damage".into(), Tag::Compound(Compound::new())),
        ]
        .into();
        assert_eq!(before.components().unwrap().as_nbt(), &expected);
        assert_eq!(before.damage_component(), None);
    }

    #[test]
    fn inventory_views_read_only_requested_slots_and_project_on_demand() {
        use std::cell::RefCell;

        let reads = RefCell::new(Vec::new());
        let projected = RefCell::new(Vec::new());
        let metadata = |slot| {
            reads.borrow_mut().push(slot);
            match slot {
                5 => Some((ItemKey(2), NonZeroU32::MIN, None)),
                8 => Some((ItemKey(3), NonZeroU32::new(7).unwrap(), Some(0))),
                _ => None,
            }
        };
        let project = |slot, (item, count, damage)| {
            projected.borrow_mut().push(slot);
            Some(ItemStack::from_scalars(item, count, damage))
        };
        let view = InventoryView::new(4..9, &metadata, &project);
        assert!(reads.borrow().is_empty());

        let stack = view.get(8).unwrap();
        assert_eq!(stack.item(), ItemKey(3));
        assert_eq!(stack.count(), 7);
        assert_eq!(stack.damage(), 0);
        assert_eq!(stack.damage_component(), Some(0));
        assert!(view.get(9).is_none());
        assert_eq!(*reads.borrow(), [8]);
        assert!(projected.borrow().is_empty());
        reads.borrow_mut().clear();

        let mut slots = view.iter();
        let (index, first) = slots.next().unwrap();
        assert_eq!(index, 5);
        assert_eq!(first.item(), ItemKey(2));
        assert_eq!(first.damage_component(), None);
        assert_eq!(*reads.borrow(), [4, 5]);
        assert!(projected.borrow().is_empty());

        let owned = stack.project().unwrap();
        assert_eq!(owned.item(), ItemKey(3));
        assert_eq!(owned.count(), 7);
        assert_eq!(owned.damage_component(), Some(0));
        assert_eq!(*projected.borrow(), [8]);
        assert_eq!(*reads.borrow(), [4, 5]);
        assert_eq!(slots.next().map(|(slot, _)| slot), Some(8));
        assert!(slots.next().is_none());
        assert_eq!(*reads.borrow(), [4, 5, 6, 7, 8]);
        assert_eq!(*projected.borrow(), [8]);
    }

    #[test]
    fn stack_errors_keep_their_wire_tags_and_append_the_component_id() {
        use crate::wire::{Reader, Writer};

        for (error, bytes) in [
            (StackError::Empty, &b"\x00\x00\x00\x00"[..]),
            (StackError::CountOverflow, &b"\x01\x00\x00\x00"[..]),
            (StackError::InvalidDamage, &b"\x02\x00\x00\x00"[..]),
            (
                StackError::InvalidComponent {
                    id: crate::ComponentId::new("minecraft:villager_food").unwrap(),
                },
                &b"\x03\x00\x00\x00\x17\x00\x00\x00minecraft:villager_food"[..],
            ),
        ] {
            let mut out = Writer::new();
            out.put(&error);
            assert_eq!(out.bytes(), bytes);
            assert_eq!(Reader::decode::<StackError>(bytes).unwrap(), error);
        }
    }

    #[test]
    fn count_validation_is_atomic_and_registry_independent() {
        let key = ItemKey(0);
        assert!(ItemStack::new(key, 0).is_none());
        let mut stack = ItemStack::new(key, u32::MAX).unwrap();
        assert_eq!(stack.grow(1), Err(StackError::CountOverflow));
        assert_eq!(stack.set_count(0), Err(StackError::Empty));
        assert_eq!(stack.count(), u32::MAX);
        stack.set_damage(u32::MAX);
        assert_eq!(stack.damage(), u32::MAX);
    }

    #[test]
    fn splits_conserve_items_without_an_empty_sentinel() {
        for n in [0, 1, 4, 5, u32::MAX] {
            let mut stack = ItemStack::new(ItemKey(0), 4).unwrap();
            stack.set(&Number, 17).unwrap();
            let split = stack.split(n);
            assert_eq!(split.taken.as_ref().map_or(0, ItemStack::count), n.min(4));
            assert_eq!(
                split.remainder.as_ref().map_or(0, ItemStack::count),
                4 - n.min(4)
            );
            for portion in [split.taken, split.remainder].into_iter().flatten() {
                assert_eq!(portion.get(&Number), Some(17));
            }
        }
        assert!(ItemStack::of(ItemKey(0)).shrink(1).is_none());
    }

    #[test]
    fn patches_are_lazy_and_comparisons_ignore_counts() {
        let mut stack = ItemStack::of(ItemKey(0));
        assert!(stack.components().is_none());
        stack.set(&Number, 19).unwrap();
        let mut other = stack.clone();
        other.grow(3).unwrap();
        assert!(stack.is_stackable_with(&other));
        other.remove(&Number);
        assert!(!stack.same_components(&other));
        assert!(other.components().is_none());
        stack.remove(&Number);
        assert!(stack.is_stackable_with(&other));
    }
}
