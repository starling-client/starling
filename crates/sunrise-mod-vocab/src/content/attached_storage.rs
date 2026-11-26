//! Stock storage a mod attaches to an entity, a player, a chunk or the level, as NeoForge 26.3
//! mods keep an `ItemStacksResourceHandler`, a `FluidStacksResourceHandler` or a
//! `SimpleEnergyHandler` in an attachment: handlers over an [`AttachmentKey`] the mod registers,
//! saved by its codec. Every write is an attachment write, so the game's transaction journal,
//! which keeps each holder's attachment map, puts it back on an abort.
use crate::content::registry_keys::{fluid_id, fluid_key, item_id, item_key_by_identifier};
use crate::{
    AttachmentCodec, AttachmentError, AttachmentKey, Components, Compound, EnergyHandler,
    FluidIdentity, FluidResource, FluidStack, Holder, Identifier, ItemResource, ItemStack,
    Overanswered, ResourceHandler, ServerHost, Tag, TransactionContext, TransactionError,
};
use std::{
    collections::BTreeMap,
    ops::{ControlFlow, Range},
};
use sunrise_core::nbt::JavaString;

/// The contents of an [`AttachedStacks`], keyed by slot or tank index. Missing indices are
/// empty; the handler's size is independent of this map.
// no wire: attachment contents are serialized only by the declared attachment codec.
pub type StackList<R> = BTreeMap<usize, (R, u32)>;

/// A resource an [`AttachedStacks`] holds: how much of it one index takes at most, given the
/// handler's own capacity. Items stack to their stack size; anything else to the capacity.
pub trait StackedResource: Clone + PartialEq + Send + Sync + 'static {
    /// The most of it one index of capacity `capacity` holds.
    fn limit(&self, host: &mut dyn ServerHost, capacity: u32) -> u32 {
        let _ = host;
        capacity
    }
}

impl StackedResource for ItemResource {
    fn limit(&self, host: &mut dyn ServerHost, capacity: u32) -> u32 {
        match self.value_id(&*host, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) => capacity.min(u32::try_from(size).unwrap_or(0)),
            _ => capacity,
        }
    }
}

impl StackedResource for FluidResource {}

/// Indices of a resource kept in an attachment of a holder (`StacksResourceHandler`): `size`
/// indices of up to `capacity` each, taking only what `valid` accepts. The attachment holds a
/// [`StackList`]; one emptied whole is removed.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct AttachedStacks<R: 'static> {
    attachment: AttachmentKey<StackList<R>>,
    holder: Holder,
    size: usize,
    capacity: u32,
    valid: fn(&R) -> bool,
}

/// Item slots in an attachment (`ItemStacksResourceHandler`); its codec is [`ITEM_STACKS`].
// no wire: an alias of AttachedStacks, a mod-side handler.
pub type AttachedItems = AttachedStacks<ItemResource>;
/// Fluid tanks in an attachment (`FluidStacksResourceHandler`); its codec is [`FLUID_STACKS`].
// no wire: an alias of AttachedStacks, a mod-side handler.
pub type AttachedFluids = AttachedStacks<FluidResource>;

enum StackWriteError {
    Read,
    Write(AttachmentError),
}

