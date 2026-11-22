//! The skull types native mods register (`EntityRenderersEvent.CreateSkullModels`), which a
//! `minecraft:head` item model may name.
use super::identifier;
use std::{collections::BTreeSet, sync::RwLock};

static SKULL: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// The seven built-in vanilla `SkullBlock.Types` identifiers, which mods cannot re-register.
const VANILLA_SKULL_KINDS: [&str; 7] = [
    "minecraft:skeleton",
    "minecraft:wither_skeleton",
    "minecraft:player",
    "minecraft:zombie",
    "minecraft:creeper",
    "minecraft:piglin",
    "minecraft:dragon",
];

/// Keeps the skull types native mods registered, before resources load.
///
/// Any valid canonical `<namespace>:<path>` identifier (including `minecraft:` for non-vanilla
/// paths) is accepted; invalid identifiers, the seven built-in vanilla skull types, and duplicate
/// registrations are logged as errors and left out, matching NeoForge's
/// `EntityRenderersEvent.CreateSkullModels`. Returns the ids kept, in registration order.
pub fn register_skull_kinds(ids: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut set = SKULL.write().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if !identifier(&id).is_ok_and(|parsed| parsed == id) {
            log::error!("skull type {id:?} is not a valid canonical <namespace>:<path> identifier");
        } else if VANILLA_SKULL_KINDS.contains(&id.as_str()) {
            log::error!("skull type {id} conflicts with a built-in vanilla skull type");
        } else if !set.insert(id.clone()) {
            log::error!("skull type {id} is already registered");
        } else {
            kept.push(id);
        }
    }
    kept
}

/// Whether a native mod registered `kind` as a skull type a `minecraft:head` model may name.
pub fn is_skull_kind(kind: &str) -> bool {
    SKULL
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(kind)
}
