//! Starling: Horizon's mod API names, on Sunrise's, for a Horizon mod built unchanged (mod-builds' adoption puts
//! this module in each adopted mod with `use crate::starling_compat::*;` in its files). Each is the old call's
//! shape over the new call, nothing more. Generated code of the patch crates names these too.
#![allow(dead_code, unused_imports)]
use mod_api::{ClientHost, ClientModifiers, Hand, Host, ItemStack, LivingRef, PlayerRef};

/// Horizon's `ClientHost::mod_key` and `consume_click`, over a key mapping's handle (`mod_key_mapping`).
pub trait StarlingModKeys {
    fn mod_key(&self, owner: &str, id: &str) -> Option<(String, ClientModifiers)>;
    fn consume_click(&mut self, owner: &str, id: &str) -> bool;
    fn is_mod_key_down(&self, owner: &str, id: &str) -> bool;
}

impl StarlingModKeys for dyn ClientHost + '_ {
    fn mod_key(&self, owner: &str, id: &str) -> Option<(String, ClientModifiers)> {
        let mapping = self.mod_key_mapping(owner, id).ok()?;
        mapping
            .binding(self)
            .ok()
            .map(|(key, modifiers)| (key.to_owned(), modifiers))
    }

    fn consume_click(&mut self, owner: &str, id: &str) -> bool {
        self.mod_key_mapping(owner, id)
            .ok()
            .and_then(|mapping| mapping.consume_click(&mut *self).ok())
            .unwrap_or(false)
    }

    fn is_mod_key_down(&self, owner: &str, id: &str) -> bool {
        self.mod_key_mapping(owner, id)
            .ok()
            .and_then(|mapping| mapping.is_down(self).ok())
            .unwrap_or(false)
    }
}

/// Horizon's `PlayerRef::inventory_slots`: every slot with an item, numbered as `with_inventory` numbers them.
pub trait StarlingInventory {
    fn inventory_slots(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>>;
    fn inventory(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>>;
}

impl StarlingInventory for PlayerRef {
    fn inventory_slots(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>> {
        self.with_inventory(host, |view| {
            view.iter()
                .filter_map(|(slot, stack)| stack.project().map(|stack| (slot as usize, stack)))
                .collect()
        })
    }

    fn inventory(self, host: &dyn Host) -> Option<Vec<(usize, ItemStack)>> {
        self.inventory_slots(host)
    }
}

/// Horizon's `LivingRef::hand_item`: the stack in `hand`, if any.
pub trait StarlingHands {
    fn hand_item(self, host: &dyn Host, hand: Hand) -> Option<ItemStack>;
}

impl StarlingHands for LivingRef {
    fn hand_item(self, host: &dyn Host, hand: Hand) -> Option<ItemStack> {
        self.with_hand_item(host, hand, |stack| stack.project()).flatten()
    }
}

/// Horizon's ids were `&str` where Sunrise's are `Option<&str>`: the old text methods, an unknown id as "".
pub trait StarlingId {
    fn to_string(&self) -> String;
    fn is_empty(&self) -> bool;
    fn as_str(&self) -> &str;
    fn starts_with(&self, prefix: &str) -> bool;
    fn ends_with(&self, suffix: &str) -> bool;
}

impl StarlingId for Option<&str> {
    fn to_string(&self) -> String {
        self.unwrap_or_default().to_owned()
    }
    fn is_empty(&self) -> bool {
        self.unwrap_or_default().is_empty()
    }
    fn as_str(&self) -> &str {
        self.unwrap_or_default()
    }
    fn starts_with(&self, prefix: &str) -> bool {
        self.unwrap_or_default().starts_with(prefix)
    }
    fn ends_with(&self, suffix: &str) -> bool {
        self.unwrap_or_default().ends_with(suffix)
    }
}

/// Values Horizon handed as `Option` that Sunrise hands plain: the old `unwrap_or` still reads them.
pub trait StarlingPlain: Sized {
    fn unwrap_or(self, _default: Self) -> Self {
        self
    }
    fn unwrap_or_default(self) -> Self {
        self
    }
}
/// `.ok()` / `.map_or(..)` read a value Horizon handed as a `Result` or an `Option`: the value itself.
pub trait StarlingPlainRead: Sized {
    fn ok(self) -> Option<Self> {
        Some(self)
    }
    fn map_or<U>(self, _default: U, f: impl FnOnce(Self) -> U) -> U {
        f(self)
    }
}
impl StarlingPlainRead for i32 {}
impl StarlingPlainRead for u32 {}
impl StarlingPlainRead for f32 {}
impl StarlingPlainRead for f64 {}
impl StarlingPlainRead for bool {}

/// `.ok()` on what Horizon handed as a `Result` and Sunrise as an `Option`.
pub trait StarlingOptionOk<T> {
    fn ok(self) -> Option<T>;
}
impl<T> StarlingOptionOk<T> for Option<T> {
    fn ok(self) -> Option<T> {
        self
    }
}

/// Text Sunrise lends from the host that Horizon handed owned: `x.id(host).starling_owned()` frees the host again
/// (fixes.py adds it where the host is wanted mutably while the text lives).
pub trait StarlingOwned {
    type Owned;
    fn starling_owned(self) -> Self::Owned;
}
impl StarlingOwned for Option<&str> {
    type Owned = Option<String>;
    fn starling_owned(self) -> Option<String> {
        self.map(str::to_owned)
    }
}
impl StarlingOwned for &str {
    type Owned = String;
    fn starling_owned(self) -> String {
        self.to_owned()
    }
}

/// Horizon's entity tags were strings to iterate; Sunrise's are UTF-16 views.
pub trait StarlingTags {
    fn iter(&self) -> std::vec::IntoIter<String>;
}
impl StarlingTags for mod_api::EntityTags<'_> {
    fn iter(&self) -> std::vec::IntoIter<String> {
        (*self).iter_utf16().map(String::from_utf16_lossy).collect::<Vec<_>>().into_iter()
    }
}

impl StarlingPlain for i32 {}
impl StarlingPlain for u32 {}
impl StarlingPlain for f32 {}
impl StarlingPlain for f64 {}
impl StarlingPlain for bool {}

/// Horizon's `movement_input` event: the old fields over Sunrise's `movement_input_update` input.
/// The adoption renames the hook (`fn movement_input` becomes a `movement_input_update` that builds this, runs the
/// old body and writes it back): the old `forward`/`strafe` are Sunrise's `move_vector` `[left, forward]`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementInputEvent {
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub forward_key: bool,
    pub backward_key: bool,
    pub left_key: bool,
    pub right_key: bool,
    pub sprint: bool,
}

impl MovementInputEvent {
    pub fn of(input: &mod_api::MovementInput) -> Self {
        let keys = input.key_presses;
        Self {
            forward: input.move_vector[1],
            strafe: input.move_vector[0],
            jump: keys.jump,
            sneak: keys.sneak,
            forward_key: keys.forward,
            backward_key: keys.backward,
            left_key: keys.left,
            right_key: keys.right,
            sprint: keys.sprint,
        }
    }