impl<R: StackedResource> AttachedStacks<R> {
    /// `size` indices of `attachment` on `holder`, each holding up to `capacity` of what `valid`
    /// accepts. Only occupied indices take space. A codec, if declared, may refuse a write it
    /// cannot represent.
    pub fn new(
        attachment: &AttachmentKey<StackList<R>>,
        holder: Holder,
        size: usize,
        capacity: u32,
        valid: fn(&R) -> bool,
    ) -> Self {
        Self {
            attachment: attachment.clone(),
            holder,
            size,
            capacity,
            valid,
        }
    }
    /// One index (`ItemStackResourceHandler`, a single tank).
    pub fn single(
        attachment: &AttachmentKey<StackList<R>>,
        holder: Holder,
        capacity: u32,
        valid: fn(&R) -> bool,
    ) -> Self {
        Self::new(attachment, holder, 1, capacity, valid)
    }
    /// Read just the part of the map the operation needs. A saved map decodes on its first read.
    fn read_stacks<T>(
        &self,
        host: &mut dyn ServerHost,
        read: impl FnOnce(&StackList<R>) -> T,
    ) -> Option<T> {
        self.attachment
            .get_existing(host, self.holder, |list| list.map(read))
            .ok()
            .flatten()
    }
    /// Project from the occupied slot without cloning the resource for count-only reads.
    fn with_entry<T>(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        read: impl FnOnce(&(R, u32)) -> T,
    ) -> Option<T> {
        if index >= self.size {
            return None;
        }
        self.read_stacks(host, |list| list.get(&index).map(read))
            .flatten()
    }
    /// Update just this index, preserving entries outside this view's logical size.
    fn write(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        now: Option<(R, u32)>,
    ) -> Result<(), StackWriteError> {
        let checked = self.attachment.live().map_err(StackWriteError::Write)?;
        let mut list = checked
            .get_existing(host.attachments(), self.holder, |list| list.cloned())
            .map_err(|_| StackWriteError::Read)?
            .unwrap_or_default();
        if let Some(held) = now.filter(|(_, amount)| *amount > 0) {
            list.insert(index, held);
        } else {
            list.remove(&index);
        }
        if list.is_empty() {
            checked.remove(host.attachments(), self.holder).map(|_| ())
        } else {
            checked.set(host.attachments(), self.holder, list)
        }
        .map_err(StackWriteError::Write)
    }

    /// A refusal to write the attachment can rule out the rest of this handler's indices.
    fn insert_index(
        &self,
        host: &mut dyn ServerHost,
        index: usize,
        resource: &R,
        amount: u32,
    ) -> ControlFlow<(), u32> {
        if !self.is_valid(host, index, resource) {
            return ControlFlow::Break(());
        }
        let room = self.capacity(host, index, Some(resource));
        let have = match self.with_entry(host, index, Clone::clone) {
            Some((held, _)) if held != *resource => return ControlFlow::Continue(0),
            Some((_, have)) => have,
            None => 0,
        };
        let went = amount.min(room.saturating_sub(have));
        if went == 0 {
            return ControlFlow::Continue(0);
        }
        match self.write(host, index, Some((resource.clone(), have + went))) {
            Ok(()) => ControlFlow::Continue(went),
            // A codec may refuse just this index, whether the slot is empty or holds a top-up
            // too large to encode. Only the stock codec's `Slot` (an NBT Int) rules out the
            // rest of the range: an index past `i32::MAX` has no suffix that can be written.
            Err(StackWriteError::Write(AttachmentError::Encode | AttachmentError::Decode)) => {
                ControlFlow::Continue(0)
            }
            Err(StackWriteError::Write(AttachmentError::StackEncoding { index: failed }))
                if failed == index && i32::try_from(index).is_ok() =>
            {
                ControlFlow::Continue(0)
            }
            // A failed read or unavailable holder affects the whole attachment, as does an
            // unencodable unchanged entry or a stock-codec `Slot` beyond its NBT Int.
            Err(_) => ControlFlow::Break(()),
        }
    }
}

impl<R: StackedResource> ResourceHandler<R> for AttachedStacks<R> {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.size
    }
    fn next_transfer_index(
        &self,
        host: &mut dyn ServerHost,
        indices: Range<usize>,
        inserting: Option<&R>,
    ) -> Option<usize> {
        let indices = indices.start..indices.end.min(self.size);
        if indices.is_empty() {
            return None;
        }
        if let Some(resource) = inserting {
            return ((self.valid)(resource) && resource.limit(host, self.capacity) > 0)
                .then_some(indices.start);
        }
        self.read_stacks(host, |list| {
            list.range(indices.clone()).next().map(|(&index, _)| index)
        })
        .flatten()
    }
    fn uniform_slots_end(&self, _: &mut dyn ServerHost, indices: Range<usize>) -> usize {
        indices.end.min(self.size)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<R> {
        self.with_entry(host, index, |(resource, _)| resource.clone())
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.with_entry(host, index, |(_, amount)| *amount)
            .unwrap_or(0)
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: Option<&R>) -> u32 {
        // The physical limit of an in-range index; capacity is not permission to insert
        // (`is_valid` is), and extraction does not look at it.
        if index < self.size {
            resource.map_or(self.capacity, |resource| {
                resource.limit(host, self.capacity)
            })
        } else {
            0
        }
    }
    fn is_valid(&self, _: &mut dyn ServerHost, index: usize, resource: &R) -> bool {
        index < self.size && (self.valid)(resource)
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
        Ok(self
            .insert_index(host, index, resource, amount)
            .continue_value()
            .unwrap_or(0))
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
        if indices.is_empty() {
            return Ok((0, indices.end));
        }
        Ok(
            match self.insert_index(host, indices.start, resource, amount) {
                ControlFlow::Continue(went) => (went, indices.start + 1),
                ControlFlow::Break(()) => (0, indices.end),
            },
        )
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
        let Some((held, have)) = self
            .with_entry(host, index, Clone::clone)
            .filter(|(held, _)| held == resource)
        else {
            return Ok(0);
        };
        Ok(match amount.min(have) {
            0 => 0,
            came if self.write(host, index, Some((held, have - came))).is_ok() => came,
            _ => 0,
        })
    }
}

