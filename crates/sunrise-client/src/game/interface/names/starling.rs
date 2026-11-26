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
            let at = if let Some(t) = r.entities.get(&(id as i32))
                && t.kind != "minecraft:text_display"
            {
                glam::DVec3::new(t.position[0], t.position[1] + 0.5, t.position[2])
            } else {
                continue;
            };
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
            });
        }
    }
}
