//! Prepared cauldron callbacks and their server-side dispatch.
use super::*;
use mod_api::{
    __private::{
        RetainedCallback,
        game::{RegistrationContextNew, discard_registration},
    },
    HookId,
};
use std::{cell::RefCell, collections::BTreeSet, sync::Arc};
use sunrise_client::cauldron::interactions::Dispatcher;

#[derive(Clone)]
pub(super) struct Callback {
    pub(super) handler: Arc<dyn mod_api::Interaction>,
    pub(super) owner: &'static str,
    guard: Option<RetainedCallback>,
}
impl Callback {
    fn new(handler: Arc<dyn mod_api::Interaction>) -> Self {
        let owner = mod_api::__private::cauldron_owner();
        let guard = mod_api::__private::mod_index(owner)
            .map(|index| RetainedCallback::new(owner, index, "cauldron_interaction", ""));
        Self {
            handler,
            owner,
            guard,
        }
    }
}
pub(super) type Table = Dispatcher<Callback>;
thread_local! {
    static TABLE: RefCell<Arc<Table>> = RefCell::new(Arc::new(Table::default()));
}
pub(super) fn snapshot() -> Arc<Table> {
    TABLE.with_borrow(Arc::clone)
}
pub(super) fn clear() {
    install(Arc::new(Table::default()));
}
pub(super) fn install(table: Arc<Table>) {
    TABLE.with_borrow_mut(|old| *old = table);
}

fn discard_callback(callback: Callback) {
    discard_registration(
        (
            HookId::RegisterCauldronInteraction,
            "register_cauldron_interaction",
        ),
        Some(callback.owner),
        callback.handler,
    );
}