/// Energy kept in an int attachment of a holder (`SimpleEnergyHandler`): up to `capacity`,
/// moving at most `max_insert` in and `max_extract` out per call.
// no wire: a mod-side handler, invoked where it was built; never crosses the boundary.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct AttachedEnergy {
    attachment: AttachmentKey<u32>,
    holder: Holder,
    capacity: u32,
    max_insert: u32,
    max_extract: u32,
}

impl AttachedEnergy {
    /// `attachment` on `holder`, holding up to `capacity`, at most `max_insert` in and
    /// `max_extract` out per call.
    pub fn new(
        attachment: &AttachmentKey<u32>,
        holder: Holder,
        capacity: u32,
        max_insert: u32,
        max_extract: u32,
    ) -> Self {
        Self {
            attachment: attachment.clone(),
            holder,
            capacity,
            max_insert,
            max_extract,
        }
    }
    /// Sets the energy: `moved` when the write went through, else 0.
    fn write(&self, host: &mut dyn ServerHost, energy: u32, moved: u32) -> u32 {
        match self.attachment.set(host, self.holder, energy) {
            Ok(()) => moved,
            Err(_) => 0,
        }
    }
}

impl EnergyHandler for AttachedEnergy {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        self.attachment
            .get_existing(host, self.holder, |value| value.copied().unwrap_or(0))
            .unwrap_or(0)
    }
    fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
        self.capacity
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let held = self.amount(host);
        Ok(
            match amount
                .min(self.max_insert)
                .min(self.capacity.saturating_sub(held))
            {
                0 => 0,
                went => self.write(host, held + went, went),
            },
        )
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        if !host.transaction_matches(Some(tx)) {
            return Err(TransactionError::InvalidContext);
        }
        let held = self.amount(host);
        Ok(match amount.min(self.max_extract).min(held) {
            0 => 0,
            came => self.write(host, held - came, came),
        })
    }
}

/// The codec of an [`AttachedItems`] attachment: `{stacks: [{Slot, id, count, components}]}`,
/// only occupied slots, as in vanilla's `ContainerHelper.saveAllItems`. In this format `Slot`
/// and `count` are NBT `Int`s. An occupied index or amount above `i32::MAX` fails encoding
/// with [`AttachmentError::StackEncoding`] rather than wrapping; a negative saved index fails
/// decoding.
pub const ITEM_STACKS: AttachmentCodec<StackList<ItemResource>> = AttachmentCodec {
    encode: |list| {
        encode_stacks(list, |item, amount| {
            let stack = item.to_stack(amount)?;
            let mut saved = Compound::new();
            saved.insert("id".into(), Tag::String(item_id(stack.item())?.into()));
            saved.insert("count".into(), Tag::Int(i32::try_from(amount).ok()?));
            let mut patch = stack
                .components()
                .map(|patch| patch.as_nbt().clone())
                .unwrap_or_default();
            if let Some(damage) = stack.damage_component() {
                patch.insert("minecraft:damage".into(), Tag::Int(damage as i32));
            }
            if !patch.is_empty() {
                saved.insert("components".into(), Tag::Compound(patch));
            }
            Some(saved)
        })
    },
    decode: |saved| {
        decode_stacks(saved, |entry| {
            let Some(Tag::String(id)) = entry.get(&"id".into()) else {
                return None;
            };
            let item = item_key_by_identifier(&Identifier::parse(&id.to_string_lossy()).ok()?)?;
            let Some(&Tag::Int(count)) = entry.get(&"count".into()) else {
                return None;
            };
            let count = u32::try_from(count).ok()?;
            let mut stack = ItemStack::new(item, 1)?;
            if let Some(Tag::Compound(patch)) = entry.get(&"components".into()) {
                stack.set_components(patch.clone()).ok()?;
            }
            Some((ItemResource::of(&stack), count))
        })
    },
};

