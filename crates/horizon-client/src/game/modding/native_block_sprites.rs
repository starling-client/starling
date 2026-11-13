//! Where blocks' textures are in the block sprite arrays, for a client mod drawing blocks with the player's own
//! textures (resource packs included) in its own pass: with `WorldDrawn::block_sprites`, the arrays themselves.
use super::Game;
use horizon_client::block_sprites::{BlockSprites, Lookup, SpriteRef};
use horizon_core::geometry::Direction;
use horizon_world::registry::BlockStateId;
use mod_api::{AtlasRegion, BlockFaceTexture, Identifier};

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: what each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left out take the
    /// block's default) draws on each of its six sides, as the mesher bakes its model: each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them), where it lies on the side, how its texture runs across it,
    /// its tint source and whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    pub(super) fn block_sprites(
        &self,
        blocks: &[horizon_world::registry::BlockStateId],
    ) -> Vec<Option<BlockSprites>> {
        let lookup = self.native_sprite_lookup();
        blocks
            .iter()
            .map(|state| {
                self.pack.context.assets.registry.state(*state)?;
                Some(lookup.state(*state))
            })
            .collect()
    }
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`), none for one no
    /// model uses. API 380.
    pub(super) fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>> {
        let lookup = self.native_sprite_lookup();
        textures
            .iter()
            .map(|texture| lookup.texture(texture))
            .collect()
    }
    /// For a client mod drawing a block entity: the texture block state `state`'s model shows on its `side`
    /// (`block/oak_planks`: vanilla's without their namespace, a mod's with it), by the name `native_draw_cube` and
    /// `native_draw_quad` take, and that face's
    /// tint index (-1 for none): a copycat block drawing a box of its material's textures with `native_draw_cube`,
    /// kept with `BlockEntityDraw::keep`. The face culled against that side comes first, else the first facing it;
    /// None when the model has no face that way or isn't baked. API @API:block_face_sprite@.
    pub(super) fn block_face_sprite(
        &self,
        state: BlockStateId,
        side: Direction,
    ) -> Option<BlockFaceTexture> {
        let wanted = horizon_assets::model::Direction::ALL[side as usize];
        let models = self
            .pack
            .render
            .model_catalog
            .collect_seeded(state, 42)
            .ok()?;
        let faces = || models.iter().flat_map(|(_, model)| &model.faces);
        let face = faces()
            .find(|f| f.quad.cull == Some(wanted))
            .or_else(|| faces().find(|f| f.quad.facing == wanted))?;
        let name = self.pack.layer_names.get(&face.layer)?.clone();
        Some(mod_api::BlockFaceTexture::new(name, face.quad.tint_index))
    }
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `block_sprites` again then. API 380.
    pub(super) fn block_sprites_version(&self) -> u64 {
        self.pack.context.render.generation
    }
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// (a server's metrics-only atlas lacks them all). The atlas names vanilla's sprites by path alone
    /// (`gui/sprites/container/slot`) and a mod's with its namespace (`mymod:gui/panel`).
    pub(super) fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion> {
        let r = if sprite.namespace() == "minecraft" {
            self.gui.atlas.sprites.get(sprite.path())
        } else {
            self.gui.atlas.sprites.get(&sprite.to_string())
        }?;
        Some(AtlasRegion::new([r.u0, r.v0, r.u1, r.v1], [r.w, r.h]))
    }
    fn native_sprite_lookup(&self) -> Lookup<'_> {
        // (no renderer, no arrays: every layer counts as array 0's)
        let split = self
            .renderer
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split());
        Lookup::new(&self.pack.context, split)
    }
}

#[cfg(test)]
mod texture_atlas_stitched_tests {
    use mod_api::{
        AtlasRegion, ClientHost, Hooks, Identifier, ModEntry, RegisterRuntimeSpritesEvent,
        RegistrationContext, RuntimeSprite, SpriteRef, TextureAtlas, TextureAtlasStitchedEvent,
        testing::with_mods,
    };
    use std::sync::{Arc, Mutex};

    /// What the mod kept: each atlas it heard, and one sprite of each, looked up once there.
    #[derive(Default)]
    struct Kept {
        atlases: Vec<TextureAtlas>,
        stone: Option<SpriteRef>,
        slot: Option<AtlasRegion>,
        registrations: u32,
        runtime_block: Option<SpriteRef>,
        runtime_gui: Option<AtlasRegion>,
    }
    struct Keeper(Arc<Mutex<Kept>>);
    #[mod_api::__subscriptions]
    impl Hooks for Keeper {
        fn register_runtime_sprites(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterRuntimeSpritesEvent<'_>,
        ) {
            use crate::game::client_reload::fixture_png;
            let mut kept = self.0.lock().unwrap();
            kept.registrations += 1;
            if kept.registrations == 1 {
                event.sprites.push(RuntimeSprite {
                    atlas: TextureAtlas::Blocks,
                    id: id("keeper:block/marker"),
                    png: fixture_png(2, 2, 32),
                });
            }
            event.sprites.push(RuntimeSprite {
                atlas: TextureAtlas::Gui,
                id: id("keeper:gui/marker"),
                png: fixture_png(kept.registrations + 2, 3, 64),
            });
        }

        fn texture_atlas_stitched(
            &self,
            host: &mut dyn ClientHost,
            event: &mut TextureAtlasStitchedEvent,
        ) {
            let mut kept = self.0.lock().unwrap();
            kept.atlases.push(event.atlas);
            match event.atlas {
                TextureAtlas::Blocks => {
                    kept.stone = host.texture_sprites(&["minecraft:block/stone"])[0];
                    kept.runtime_block = host.texture_sprites(&["keeper:block/marker"])[0];
                }
                TextureAtlas::Gui => {
                    kept.slot = host.gui_sprite(&id("gui/sprites/container/slot"));
                    kept.runtime_gui = host.gui_sprite(&id("keeper:gui/marker"));
                }
                _ => {}
            }
        }
    }

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    /// `TextureAtlasStitchedEvent`: the mod hears the blocks' atlas and then the GUI's, finds a
    /// sprite of each, and hears both again after a resource reload.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_mod_finds_its_sprites_each_time_an_atlas_is_stitched() {
        let kept = Arc::new(Mutex::new(Kept::default()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("keeper", Box::new(Keeper(Arc::clone(&kept))))].into();
        with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                // The first load, as `run` fires it before `client_setup`.
                game.texture_atlases_stitched();
                {
                    let kept = kept.lock().unwrap();
                    assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                    assert!(kept.stone.is_some(), "stone is in the block arrays");
                    assert!(
                        kept.runtime_block.is_some(),
                        "no model references this block sprite"
                    );
                    assert_eq!(kept.runtime_gui.map(|sprite| sprite.size), Some([3, 3]));
                    let slot = kept.slot.expect("the slot sprite is in the GUI atlas");
                    assert_eq!(slot.size, [18, 18], "the slot's own pixels");
                    let [u0, v0, u1, v1] = slot.uv;
                    assert!(0.0 <= u0 && u0 < u1 && u1 <= 1.0, "{:?}", slot.uv);
                    assert!(0.0 <= v0 && v0 < v1 && v1 <= 1.0, "{:?}", slot.uv);
                }
                // A mod's namespace is part of the name; one the atlas lacks is none.
                assert_eq!(
                    game.gui_sprite(&id("mymod:gui/sprites/container/slot")),
                    None
                );
                assert_eq!(game.gui_sprite(&id("gui/no_such_sprite")), None);
                // F3+T: the reload stages on a worker and applies here.
                kept.lock().unwrap().atlases.clear();
                game.spawn_pack_reload(super::super::packs::ReloadKind::Selection)
                    .unwrap();
                let started = std::time::Instant::now();
                while game.pack_reload.is_some() {
                    assert!(
                        started.elapsed().as_secs() < 300,
                        "the reload never finished"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    game.drive_pack_reload();
                }
                let kept = kept.lock().unwrap();
                assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                assert_eq!(kept.registrations, 2);
                assert!(
                    kept.runtime_block.is_none(),
                    "omitted sprite leaves the new atlas"
                );
                assert_eq!(kept.runtime_gui.map(|sprite| sprite.size), Some([4, 3]));
                assert_eq!(
                    kept.slot.map(|s| s.size),
                    Some([18, 18]),
                    "looked up again in the new atlas"
                );
            });
        });
    }
}
