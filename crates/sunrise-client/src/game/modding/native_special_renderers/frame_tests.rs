//! Special item renderers in the real game: dropped-item draw and outline submission, and when
//! the renderer is called (on the client's frame, outside lifted passes and action transactions).
//! Each test runs in its own process and changes the game's stick into a special model.
use crate::game::Game;
use mod_api::{
    ClientHost, SpecialModelCollector, SpecialModelContext, SpecialModelFactory,
    SpecialModelGeometry, SpecialModelQuad, SpecialModelRenderer, SpecialModelStack,
    SpecialModelTints,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::item_render::{ItemRenderState, OwnedSpecialModelFactory};

/// One directly submitted quad goes through the real dropped-item draw and its outline copy.
#[test]
#[ignore = "daily"]
fn a_special_dropped_item_outlines_only_its_committed_world_vertices() {
    use sunrise_client::{
        chunk::{Chunk, Light, index},
        entities::ItemEntity,
        hud::Rect,
        item_render::GlintFrame,
        items::ItemStack,
        mesher::Vertex,
    };
    use sunrise_core::{
        geometry::Vec3,
        nbt::{Compound, Tag},
    };

    crate::game::in_own_process(
        "game::native_special_renderers::frame_tests::a_special_dropped_item_outlines_only_its_committed_world_vertices",
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                install_special_stick(game);
                game.gui.atlas.sprites.insert(
                    "block/stone".into(),
                    Rect {
                        u0: 0.0,
                        v0: 0.0,
                        u1: 1.0,
                        v1: 1.0,
                        w: 16,
                        h: 16,
                    },
                );
                game.glint_frame = GlintFrame {
                    millis: 0.0,
                    speed: 0.5,
                    alpha: 0.75,
                    sprite: true,
                };

                // Light the eye's cell (y = 65), not the feet's cell (y = 64).
                let mut chunk = Chunk::empty(0, 0);
                let section = chunk.section_index(65).unwrap();
                let mut light = Box::new([0; 4096]);
                light[index(0, 65, 0)] = 0xA5;
                Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(light);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                const ID: u32 = 1_900_000_101;
                game.remember_tags(
                    ID,
                    &Tag::Compound(Compound::from([("Glowing".into(), Tag::Byte(1))])),
                );
                let mut stick = ItemStack::new("minecraft:stick", 1);
                stick
                    .data_mut()
                    .set_other("minecraft:enchantment_glint_override", Some(Tag::Byte(1)));
                game.dropped.push(ItemEntity::new(
                    ID,
                    stick,
                    Vec3::new(0.5, 64.875, 0.5),
                    Vec3::ZERO,
                    0.0,
                ));
                let last = game.dropped.len() - 1;

                // Pre-existing geometry is not part of the glowing item's silhouette.
                let prefix = Vertex {
                    pos: [2.0, 3.0, 4.0],
                    uv: [0.0; 2],
                    layer: 0,
                    color: 0xFF00_00FF,
                    light: 0,
                    normal: [0; 4],
                    glint_uv: [0.0; 2],
                };
                let mut vertices = vec![prefix];
                game.dropped_item_geometry(
                    glam::DVec3::new(0.0, 64.0, 0.0),
                    0.0,
                    &mut vertices,
                    last..last + 1,
                );
                assert_eq!(vertices.len(), 5, "one committed quad plus the prefix");
                assert_eq!(
                    bytemuck::bytes_of(&vertices[0]),
                    bytemuck::bytes_of(&prefix)
                );
                let drawn = &vertices[1..];
                assert!(drawn.iter().all(|v| v.light == 0x50A0)); // block 5, sky 10
                assert!(drawn.iter().all(|v| v.layer & 0xFFFF == 7)); // solid foil overlay
                assert_ne!(drawn[1].glint_uv, [0.0; 2]);

                let outline = game.glowing_vertices.borrow();
                assert_eq!(outline.len(), 4);
                for (outline, vertex) in outline.iter().zip(drawn) {
                    assert_eq!(outline.pos, vertex.pos);
                    assert_eq!(outline.uv, vertex.uv);
                    assert_eq!(outline.light, 0x50A0);
                    assert_eq!(outline.color, 0xFFFF_FFFF); // unteamed glowing item
                    assert_eq!(outline.normal, [0; 4]);
                }
            });
        },
    );
}

/// The stack counts the renderer was asked about, in order.
static ASKED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Draws a stick for every stack.
struct Stick(SpecialModelGeometry);
impl SpecialModelRenderer for Stick {
    type Argument = u32;

    fn extract_argument(&self, _: &dyn ClientHost, stack: &SpecialModelStack<'_>) -> u32 {
        stack.count()
    }

    fn submit(
        &self,
        count: &u32,
        _: &SpecialModelContext,
        collector: &mut SpecialModelCollector<'_>,
    ) -> Result<(), String> {
        ASKED.lock().unwrap().push(*count);
        collector.submit_geometry(&self.0, SpecialModelTints::Untinted)
    }
}