/// The codec of an [`AttachedFluids`] attachment: `{stacks: [{Slot, id, amount, components}]}`.
/// As with [`ITEM_STACKS`], only occupied indices are saved. `Slot` and `amount` are NBT
/// `Int`s: an occupied index or amount above `i32::MAX` fails with
/// [`AttachmentError::StackEncoding`], and a negative saved index fails decoding.
pub const FLUID_STACKS: AttachmentCodec<StackList<FluidResource>> = AttachmentCodec {
    encode: |list| {
        encode_stacks(list, |fluid, amount| {
            let stack = fluid.to_stack(amount);
            let mut saved = Compound::new();
            saved.insert("id".into(), Tag::String(fluid_id(stack.key()?)?.into()));
            saved.insert("amount".into(), Tag::Int(i32::try_from(amount).ok()?));
            if stack.has_components()
                && let Some(components) = &stack.components
            {
                saved.insert(
                    "components".into(),
                    Tag::Compound(components.as_nbt().clone()),
                );
            }
            Some(saved)
        })
    },
    decode: |saved| {
        decode_stacks(saved, |entry| {
            let Some(Tag::String(id)) = entry.get(&"id".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = entry.get(&"amount".into()) else {
                return None;
            };
            let components = match entry.get(&"components".into()) {
                Some(Tag::Compound(patch)) => Some(Box::new(Components::from(patch.clone()))),
                _ => None,
            };
            let stack = FluidStack {
                fluid: FluidIdentity::Resolved(fluid_key(&id.to_string_lossy())?),
                amount: 1,
                components,
            };
            Some((FluidResource::of(&stack), u32::try_from(amount).ok()?))
        })
    },
};

/// `list`'s held indices, each saved by `entry` with its `Slot`; a resource `entry` can't save
/// (an id the registry lacks) fails the encode, so nothing is lost silently.
fn encode_stacks<R>(
    list: &StackList<R>,
    entry: impl Fn(&R, u32) -> Option<Compound>,
) -> Result<Compound, AttachmentError> {
    let mut stacks = Vec::new();
    for (&index, (resource, amount)) in list {
        let slot = i32::try_from(index).map_err(|_| AttachmentError::StackEncoding { index })?;
        let mut saved = entry(resource, *amount).ok_or(AttachmentError::StackEncoding { index })?;
        saved.insert("Slot".into(), Tag::Int(slot));
        stacks.push(Tag::Compound(saved));
    }
    let mut saved = Compound::new();
    saved.insert(JavaString::from("stacks"), Tag::List(stacks));
    Ok(saved)
}

/// Decode each occupied index. A high `Slot` does not allocate any of the preceding slots.
fn decode_stacks<R>(
    saved: &Compound,
    entry: impl Fn(&Compound) -> Option<(R, u32)>,
) -> Result<StackList<R>, AttachmentError> {
    let mut list = StackList::new();
    let Some(Tag::List(stacks)) = saved.get(&JavaString::from("stacks")) else {
        return Ok(list);
    };
    for stack in stacks {
        let Tag::Compound(stack) = stack else {
            return Err(AttachmentError::Decode);
        };
        let Some(&Tag::Int(slot)) = stack.get(&"Slot".into()) else {
            return Err(AttachmentError::Decode);
        };
        let slot = usize::try_from(slot).map_err(|_| AttachmentError::Decode)?;
        let held = entry(stack).ok_or(AttachmentError::Decode)?;
        list.insert(slot, held);
    }
    Ok(list)
}

#[cfg(test)]
#[path = "attached_storage_tests.rs"]
mod tests;