    pub fn write(&self, input: &mut mod_api::MovementInput) {
        input.move_vector = [self.strafe, self.forward];
        let keys = &mut input.key_presses;
        keys.jump = self.jump;
        keys.sneak = self.sneak;
        keys.forward = self.forward_key;
        keys.backward = self.backward_key;
        keys.left = self.left_key;
        keys.right = self.right_key;
        keys.sprint = self.sprint;
    }
}

/// Horizon's level queries that returned lists, over Sunrise's visits (`visit_entities_in` and the rest): the same
/// entities in the same order, `None` where the visit refuses (a stale level, a client, a parked level).
pub trait StarlingLevelEntities {
    fn entities_in(self, host: &dyn Host, min: [f64; 3], max: [f64; 3]) -> Option<Vec<mod_api::EntityRef>>;
    fn all_entities(self, host: &dyn Host) -> Option<Vec<mod_api::EntityRef>>;
    fn online_players(self, host: &dyn Host) -> Option<Vec<PlayerRef>>;
}

impl StarlingLevelEntities for mod_api::LevelRef {
    fn entities_in(self, host: &dyn Host, min: [f64; 3], max: [f64; 3]) -> Option<Vec<mod_api::EntityRef>> {
        let mut found = Vec::new();
        self.visit_entities_in(host, min, max, &mut |entity| {
            found.push(entity);
            core::ops::ControlFlow::Continue(())
        })
        .ok()?;
        Some(found)
    }

    fn all_entities(self, host: &dyn Host) -> Option<Vec<mod_api::EntityRef>> {
        let mut found = Vec::new();
        self.visit_all_entities(host, &mut |entity| {
            found.push(entity);
            core::ops::ControlFlow::Continue(())
        })
        .ok()?;
        Some(found)
    }

