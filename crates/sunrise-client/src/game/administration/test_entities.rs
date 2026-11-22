//! What the game tests do to the entities and chunks of a test's area:
//! `StructureUtils.clearSpaceForStructure`'s and `GameTestInfo.succeed`'s
//! entity removal, and the force-loaded chunks of
//! `TestInstanceBlockEntity.forceLoadChunks` and `GameTestRunner`'s release.
use super::Game;
use sunrise_core::bounding_box::BoundingBox;

pub(super) use sunrise_gameplay::test_entities::{Aabb, intersects, standing};
pub use sunrise_gameplay::test_entities::{aabb_of, inflate};

impl Game<'_> {
    /// `level.getEntitiesOfClass(Entity.class, bounds, mob -> !(mob instanceof
    /// Player)).forEach(Entity::discard)`: every entity but a player whose box
    /// meets `bounds` is removed without dying or dropping anything.
    pub(super) fn discard_entities_in(&mut self, bounds: Aabb) {
        let at = |v: sunrise_core::geometry::Vec3| [v.x, v.y, v.z];
        for m in &mut self.mobs {
            if !m.removed
                && intersects(
                    standing(at(m.position), f64::from(m.width()), f64::from(m.height())),
                    bounds,
                )
            {
                m.removed = true;
            }
        }
        // EntityType.ITEM and EXPERIENCE_ORB.
        self.dropped
            .retain(|i| !intersects(standing(at(i.position), 0.25, 0.25), bounds));
        self.orbs
            .retain(|o| !intersects(standing(at(o.position), 0.5, 0.5), bounds));
        self.creatures
            .projectiles
            .retain(|p| !intersects(standing(at(p.position), 0.5, 0.5), bounds));
        self.armor_stands.retain(|a| {
            // ArmorStand.getDimensions: a marker has none, a small one is
            // half scale.
            let (w, h) = match (a.marker, a.small) {
                (true, _) => (0.0, 0.0),
                (_, true) => (0.25, 0.9875),
                _ => (0.5, 1.975),
            };
            !intersects(standing(at(a.position), w, h), bounds)
        });
        self.item_frames.retain(|f| !intersects(f.bounds(), bounds));
        self.paintings.retain(|p| !intersects(p.bounds(), bounds));
        let (boat_w, boat_h) = (super::boats::WIDTH, super::boats::HEIGHT);
        let gone: Vec<u32> = self
            .boats
            .iter()
            .filter(|b| intersects(standing(at(b.position), boat_w, boat_h), bounds))
            .map(|b| b.id)
            .collect();
        for id in gone {
            if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                let b = self.boats.remove(i);
                self.boat_removed(&b);
            }
        }
        let (cart_w, cart_h) = (super::minecarts::WIDTH, super::minecarts::HEIGHT);
        self.carts
            .retain(|c| !intersects(standing(at(c.position), cart_w, cart_h), bounds));
        // EntityType.END_CRYSTAL is 2 by 2.
        self.crystals
            .retain(|c| !intersects(standing(at(c.position), 2.0, 2.0), bounds));
        // Entities Sunrise does not simulate have no box of their own: the
        // position decides.
        self.others.retain(|o| {
            let p = at(o.position);
            !(0..3).all(|i| p[i] >= bounds.0[i] && p[i] < bounds.1[i])
        });
    }

    /// `ServerLevel.setChunkForced(x, z, true)` for the chunks of `area`
    /// (`TestInstanceBlockEntity.forceLoadChunks`).
    pub(super) fn force_load_test_chunks(&mut self, area: &BoundingBox) {
        let (x0, x1) = (area.min[0] >> 4, area.max[0] >> 4);
        let (z0, z1) = (area.min[2] >> 4, area.max[2] >> 4);
        self.streaming
            .set_forced_area((x0, z0), (x1, z1), true, |_, _, _| {});
    }
}