/// How many times the renderer was asked about a stack of `count`.
pub(in crate::game) fn asked(count: u32) -> usize {
    ASKED
        .lock()
        .unwrap()
        .iter()
        .filter(|&&c| c == count)
        .count()
}

/// Installs a special renderer for sticks in this game's item definitions. The fixture submits
/// its own quad and needs no display transforms from a client-only model library.
pub(in crate::game) fn install_special_stick(game: &mut Game<'_>) {
    let factories = BTreeMap::from([(
        mod_api::Identifier::parse("test:stick").unwrap(),
        OwnedSpecialModelFactory::new(
            "test",
            SpecialModelFactory::new(
                |_| Ok(()),
                |(), bake| {
                    bake.bake_geometry(&[SpecialModelQuad::new(
                        [
                            [0.0, 0.0, 0.0],
                            [1.0, 0.0, 0.0],
                            [1.0, 1.0, 0.0],
                            [0.0, 1.0, 0.0],
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                        "minecraft:block/stone",
                    )])
                    .map(Stick)
                },
            ),
        ),
    )]);
    let definition = game.pack.item_renderer.decode_definition(
        r#"{"model":{"type":"minecraft:special","base":"minecraft:builtin/missing","model":{"type":"test:stick"}}}"#,
        &sunrise_client::item_render::ItemProviderTables {
            special: Some(&factories),
            ..Default::default()
        },
    ).unwrap();
    Arc::get_mut(&mut game.pack.item_renderer)
        .expect("the game's item renderer is shared")
        .models
        .definitions
        .insert("minecraft:stick".into(), definition);
}

/// A stick of `count` resolved as a frame resolves a held item (`ItemModelResolver.updateForTopItem`).
pub(in crate::game) fn draw_stick(game: &Game<'_>, count: u32) -> ItemRenderState {
    game.resolve_item(
        &sunrise_client::items::ItemStack::new("minecraft:stick", count),
        DisplayContext::ThirdPersonRightHand,
        None,
        0,
    )
}

/// What the renderer had been asked about a stack of [`COUNT`] when the tick's mob pass was
/// lifted: set from inside the pass.
static ASKED_IN_PASS: Mutex<Option<usize>> = Mutex::new(None);
const COUNT: u32 = 6;

/// Row 6: the renderer and the game's special stick are in place, and a frame's resolve asks
/// the renderer (the positive control). A tick then lifts its mob pass and ticks a zombie
/// holding that stick, and the renderer is not asked, inside the pass or after it. Item models
/// resolve only on the client's frame, never inside a tick's lifted pass or an action
/// transaction.
#[test]
#[ignore = "daily"]
fn a_special_renderer_is_never_asked_inside_a_lifted_pass() {
    crate::game::in_own_process(
        "game::native_special_renderers::frame_tests::a_special_renderer_is_never_asked_inside_a_lifted_pass",
        a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated,
    );
}

fn a_special_renderer_is_never_asked_inside_a_lifted_pass_isolated() {
    fn in_pass(_: &mut Game<'_>) {
        assert!(
            !sunrise_client::join_seq::visibility::passes_settled(),
            "not inside a lifted pass"
        );
        *ASKED_IN_PASS.lock().unwrap() = Some(asked(COUNT));
    }
    crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
        const ZOMBIE: u32 = 1_900_000_100;
        install_special_stick(game);
        let before = asked(COUNT);
        draw_stick(game, COUNT);
        let asked_by_the_frame = asked(COUNT);
        assert_eq!(
            asked_by_the_frame,
            before + 1,
            "the frame's resolve didn't ask"
        );
        balance.drain_balanced();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = sunrise_client::mobs::Mob::new(
            ZOMBIE,
            kind,
            sunrise_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            0.0,
        );
        mob.no_ai = true;
        mob.extra.equipment.set(
            sunrise_items::equipment::MAINHAND,
            Some(sunrise_client::items::ItemStack::new(
                "minecraft:stick",
                COUNT,
            )),
        );
        game.mobs.push(mob);
        if !game.world.chunks.contains_key(&(0, 0)) {
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
        }
        crate::game::entity::taken_pass_tests::with_spawn(in_pass, || {
            game.tick_creature_entities().unwrap();
        });
        assert!(balance.drain_balanced() > 0, "the tick lifted no mob pass");
        let zombie = game.mobs.iter().find(|m| m.id == ZOMBIE).unwrap();
        assert_eq!(zombie.age, 1, "the zombie didn't tick");
        assert_eq!(
            ASKED_IN_PASS.lock().unwrap().take(),
            Some(asked_by_the_frame),
            "the renderer was asked before the pass, inside the tick"
        );
        assert_eq!(
            asked(COUNT),
            asked_by_the_frame,
            "the renderer was asked in the tick"
        );
        game.mobs.clear();
    });
}
