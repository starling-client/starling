//! The block-state definition types native mods register (NeoForge's `RegisterBlockStateModels`),
//! bound once at start so a resource load invokes each type's provider directly when reading a
//! blockstate file naming it (`neoforge:definition_type`).
use mod_api::{
    __private::game::{BlockStateDefinitionRegistration, CallbackOwner, CallbackValue},
    Answered, BlockStateDefinitionFn, BlockStateDefinitionInput, BlockStateModelPick, Identifier,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
use sunrise_assets::blockstate::{Variant, bind::Properties};

struct BoundBlockStateModel {
    instantiate: CallbackValue<Arc<BlockStateDefinitionFn>>,
}

pub(crate) struct BlockStateModelTable(BTreeMap<String, BoundBlockStateModel>);

static DEFINITIONS: OnceLock<BlockStateModelTable> = OnceLock::new();

fn freeze(
    definitions: Vec<Answered<BlockStateDefinitionRegistration>>,
) -> (BlockStateModelTable, Vec<String>) {
    let mut bound = BTreeMap::new();
    let mut kept = Vec::new();
    for Answered { answer, mod_id, .. } in definitions {
        let id = answer.id;
        let Some(owner) = CallbackOwner::new(mod_id) else {
            log::warn!("Block state definition type {id} has no installed owner {mod_id}");
            CallbackOwner::for_mod(mod_id).dispose(
                "block state definition",
                "drop_provider",
                answer.instantiate,
            );
            continue;
        };
        let key = id.to_string();
        if let Some(earlier) = bound.insert(
            key.clone(),
            BoundBlockStateModel {
                instantiate: CallbackValue::new(
                    owner,
                    "block state definition",
                    "drop_provider",
                    answer.instantiate,
                ),
            },
        ) {
            log::warn!(
                "Sunrise: block state definition type {key} registered by {} is replaced by {mod_id}'s registration",
                earlier.instantiate.owner().id()
            );
        } else {
            kept.push(key);
        }
    }
    (BlockStateModelTable(bound), kept)
}

/// Binds the block-state definition providers native mods registered once at start, before
/// resources load. Any valid `Identifier` (including `minecraft:` and empty-path ids) is accepted;
/// a later registration for the same id replaces the earlier one (last-writer-wins) with a warning
/// naming both mods. Returns the unique ids kept, in first-registration order.
pub fn register(definitions: Vec<Answered<BlockStateDefinitionRegistration>>) -> Vec<String> {
    let (bound, kept) = freeze(definitions);
    if DEFINITIONS.set(bound).is_err() {
        log::warn!("block state definition types were registered already; these are left out");
        return Vec::new();
    }
    kept
}

pub(crate) fn installed_definitions() -> Option<&'static BlockStateModelTable> {
    DEFINITIONS.get()
}

/// Each state's variant, by index into `states`, as the bound provider for definition type `kind`
/// picks them for `block` from the file's JSON `config`. It runs on the resource loader, as a
/// registration. An error says why the file is refused; the caller names the file.
pub(crate) fn instantiate(
    definitions: Option<&BlockStateModelTable>,
    kind: &str,
    block: &str,
    config: &Value,
    states: &[Properties],
) -> Result<Vec<Option<Variant>>, String> {
    let Some(entry) = definitions.and_then(|table| table.0.get(kind)) else {
        return Err(format!("unknown block state definition type {kind}"));
    };
    let block = Identifier::parse(block).map_err(|e| format!("block {block}: {e}"))?;
    let config = sunrise_core::json_nbt::json_to_tag(config)
        .map_err(|e| format!("definition type {kind}: its JSON doesn't convert to NBT: {e}"))?;

    let input = BlockStateDefinitionInput::new(&block, &config, states);
    let owner = entry.instantiate.owner();
    let picks = match owner.call(
        "block state definition",
        "instantiate_block_state_definition",
        || (entry.instantiate.value())(&input),
    ) {
        Ok(Ok(picks)) => picks,
        Ok(Err(reason)) => {
            return Err(format!(
                "mod {} definition type {kind} refused it: {reason}",
                owner.id()
            ));
        }
        Err(error) => {
            return Err(format!("definition type {kind}: {error}"));
        }
    };
    picked(kind, states.len(), picks)
}

/// The picks a type's provider answered, one slot per state: refused whole by a `Refuse`, a pick
/// of a state the block lacks or of one picked already, a model id that doesn't read, or no pick
/// at all.
fn picked(
    kind: &str,
    count: usize,
    picks: Vec<BlockStateModelPick>,
) -> Result<Vec<Option<Variant>>, String> {
    let mut models = vec![None; count];
    for pick in picks {
        match pick {
            BlockStateModelPick::Refuse { reason } => {
                return Err(format!("definition type {kind} refused it: {reason}"));
            }
            BlockStateModelPick::Model {
                state,
                model,
                turns,
                uvlock,
            } => {
                let slot = usize::try_from(state)
                    .ok()
                    .and_then(|state| models.get_mut(state))
                    .ok_or_else(|| {
                        format!(
                            "definition type {kind} picked state {state}, but the block has \
                             {count} states"
                        )
                    })?;
                let variant = Variant::new(&model, turns, uvlock)
                    .map_err(|e| format!("definition type {kind} picked model {model:?}: {e}"))?;
                if slot.replace(variant).is_some() {
                    return Err(format!("definition type {kind} picked state {state} twice"));
                }
            }
        }
    }
    if models.iter().all(Option::is_none) {
        return Err(format!("definition type {kind} picked no model"));
    }
    Ok(models)
}

#[cfg(test)]
#[path = "mod_block_state_models_tests.rs"]
mod tests;
