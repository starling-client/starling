//! A level's shape, chunk format and facts, from the dimension type its server synced: what the
//! player's level takes on entering it (`enter_remote_dimension`), and a viewed level's world on
//! its `open` (`viewed_worlds`). Only the player's level also takes the type's environment
//! attributes (`build_level_environments`).
use super::Remote;
use crate::game::{Dim, remote_chunks::ChunkFormat};
use std::sync::{Arc, Weak};
use sunrise_client::{chunk::LevelShape, environment::Environment};
use sunrise_protocol::{packets::play, registry::DimensionType};
use sunrise_world::dimension_type::DimensionFacts;

/// What a level of the server is, from its synced dimension type.
pub(in crate::game) struct LevelFormat {
    pub(in crate::game) dimension: DimensionType,
    pub(in crate::game) shape: LevelShape,
    /// The format its chunk packets decode with.
    pub(in crate::game) chunk_format: Arc<ChunkFormat>,
    /// Its facts (what it does to fluids, mobs, beds and time); `None` when the server sent no
    /// type the game takes.
    pub(in crate::game) facts: Option<Arc<DimensionFacts>>,
    /// The synchronized dimension type's name and JSON, which the level's environment attributes
    /// start from (`Game::build_level_environments`); `None` only when neither the server nor the
    /// client's own resources have data for it (`fill_known_entries` fills a known pack's entries
    /// from the client's files).
    pub(in crate::game) synced_type: Option<(String, serde_json::Value)>,
}

impl crate::game::Game<'_> {
    /// The client's table of the server's levels on entering `entered`: a level past vanilla's
    /// three that the process does not know yet is added, so that [`Dim::from_id`] finds it, and
    /// the level left is dropped when the server opened it after Login (a runtime level), so that a
    /// long session through many of them keeps none. A viewed world of such a level needs no
    /// entry: it keeps its own name. Only a level this connection added leaves the table: in single
    /// player the integrated server shares the table, and a runtime level it opened is its own
    /// until it closes it (`runtime_levels.rs`).
    pub(in crate::game) fn track_remote_levels(&mut self, entered: &str) {
        if let Some(r) = self.remote.as_mut()
            && r.level != entered
            && !Dim::is_vanilla(&r.level)
            && !r.levels.contains(&r.level)
        {
            if r.added_levels.remove(&r.level) {
                Dim::remove_extra(&r.level);
            }
            r.facts_cache.remove(&r.level);
        }
        if !Dim::is_vanilla(entered)
            && !Dim::exists(entered)
            && let Some(r) = self.remote.as_mut()
        {
            Dim::add_extra(entered);
            r.added_levels.insert(entered.to_owned());
        }
    }

    /// The environment attributes of the level the player is in, from its synchronized dimension
    /// type, as `EnvironmentAttributeSystem.addDimensionLayer` takes `level.dimensionType()`'s
    /// attributes (and `addDynamicLayers` its timelines): the type's attributes and timelines over
    /// the client's own environment of that type, for gameplay, and over the packs' Polytone stack
    /// of that type, whose modifiers then apply over them, for the frame. A built-in level is
    /// included, so a data pack's Overworld reaches the frame; the base is the client's
    /// environment of the same type, so the Nether and the End keep their own file's layers (no
    /// weather) instead of the Overworld's. Only the three vanilla type IDs have a pack stack: a
    /// mod's type is drawn without the packs' layers. Run again when the packs change.
    ///
    /// Building one reads every biome file, so both are kept by type name with the JSON and the
    /// packs' stacks they were built from (`LevelEnvironments`): entering a level of an unchanged
    /// type again (a respawn, a travel back) reuses them as kept, without cloning the JSON, and a
    /// pack reload, which makes new stacks, builds only the frame's again, in the same entry.
    pub(in crate::game) fn build_level_environments(&mut self) {
        let stacks = self.pack.environments.as_ref();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let Some((name, json)) = &r.synced_type else {
            r.level_environment = None;
            r.level_pack_environment = None;
            return;
        };
        if let Some(e) = r
            .level_environments
            .get_mut(name)
            .filter(|e| e.json == *json)
        {
            if !same_stacks(e.stacks.as_ref(), stacks) {
                e.pack = pack_environment(name, json, stacks);
                e.stacks = stacks.map(Arc::downgrade);
            }
            r.level_environment = e.own.clone();
            r.level_pack_environment = e.pack.clone();
            return;
        }
        let own = Environment::with_type(self.res.environment_of(name), json).map(Arc::new);
        let pack = pack_environment(name, json, stacks);
        r.level_environments.insert(
            name.clone(),
            LevelEnvironments {
                json: json.clone(),
                own: own.clone(),
                stacks: stacks.map(Arc::downgrade),
                pack: pack.clone(),
            },
        );
        r.level_environment = own;
        r.level_pack_environment = pack;
    }
}

/// The environments `Game::build_level_environments` built from one synchronized dimension type,
/// and what they were built from.
pub(super) struct LevelEnvironments {
    /// The type's JSON both were built from.
    json: serde_json::Value,
    /// Over the client's own environment of the type: gameplay's.
    own: Option<Arc<Environment>>,
    /// The packs' stacks `pack` was built over (`None` without pack modifiers). A weak handle:
    /// a reload makes new stacks, and the old ones' address is not reused while it lives.
    stacks: Option<Weak<[Environment; 3]>>,
    /// Over the packs' stack of the type: the frame's.
    pack: Option<Arc<Environment>>,
}

