//! Starling's part of names.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: on a server, the visible custom names of the other objects, styled as the server
    /// sent them. Armor stands (the holograms) and mobs name themselves above, from the same
    /// entity data; text displays draw their own text.
    pub(super) fn remote_name_tags(&self, camera: glam::DVec3, tags: &mut Vec<NameTag>) {
        let Some(r) = &self.remote else {
            return;
        };
        for (id, (name, visible)) in &r.names {
            let (Some(name), true) = (name, *visible) else {
                continue;
            };
            let id = *id as u32;
            if self.armor_stands.iter().any(|a| a.id == id) || self.mobs.iter().any(|m| m.id == id)
            {
                continue;
            }
            // EntityAttachment.NAME_TAG: the entity's height, then 0.5 up.
            let Some(t) = r
                .entities
                .get(&(id as i32))
                .filter(|t| t.kind != "minecraft:text_display")
            else {
                continue;
            };
            let origin = glam::DVec3::new(t.position[0], t.position[1], t.position[2]);
            let at = origin + glam::DVec3::Y * 0.5;
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let light = self.world.light(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            );
            tags.push(NameTag {
                at,
                lines: vec![(name.resolve_legacy(), 0.0)],
                light,
                discrete: false,
                entity_id: id,
                kind: TagKind::Fixed(entity_kind(&t.kind)),
                origin,
                score: true,
            });
        }
    }
}

/// An entity type's id kept for the life of the process, as a name tag's kind is `&'static str`
/// (`TagKind::Fixed`). Equal ids share one allocation: a server's few entity types, once each.
fn entity_kind(id: &str) -> &'static str {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, PoisonError};
    static POOL: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut pool = POOL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&known) = pool.get(id) {
        return known;
    }
    let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
    pool.insert(leaked);
    leaked
}
