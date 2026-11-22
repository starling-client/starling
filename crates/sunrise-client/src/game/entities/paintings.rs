//! Paintings: `HangingEntityItem.useOn` placement with `Painting.create`'s
//! variant choice (or the stack's `painting/variant`), `HangingEntity.survives`
//! checks every 100 ticks, breaking by attack (`dropItem`), saving, and
//! `PaintingRenderer` quads.
use super::{Game, Hit, entity_culling::default_range};
use std::sync::Arc;
use sunrise_client::{entities::Painting, items::ItemStack, mesher::Vertex, random_source};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub(super) use sunrise_gameplay::painting_variants::{
    PaintingVariants, Variant, frame_coexists, painting_coexists, painting_facing,
};
/// Whether the camera, at the origin of `corners`' space, is on the side of the quad
/// that `normal` points to.
fn faces_camera(corners: &[glam::DVec3; 4], normal: glam::DVec3) -> bool {
    let centre = (corners[0] + corners[1] + corners[2] + corners[3]) / 4.0;
    normal.dot(-centre) > 0.0
}

impl Game<'_> {
    /// `HangingEntity.survives`: no block collision in the box, solid blocks
    /// (or diodes) behind it and no conflicting hanging entity overlapping it.
    pub(super) fn painting_survives(&self, p: &Painting) -> bool {
        let (lo, hi) = p.bounds();
        let bounds = Aabb::new(
            Vec3::new(lo[0], lo[1], lo[2]),
            Vec3::new(hi[0], hi[1], hi[2]),
        );
        let mut scene = sunrise_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            return false;
        }
        // calculateSupportBox: moved half a block into the wall, deflated.
        let s = p.step().map(f64::from);
        let (slo, shi) = (
            std::array::from_fn::<f64, 3, _>(|i| lo[i] - s[i] * 0.5 + 1.0e-7),
            std::array::from_fn::<f64, 3, _>(|i| hi[i] - s[i] * 0.5 - 1.0e-7),
        );
        for x in slo[0].floor() as i32..=shi[0].floor() as i32 {
            for y in slo[1].floor() as i32..=shi[1].floor() as i32 {
                for z in slo[2].floor() as i32..=shi[2].floor() as i32 {
                    let f = self.res.facts().get(self.world.block(x, y, z));
                    if !(f.legacy_solid || f.has_class("DiodeBlock")) {
                        return false;
                    }
                }
            }
        }
        painting_coexists(p, &self.paintings, &self.item_frames)
    }
    /// `HangingEntityItem.useOn` on a client level: SUCCESS once
    /// `Painting.create` finds a surviving variant (and the stack's
    /// `painting/variant`, if any, also survives). The random pick and
    /// spawn are the server's.
    pub(super) fn painting_fits(&self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:painting" {
            return false;
        }
        let Some(facing) = painting_facing(hit.face) else {
            return false;
        };
        let pos = [
            hit.pos[0] + hit.face[0],
            hit.pos[1],
            hit.pos[2] + hit.face[2],
        ];
        let mut candidate = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.next_entity.wrapping_add(1),
            pos,
            facing,
            variant: Arc::from(""),
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        let mut any = false;
        for id in &self.painting_variants.placeable {
            let Some(v) = self.painting_variants.get(id) else {
                continue;
            };
            (candidate.width, candidate.height) = (v.width, v.height);
            if self.painting_survives(&candidate) {
                any = true;
                break;
            }
        }
        if !any {
            return false;
        }
        if let Some(v) = self.painting_variants.of_stack(stack).cloned() {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            return self.painting_survives(&candidate);
        }
        true
    }
    /// `HangingEntityItem.useOn` for a painting against `hit`'s face.
    pub(super) fn place_painting(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:painting" {
            return false;
        }
        // HangingEntityItem.mayPlace: never on a top or bottom face (FAIL).
        let Some(facing) = painting_facing(hit.face) else {
            self.use_failed = true;
            return true;
        };
        let pos = [
            hit.pos[0] + hit.face[0],
            hit.pos[1],
            hit.pos[2] + hit.face[2],
        ];
        self.next_entity += 1;
        let mut candidate = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.next_entity,
            pos,
            facing,
            variant: Arc::from(""),
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        // Painting.create: the largest placeable variants that survive, one at random.
        let mut fitting: Vec<Variant> = Vec::new();
        for id in self.painting_variants.placeable.clone() {
            let Some(v) = self.painting_variants.get(&id).cloned() else {
                continue;
            };
            (candidate.width, candidate.height) = (v.width, v.height);
            if self.painting_survives(&candidate) {
                fitting.push(v);
            }
        }
        // Nothing fits: CONSUME, which ItemStack.useOn still counts as a use.
        let Some(largest) = fitting.iter().map(|v| v.width * v.height).max() else {
            self.award_stat("used", &stack.item, 1);
            return true;
        };
        fitting.retain(|v| v.width * v.height >= largest);
        // Util.getRandomSafe(variants, candidate.random): the painting's own random.
        let pick = random_source::entity()
            .next_int_bound(fitting.len() as i32)
            .unwrap_or(0) as usize;
        let v = &fitting[pick];
        (candidate.variant, candidate.width, candidate.height) = (v.id.clone(), v.width, v.height);
        // EntityType.createDefaultStackConfig -> Painting.applyImplicitComponents:
        // the stack's `painting/variant` replaces the random pick, and that
        // painting has to survive where it goes (else the use is CONSUME).
        if let Some(v) = self.painting_variants.of_stack(stack).cloned() {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            if !self.painting_survives(&candidate) {
                self.award_stat("used", &stack.item, 1);
                return true;
            }
        }
        // `updateCustomEntityTag`: the item's `custom_data` and its `entity_data`
        // merged over the painting, so a `variant` there replaces the pick (and
        // has to survive, else the use is CONSUME).
        let own = sunrise_client::entity_nbt::painting_to_nbt(&candidate, [0; 4], None);
        let mut raw = sunrise_client::block_entity_item::stack_configured_entity(
            stack,
            "minecraft:painting",
            &own,
        );
        if let Some((_, _, Some(variant))) = sunrise_client::entity_nbt::painting_from_nbt(&raw)
            && *candidate.variant != *variant
            && let Some(v) = self
                .painting_variants
                .get(&sunrise_world::datafix::dynamic::namespaced(&variant))
                .cloned()
        {
            (candidate.variant, candidate.width, candidate.height) = (v.id, v.width, v.height);
            if !self.painting_survives(&candidate) {
                self.award_stat("used", &stack.item, 1);
                return true;
            }
        }
        if let sunrise_core::nbt::Tag::Compound(map) = &mut raw {
            map.remove(&"UUID".into());
        }
        if self.hanging_place_cancelled("minecraft:painting", pos) {
            return true;
        }
        self.persist.raw.insert(candidate.id, raw);
        let (lo, hi) = candidate.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.audio
            .play("minecraft:entity.painting.place", Some(at), 1.0, 1.0);
        // HangingEntityItem.useOn: ENTITY_PLACE at the painting, by the player, before it is
        // added; ItemStack.useOn then counts the use.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:entity_place", at, ctx);
        self.paintings.push(candidate);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        if !self.creative {
            self.consume_held();
        }
        self.award_stat("used", &stack.item, 1);
        // HangingEntityItem.useOn returns SUCCESS: the hand swings.
        self.swing_arm();
        true
    }
    /// The painting under the crosshair within reach and its distance.
    pub(super) fn painting_hit(&self) -> Option<(usize, f64)> {
        let mut pick = crate::game::pick_accumulator::PickAccumulator::new(self.entity_reach);
        self.painting_hit_with_pick(&mut pick)
    }
    pub(super) fn painting_hit_with_pick(
        &self,
        pick: &mut crate::game::pick_accumulator::PickAccumulator,
    ) -> Option<(usize, f64)> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, _) = hit?;
            let index = self.paintings.iter().position(|f| f.id == entity.id())?;
            return Some((index, distance));
        }
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, p) in self.paintings.iter().enumerate() {
            let (lo, hi) = p.bounds();
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && pick.consider(self, p.id, t0) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// `BlockAttachedEntity.hurtServer` -> `kill` and `dropItem`. False when a mod kept it
    /// (`native_hanging_break`): it stays at `index`.
    pub(super) fn break_painting(&mut self, index: usize, by_player: bool) -> bool {
        self.break_painting_by(
            index,
            by_player,
            by_player.then_some(super::commands::Target::Player),
        )
    }
    /// `BlockAttachedEntity.kill`: `ENTITY_DIE` at the painting, then `dropItem`.
    pub(super) fn break_painting_by(
        &mut self,
        index: usize,
        by_player: bool,
        source: Option<super::commands::Target>,
    ) -> bool {
        let Some((p, at)) = self.remove_painting(index, by_player) else {
            return false;
        };
        // BlockAttachedEntity.kill, before dropItem.
        self.entity_killed_event_by(at, source);
        self.drop_painting(p, at, by_player);
        true
    }
    /// `BlockAttachedEntity.tick`: a painting whose `survives()` failed is
    /// `discard()`ed and drops its item, with no game event.
    pub(super) fn discard_painting(&mut self, index: usize) -> bool {
        let Some((p, at)) = self.remove_painting(index, false) else {
            return false;
        };
        self.drop_painting(p, at, false);
        true
    }
    /// The removal `kill` and `discard` share: the mod hook, the entity, its
    /// chunk's save mark and the centre of its box.
    fn remove_painting(&mut self, index: usize, by_player: bool) -> Option<(Painting, [f64; 3])> {
        let (id, pos) = (self.paintings[index].id, self.paintings[index].pos);
        if self.hanging_break_cancelled("minecraft:painting", pos, by_player) {
            return None;
        }
        // Found again by id after the listener: gone, nothing is removed.
        let index = self.paintings.iter().position(|p| p.id == id)?;
        let p = self.paintings.remove(index);
        let (lo, hi) = p.bounds();
        let at = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        self.world.modified.insert((p.pos[0] >> 4, p.pos[2] >> 4));
        Some((p, at))
    }
    /// `Painting.dropItem`: the break sound, then the item unless drops are off
    /// or the player who broke it has infinite materials.
    fn drop_painting(&mut self, p: Painting, at: [f64; 3], by_player: bool) {
        if !self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ENTITY_DROPS)
        {
            return;
        }
        self.audio
            .play("minecraft:entity.painting.break", Some(at), 1.0, 1.0);
        if by_player && self.creative {
            return;
        }
        let stack = ItemStack {
            item: Arc::from("minecraft:painting"),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(
            stack,
            super::frames::hanging_drop_position(at, p.step()),
            [0.0; 3],
            10,
        );
    }
    /// BlockAttachedEntity.tick: check survival every 100 ticks.
    pub(super) fn tick_paintings(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.paintings.len() {
            let (lo, hi) = self.paintings[i].bounds();
            let centre = [0, 2].map(|k| ((lo[k] + hi[k]) / 2.0).floor() as i32);
            if !ticking(centre[0], centre[1]) {
                i += 1;
                continue;
            }
            // `Entity.tickCount` is raised before the rest of the tick, and is not saved.
            self.paintings[i].ticks = self.paintings[i].ticks.saturating_add(1);
            self.paintings[i].check_interval += 1;
            if self.paintings[i].check_interval >= 100 {
                self.paintings[i].check_interval = 0;
                let p = self.paintings[i].clone();
                if !self.painting_survives(&p) && self.discard_painting(i) {
                    continue;
                }
            }
            i += 1;
        }
    }
    /// PaintingRenderer: the variant on the front, the back texture behind and
    /// on the edges.
    pub(super) fn painting_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let sprites = &self.gui.atlas.sprites;
        let partial = alpha as f32;
        for p in &self.paintings {
            // EntityRenderer.shouldRender: the default range and its box,
            // measured from the box's centre (HangingEntity's position).
            let (lo, hi) = p.bounds();
            let (lo, hi) = (glam::DVec3::from(lo), glam::DVec3::from(hi));
            let range = default_range(hi - lo, self.view_scale());
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let culling = self.entity_cull_decision(p, 1.0);
            if culling.hides_entity() {
                continue;
            }
            if let Some(resolved) = super::native_entity_renderers::painting_renderer() {
                let entity_start = out.len();
                self.mark_motion(entity_start, Self::MOTION_OBJECT | u64::from(p.id));
                let (lo, hi) = p.bounds();
                let at = glam::DVec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                );
                let facing = match p.facing & 3 {
                    0 => mod_api::Direction::South,
                    1 => mod_api::Direction::West,
                    2 => mod_api::Direction::North,
                    _ => mod_api::Direction::East,
                };
                let entity: mod_api::EntityRenderState =
                    mod_api::__private::built::EntityRenderState {
                        id: p.id,
                        partial,
                        age: p.ticks as f32 + partial,
                        yaw: f32::from(p.facing & 3) * 90.0,
                        pitch: 0.0,
                        living: None,
                        tnt_fuse: None,
                        orb_value: None,
                        painting: Some(mod_api::__private::built::PaintingRenderState {
                            facing,
                            width: p.width,
                            height: p.height,
                        }),
                        item_frame: None,
                        attachments: None,
                    }
                    .into();
                let light = self.world.light(p.pos[0], p.pos[1], p.pos[2]);
                let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
                if self.mod_entity_renderer_geometry(
                    &resolved,
                    &entity,
                    "minecraft:painting",
                    at - camera,
                    Some(at),
                    p.pos,
                    packed,
                    out,
                ) {
                    self.finish_entity_occlusion(culling, p, out, entity_start);
                    continue;
                }
            }
            let Some(back) = sprites.get("painting/back").copied() else {
                continue;
            };
            let Some(v) = self.painting_variants.get(&p.variant) else {
                continue;
            };
            let Some(front) = sprites.get(&format!("painting/{}", v.asset)).copied() else {
                continue;
            };
            let (lo, hi) = p.bounds();
            let c = glam::DVec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            ) - camera;
            let s = p.step().map(f64::from);
            let l = p.left().map(f64::from);
            let out_dir = glam::DVec3::new(s[0], 0.0, s[2]);
            // The viewer's right is the painting's left.
            let right = glam::DVec3::new(l[0], 0.0, l[2]);
            let up = glam::DVec3::Y;
            let (hw, hh, d) = (
                f64::from(p.width) / 2.0,
                f64::from(p.height) / 2.0,
                1.0 / 32.0,
            );
            let entity_start = out.len();
            self.mark_motion(entity_start, Self::MOTION_OBJECT | u64::from(p.id));
            let light = self.world.light(p.pos[0], p.pos[1], p.pos[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let mut quad =
                |corners: [glam::DVec3; 4], normal: glam::DVec3, uv: [[f32; 2]; 4], shade: f32| {
                    // Back faces are culled, as the entity pipeline culls them: inside the
                    // painting's box nothing of it shows. `corners` are relative to the camera.
                    if !faces_camera(&corners, normal) {
                        return;
                    }
                    let k = (255.0 * shade) as u32;
                    let color = 0xFF00_0000 | k << 16 | k << 8 | k;
                    for (p, uv) in corners.iter().zip(uv) {
                        out.push(Vertex {
                            normal: [0; 4],
                            pos: p.as_vec3().to_array(),
                            uv,
                            layer: 0,
                            color,
                            light: packed,

                            glint_uv: [0.0, 0.0],
                        });
                    }
                };
            let full = |r: sunrise_client::hud::Rect| {
                [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]]
            };
            let f = c + out_dir * d;
            quad(
                [
                    f - right * hw - up * hh,
                    f + right * hw - up * hh,
                    f + right * hw + up * hh,
                    f - right * hw + up * hh,
                ],
                out_dir,
                full(front),
                1.0,
            );
            let b = c - out_dir * d;
            quad(
                [
                    b + right * hw - up * hh,
                    b - right * hw - up * hh,
                    b - right * hw + up * hh,
                    b + right * hw + up * hh,
                ],
                -out_dir,
                full(back),
                0.8,
            );
            // Edges: a one-pixel strip of the back texture.
            let strip = [
                [back.u0, back.v0 + (back.v1 - back.v0) / 16.0],
                [back.u1, back.v0 + (back.v1 - back.v0) / 16.0],
                [back.u1, back.v0],
                [back.u0, back.v0],
            ];
            let (t, bt) = (up * hh, -up * hh);
            quad(
                [
                    b + t + right * hw,
                    b + t - right * hw,
                    f + t - right * hw,
                    f + t + right * hw,
                ],
                up,
                strip,
                1.0,
            );
            quad(
                [
                    f + bt + right * hw,
                    f + bt - right * hw,
                    b + bt - right * hw,
                    b + bt + right * hw,
                ],
                -up,
                strip,
                0.5,
            );
            quad(
                [
                    b - right * hw - up * hh,
                    f - right * hw - up * hh,
                    f - right * hw + up * hh,
                    b - right * hw + up * hh,
                ],
                -right,
                strip,
                0.6,
            );
            quad(
                [
                    f + right * hw - up * hh,
                    b + right * hw - up * hh,
                    b + right * hw + up * hh,
                    f + right * hw + up * hh,
                ],
                right,
                strip,
                0.6,
            );
            self.finish_entity_occlusion(culling, p, out, entity_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::entities::ItemFrame;

    #[test]
    fn hanging_entities_block_overlapping_other_types() {
        let p = Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            pos: [0, 0, 0],
            facing: 0,
            variant: Arc::from("minecraft:kebab"),
            width: 1,
            height: 1,
            check_interval: 0,
            ticks: 0,
        };
        let mut f = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 2,
            pos: [0, 0, 0],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            ticks: 0,
            custom_name: None,
            random: sunrise_client::random_source::entity(),
        };
        for glow in [false, true] {
            f.glow = glow;
            assert!(!frame_coexists(&f, &[], std::slice::from_ref(&p)));
            assert!(!painting_coexists(&p, &[], std::slice::from_ref(&f)));
            f.pos = [2, 0, 0];
            assert!(frame_coexists(&f, &[], std::slice::from_ref(&p)));
            assert!(painting_coexists(&p, &[], std::slice::from_ref(&f)));
            f.pos = [0, 0, 0];
        }
    }

    /// `HangingEntityItem.useOn` returns SUCCESS (the hand swings) when a
    /// painting is created on a wall. Floor, ceiling, or a failed create is
    /// CONSUME: no swing.
    fn painting_use_on_swings(face: [i32; 3], created: bool) -> bool {
        painting_facing(face).is_some() && created
    }

    #[test]
    fn placing_a_painting_on_a_wall_swings() {
        assert!(painting_use_on_swings([0, 0, 1], true));
        assert!(painting_use_on_swings([-1, 0, 0], true));
        assert!(painting_use_on_swings([0, 0, -1], true));
        assert!(painting_use_on_swings([1, 0, 0], true));
        assert!(!painting_use_on_swings([0, 1, 0], true));
        assert!(!painting_use_on_swings([0, -1, 0], true));
        assert!(!painting_use_on_swings([0, 0, 1], false));
    }
}

#[cfg(test)]
mod face_tests {
    use super::faces_camera;
    use glam::DVec3;

    /// A quad at z = 2 facing -z (towards a camera at the origin) shows; seen from behind
    /// (the camera on the other side, the quad now at z = -2) it does not.
    #[test]
    fn a_quad_shows_only_to_the_side_it_faces() {
        let at = |z: f64| {
            [
                DVec3::new(-1.0, -1.0, z),
                DVec3::new(1.0, -1.0, z),
                DVec3::new(1.0, 1.0, z),
                DVec3::new(-1.0, 1.0, z),
            ]
        };
        let normal = DVec3::new(0.0, 0.0, -1.0);
        assert!(faces_camera(&at(2.0), normal));
        assert!(!faces_camera(&at(-2.0), normal));
        // Edge on: nothing to draw.
        assert!(!faces_camera(&at(0.0), normal));
    }
}