/// One admitted cauldron registration edit, replayed in registration order at `finish`.
enum Edit {
    Item {
        block: String,
        item: String,
        callback: Callback,
    },
    AllItem {
        item: String,
        callback: Callback,
    },
    Tag {
        block: String,
        tag: String,
        callback: Callback,
    },
    AllTag {
        tag: String,
        callback: Callback,
    },
}
impl Edit {
    fn into_callback(self) -> Callback {
        match self {
            Self::Item { callback, .. }
            | Self::AllItem { callback, .. }
            | Self::Tag { callback, .. }
            | Self::AllTag { callback, .. } => callback,
        }
    }
}
pub(in crate::game) struct Registrar<'a> {
    items: &'a Items,
    registry: &'a sunrise_world::registry::BlockRegistry,
    edits: Vec<Edit>,
    error: Option<mod_api::CauldronRegistrationError>,
}
/// A tag as entries keep it: its resource location, without `#`.
fn tag_id(tag: &str) -> Option<String> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    mod_api::Identifier::parse(tag)
        .ok()
        .map(|id| id.to_string())
}
impl<'a> Registrar<'a> {
    fn new(items: &'a Items, registry: &'a sunrise_world::registry::BlockRegistry) -> Self {
        Self {
            items,
            registry,
            edits: Vec::new(),
            error: None,
        }
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn mod_api::CauldronInteractions) -> bool,
    ) {
        let mut pending = Registrar::new(self.items, self.registry);
        if listen(&mut pending) {
            let Registrar { edits, error, .. } = pending;
            self.edits.extend(edits);
            if let Some(error) = error {
                self.error = Some(error);
            }
        } else {
            for edit in pending.edits {
                discard_callback(edit.into_callback());
            }
        }
    }

    /// Record an admission error, which fails the whole preparation when it finishes.
    fn refuse(
        &mut self,
        error: mod_api::CauldronRegistrationError,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        self.error = Some(error);
        Err(error)
    }
    fn finish(self) -> Result<Arc<Table>> {
        let Self { edits, error, .. } = self;
        if let Some(error) = error {
            for edit in edits {
                discard_callback(edit.into_callback());
            }
            return Err(format!("cauldron registration: {error:?}").into());
        }
        let mut table = Table::default();
        let mut blocks = table.blocks();
        let mut extra = BTreeSet::new();
        for edit in &edits {
            if let Edit::Item { block, .. } | Edit::Tag { block, .. } = edit
                && !table.contains_block(block)
            {
                extra.insert(block.clone());
            }
        }
        blocks.extend(extra);
        for edit in edits {
            match edit {
                Edit::Item {
                    block,
                    item,
                    callback,
                } => {
                    if let Some(old) = table.register(block, item, callback) {
                        discard_callback(old);
                    }
                }
                Edit::Tag {
                    block,
                    tag,
                    callback,
                } => {
                    if let Some(old) = table.register_tag(block, tag, callback) {
                        discard_callback(old);
                    }
                }
                Edit::AllItem { item, callback } => {
                    for block in &blocks {
                        if let Some(old) =
                            table.register(block.clone(), item.clone(), callback.clone())
                        {
                            discard_callback(old);
                        }
                    }
                    discard_callback(callback);
                }
                Edit::AllTag { tag, callback } => {
                    for block in &blocks {
                        if let Some(old) =
                            table.register_tag(block.clone(), tag.clone(), callback.clone())
                        {
                            discard_callback(old);
                        }
                    }
                    discard_callback(callback);
                }
            }
        }
        Ok(Arc::new(table))
    }
}
impl mod_api::CauldronInteractions for Registrar<'_> {
    fn register(
        &mut self,
        block: mod_api::BlockKey,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let Some(block) = mod_api::__private::block_key_id(block)
            .filter(|id| self.registry.default_state(id).is_some())
        else {
            return self.refuse(Error::UnknownBlock);
        };
        let Some(item) =
            mod_api::__private::item_key_id(item).filter(|id| self.items.items.contains_key(*id))
        else {
            return self.refuse(Error::UnknownItem);
        };
        self.edits.push(Edit::Item {
            block: block.to_string(),
            item: item.to_string(),
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_to_all(
        &mut self,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(item) =
            mod_api::__private::item_key_id(item).filter(|id| self.items.items.contains_key(*id))
        else {
            return self.refuse(mod_api::CauldronRegistrationError::UnknownItem);
        };
        self.edits.push(Edit::AllItem {
            item: item.to_string(),
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_tag(
        &mut self,
        block: mod_api::BlockKey,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let Some(block) = mod_api::__private::block_key_id(block)
            .filter(|id| self.registry.default_state(id).is_some())
        else {
            return self.refuse(Error::UnknownBlock);
        };
        let Some(tag) = tag_id(tag) else {
            return self.refuse(Error::InvalidTag);
        };
        self.edits.push(Edit::Tag {
            block: block.to_string(),
            tag,
            callback: Callback::new(handler),
        });
        Ok(())
    }
    fn register_tag_to_all(
        &mut self,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(tag) = tag_id(tag) else {
            return self.refuse(mod_api::CauldronRegistrationError::InvalidTag);
        };
        self.edits.push(Edit::AllTag {
            tag,
            callback: Callback::new(handler),
        });
        Ok(())
    }
}
impl Game<'_> {
    pub(super) fn prepare_cauldron_interactions(&self, items: &Items) -> Result<Arc<Table>> {
        let mut registrar = Registrar::new(items, self.pack.assets.registry);
        Self::native_register_cauldron_interaction(
            &mod_api::RegistrationContext::new(),
            &mut registrar,
        );
        registrar.finish()
    }
    /// Invoke a mod's authoritative cauldron interaction on the server.
    pub(super) fn use_mod_cauldron(
        &mut self,
        pos: [i32; 3],
        under_water: bool,
        callback: &Callback,
    ) -> sunrise_client::cauldron::Outcome {
        use crate::game::api_values::TryIntoApi;
        use mod_api::{CauldronDecision, CauldronUse};
        use sunrise_client::cauldron::Outcome;
        let Some(guard) = callback.guard.as_ref() else {
            log::warn!(
                "cauldron interaction for mod {} has no installed owner",
                callback.owner
            );
            return Outcome::Consume;
        };
        let input = (|| -> std::result::Result<CauldronUse, String> {
            let held = self
                .held()
                .cloned()
                .ok_or_else(|| "missing held stack".to_owned())?;
            let before = self.world.block(pos[0], pos[1], pos[2]);
            let index = api_values::block_state::current()
                .map_err(|err| format!("block state index unavailable: {err:?}"))?;
            let state = index
                .to_api(before)
                .map_err(|err| format!("block state {before:?} unavailable: {err:?}"))?;
            let epoch = self
                .native_lifecycle
                .epoch()
                .ok_or_else(|| "no active world epoch".to_owned())?;
            let player_id = self.native_acting_id();
            let player = self.player(player_id).unwrap_or_else(|| {
                // A fake player can be the acting body without an online-player entry.
                let actor = mod_api::EntityRef::new(epoch, self.native_acting_entity());
                mod_api::__private::game::player_ref(actor, player_id)
            });
            let level = self.level_ref(epoch, self.dim.key());
            let held = held
                .try_into_api()
                .map_err(|err| format!("held stack projection failed: {err:?}"))?;
            Ok(CauldronUse {
                level,
                pos,
                state,
                player,
                hand: match self.hand {
                    Hand::Main => mod_api::Hand::Main,
                    Hand::Off => mod_api::Hand::Off,
                },
                held,
                underwater: under_water,
                creative: self.creative,
            })
        })();
        let input = match input {
            Ok(input) => input,
            Err(why) => {
                log::warn!(
                    "cauldron interaction input refused for mod {}: {why}",
                    callback.owner
                );
                return Outcome::Consume;
            }
        };
        let decision = guard.call(|| callback.handler.interact(self, &input));
        match decision {
            Ok(CauldronDecision::Success) => Outcome::Success,
            Ok(CauldronDecision::Consume) => Outcome::Consume,
            Ok(_) => Outcome::Pass,
            Err(why) => {
                log::warn!("cauldron interaction refused: {why:?}");
                Outcome::Consume
            }
        }
    }
}

#[cfg(test)]
#[path = "cauldron_interactions/tests.rs"]
mod tests;