    fn online_players(self, host: &dyn Host) -> Option<Vec<PlayerRef>> {
        let mut found = Vec::new();
        self.visit_online_players(host, &mut |player| {
            found.push(player);
            core::ops::ControlFlow::Continue(())
        })
        .ok()?;
        Some(found)
    }
}

/// Horizon's `insert_any` / `extract_any`, which moved what they could and said how much: Sunrise's `insert` and
/// `extract` (its old per-index ones are `insert_at` / `extract_at`), with a handler that answered more than it was
/// asked counted as nothing moved.
pub trait StarlingResources<R> {
    fn insert_any(&self, host: &mut dyn mod_api::ServerHost, tx: &mod_api::Transaction, resource: &R, amount: u32)
    -> u32;
    fn extract_any(&self, host: &mut dyn mod_api::ServerHost, tx: &mod_api::Transaction, resource: &R, amount: u32)
    -> u32;
}

impl<R, T: mod_api::ResourceHandler<R> + ?Sized> StarlingResources<R> for T {
    fn insert_any(&self, host: &mut dyn mod_api::ServerHost, tx: &mod_api::Transaction, resource: &R, amount: u32)
    -> u32 {
        self.insert(host, tx, resource, amount).unwrap_or(0)
    }

    fn extract_any(&self, host: &mut dyn mod_api::ServerHost, tx: &mod_api::Transaction, resource: &R, amount: u32)
    -> u32 {
        self.extract(host, tx, resource, amount).unwrap_or(0)
    }
}

/// Names Sunrise keeps at the API's root that Horizon's mods used without importing them (a hook's parameter types
/// the adoption's rules write, or the mod named them through a glob Sunrise no longer has).
pub use mod_api::{ConfigContext, HostSide, PackDiscoveryContext};
/// And every other root name of the API: Horizon's mods reached some through modules or globs that moved (a hook's
/// host trait, `Flow`, a reload context). A name the mod imports or defines itself wins over this glob.
pub use mod_api::*;

/// Horizon's `move_resources(host, from, to, filter, max, tx)` at the root, over Sunrise's `resource_util` one
/// (whose transaction is optional).
pub fn move_resources<'a, R: 'a>(
    host: &mut dyn mod_api::ServerHost,
    from: &'a mod_api::Capability<dyn mod_api::ResourceHandler<R>>,
    to: &'a mod_api::Capability<dyn mod_api::ResourceHandler<R>>,
    filter: impl Fn(&R) -> bool,
    max: u32,
    tx: &mod_api::Transaction,
) -> Result<u32, mod_api::Overanswered> {
    mod_api::resource_util::move_resources(host, from, to, filter, max, Some(tx))
}

/// Horizon's `block_entity_on_load` event: the type and the block entity's handle. Sunrise lends a scope instead,
/// whose view has the handle and whose host the old hook got beside the event (the adoption's rule builds this,
/// runs the old body on it with `subject.host()`).
pub struct BlockEntityOnLoadEvent {
    pub id: mod_api::BlockEntityTypeKey,
    pub entity: mod_api::BlockEntityRef,
}

impl BlockEntityOnLoadEvent {
    pub fn of(event: &mut mod_api::BlockEntityOnLoadEvent<'_>) -> Option<Self> {
        let entity = match &mut event.subject {
            mod_api::BlockEntitySideScope::Server(scope) => scope.with_view(|view| view.handle()),
            mod_api::BlockEntitySideScope::Client(scope) => scope.with_view(|view| view.handle()),
        }?;
        Some(Self { id: event.id, entity })
    }
}

/// Fields Horizon's types had that Sunrise's compute: tools/android_compat/fixes.py turns `x.field` into the call.
pub trait StarlingRespawnData {
    /// Horizon's `RespawnData::pos`: the block of Sunrise's `position`.
    fn starling_respawn_pos(&self) -> [i32; 3];
}

impl StarlingRespawnData for mod_api::RespawnData {
    fn starling_respawn_pos(&self) -> [i32; 3] {
        let pos = self.position.pos();
        [pos.x, pos.y, pos.z]
    }
}

/// An id Horizon handed as `Option<String>` (`.as_deref()` read it) that Sunrise hands as `Option<Option<&str>>`.
pub trait StarlingNestedId {
    fn as_deref(&self) -> Option<&str>;
}

impl StarlingNestedId for Option<Option<&str>> {
    fn as_deref(&self) -> Option<&str> {
        self.flatten()
    }
}