/// The frame's environment of the synchronized type `name` with `json`: the type over the packs'
/// stack of that type, or `None` without pack modifiers or for a type with no pack stack.
fn pack_environment(
    name: &str,
    json: &serde_json::Value,
    stacks: Option<&Arc<[Environment; 3]>>,
) -> Option<Arc<Environment>> {
    let base = match name {
        "minecraft:overworld" => &stacks?[0],
        "minecraft:the_nether" => &stacks?[1],
        "minecraft:the_end" => &stacks?[2],
        _ => return None,
    };
    Environment::with_type(base, json).map(Arc::new)
}

/// Whether `built` are the packs' current stacks `now`.
fn same_stacks(
    built: Option<&Weak<[Environment; 3]>>,
    now: Option<&Arc<[Environment; 3]>>,
) -> bool {
    match (built, now) {
        (None, None) => true,
        (Some(built), Some(now)) => std::ptr::eq(built.as_ptr(), Arc::as_ptr(now)),
        _ => false,
    }
}

impl Remote {
    /// The format and facts of the level `spawn` names, from the server's dimension type. The
    /// facts are cached by level and named type.
    pub(in crate::game) fn level_format(&mut self, spawn: &play::SpawnInfo) -> LevelFormat {
        let dimension = self.registries.dimension(spawn.dimension_type);
        // The server's dimension type gives the level's shape, and its
        // chunks the shape's column.
        let shape = LevelShape::new(
            dimension.min_y,
            dimension.height,
            dimension.logical_height,
            dimension.has_skylight,
            dimension.has_ceiling,
        )
        .unwrap_or_else(|_| Dim::from_id(&dimension.name).shape());
        let chunk_format = Arc::new(
            self.chunk_format
                .with_dimension(&dimension)
                .in_column(shape.column()),
        );
        // The server's dimension type gives the level's facts (what it does to fluids, mobs,
        // beds and time), not only its shape: a level of a mod the server runs is no
        // longer taken for the Overworld.
        let mut facts = None;
        let synced_type = self.registries.dimension_json(spawn.dimension_type);
        if let Some((name, json)) = &synced_type {
            let cached = self
                .facts_cache
                .get(&spawn.dimension)
                .filter(|(old_name, old, _)| old_name == name && old == json)
                .map(|(_, _, facts)| facts.clone());
            facts = cached.or_else(|| {
                let base = DimensionFacts::base_for_type(name);
                let facts = (|| {
                    for field in ["coordinate_scale", "ambient_light"] {
                        if json.get(field).is_some()
                            && !json[field].as_f64().is_some_and(f64::is_finite)
                        {
                            return Err(format!("invalid {field}"));
                        }
                    }
                    if json.get("monster_spawn_block_light_limit").is_some()
                        && json["monster_spawn_block_light_limit"].as_i64().is_none()
                    {
                        return Err("invalid monster_spawn_block_light_limit".into());
                    }
                    DimensionFacts::patched(base, name, json)
                })()
                .map_err(|why| log::warn!("the server's dimension type {name} is refused ({why})"))
                .unwrap_or_else(|_| base.clone());
                if !sane_facts(&facts) {
                    log::warn!(
                        "the server's dimension type {name} is refused (a value out of range)"
                    );
                    return Some(Arc::new(base.clone()));
                }
                let facts = Arc::new(facts);
                self.facts_cache.insert(
                    spawn.dimension.clone(),
                    (name.clone(), json.clone(), facts.clone()),
                );
                Some(facts)
            });
        }
        LevelFormat {
            dimension,
            shape,
            chunk_format,
            facts,
            synced_type,
        }
    }
}

/// Whether a synchronized dimension type's values are inside the codec's ranges
/// (`DimensionType.java`: a coordinate scale of `1.0E-5F` to `3.0E7`, a monster spawn block light
/// limit of 0 to 15; the ambient light has no range), so a portal's ratio of two scales is finite.
fn sane_facts(facts: &DimensionFacts) -> bool {
    (f64::from(1.0e-5_f32)..=3.0e7).contains(&facts.coordinate_scale)
        && (0..=15).contains(&facts.monster_spawn_block_light_limit)
        && facts.ambient_light.is_finite()
}

#[cfg(test)]
mod tests {
    use super::sane_facts;
    use sunrise_world::dimension_type::DimensionFacts;

    /// A type outside the codec's ranges is not taken; the vanilla ones are.
    #[test]
    fn a_type_outside_the_codecs_ranges_is_refused() {
        let mut facts = DimensionFacts::overworld().clone();
        assert!(sane_facts(&facts));
        assert!(sane_facts(DimensionFacts::of_builtin_level(
            sunrise_core::LevelKey::NETHER
        )));
        for scale in [0.0, f64::from_bits(1), -1.0, 3.1e7, f64::NAN] {
            facts.coordinate_scale = scale;
            assert!(!sane_facts(&facts), "{scale}");
        }
        facts.coordinate_scale = 1.0;
        // the codec sets no range on the ambient light
        facts.ambient_light = 1.5;
        assert!(sane_facts(&facts));
        facts.monster_spawn_block_light_limit = 16;
        assert!(!sane_facts(&facts));
    }
}
