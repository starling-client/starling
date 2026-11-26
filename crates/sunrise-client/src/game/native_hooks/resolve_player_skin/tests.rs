use super::*;
use crate::game::{
    Screen,
    native_entity_refs::tests::{run_fresh_cases, with_client_game},
    native_hooks::particle_count_tests::frame_scene,
};
use mod_api::{ClientHost, Hooks, Identifier, ModEntry, ResolvePlayerSkinEvent};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};
use sunrise_client::{
    camera::CameraType,
    crash::{ModCall, enter_native_mod},
    hud::Rect,
};

#[derive(Default)]
struct Choice {
    selection: Option<PlayerSkinOverride>,
    calls: usize,
}

struct SkinPicker(Arc<Mutex<Choice>>);

#[mod_api::__subscriptions]
impl Hooks for SkinPicker {
    fn resolve_player_skin(&self, _: &dyn ClientHost, event: &mut ResolvePlayerSkinEvent<'_>) {
        assert!(
            event.selection.is_none(),
            "each frame starts with no selection"
        );
        let mut choice = self.0.lock().unwrap();
        choice.calls += 1;
        event.selection.clone_from(&choice.selection);
    }
}

/// The atlas sprite a selected dynamic texture is copied into: its skin slot (placing it is
/// idempotent).
fn skin_slot_name(game: &mut crate::game::Game<'_>, texture: &Identifier) -> String {
    let (slot, _) = game
        .skin_slots
        .place(format!("dynamic:{texture}"))
        .expect("a skin slot");
    format!("sunrise/skin{slot}")
}

#[test]
#[ignore = "daily"]
fn live_skin_selection_reaches_the_hand_body_and_preview_and_restores_vanilla() {
    if run_fresh_cases(
        "game::native_hooks::resolve_player_skin::tests::live_skin_selection_reaches_the_hand_body_and_preview_and_restores_vanilla",
        "SUNRISE_SKIN_SELECTION_CHILD",
        &["live"],
    ) {
        return;
    }
    let choice = Arc::new(Mutex::new(Choice::default()));
    with_mods(
        vec![ModEntry::new(
            "picker",
            Box::new(SkinPicker(choice.clone())),
        )]
        .into(),
        || {
            with_client_game(|game| {
                let _event_loop = frame_scene(game);
                game.tick_player_model();
                let avatar = game.player_model.as_mut().unwrap();
                let vanilla = "entity/player/wide/steve";
                avatar.skin = Some(vanilla.into());
                avatar.slim = false;

                let wide = Identifier::parse("picker:wide").unwrap();
                let slim = Identifier::parse("picker:slim").unwrap();
                let wrong_size = Identifier::parse("picker:wrong_size").unwrap();
                let edit_texture = || {
                    enter_native_mod(ModCall {
                        id: "picker",
                        hook: "client_tick_post",
                        part: None,
                    })
                };
                {
                    let _call = edit_texture();
                    game.create_dynamic_texture(
                        &wide,
                        64,
                        64,
                        1,
                        &[255, 0, 0, 255].repeat(64 * 64),
                    )
                    .unwrap();
                    game.create_dynamic_texture(
                        &slim,
                        64,
                        64,
                        1,
                        &[0, 0, 255, 255].repeat(64 * 64),
                    )
                    .unwrap();
                    game.create_dynamic_texture(&wrong_size, 32, 64, 1, &[255; 32 * 64 * 4])
                        .unwrap();
                }

                // A complete frame must sample the chosen sprite, not merely expose the answer
                // from the hook. Parts outside the sprite do not count (armor, items, GUI chrome).
                let draw = |game: &mut crate::game::Game<'_>, key: &str| {
                    let Rect { u0, v0, u1, v1, .. } = game.gui.atlas.sprites[key];
                    let inside = |[u, v]: [f32; 2]| u0 <= u && u < u1 && v0 <= v && v < v1;
                    let mut hand = Vec::new();
                    let mut body = false;
                    let mut preview = false;
                    let before_calls = choice.lock().unwrap().calls;
                    let rendered = game
                        .render_with(|_, frame| {
                            hand.extend(frame.hand.iter().filter(|v| inside(v.uv)).map(|v| v.pos));
                            body = frame.entities.iter().any(|v| inside(v.uv));
                            preview = frame.hud.vertices.iter().any(|v| inside(v.uv));
                            Ok(true)
                        })
                        .unwrap();
                    assert!(rendered);
                    assert_eq!(choice.lock().unwrap().calls, before_calls + 1);
                    let avatar = game.player_model.as_ref().unwrap();
                    assert_eq!(avatar.skin.as_deref(), Some(vanilla));
                    assert!(!avatar.slim, "the selected width must not escape the frame");
                    (hand, body, preview)
                };

                let vanilla_hand = draw(game, vanilla).0;
                assert!(!vanilla_hand.is_empty(), "the vanilla arm draws");
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(wide.clone(), PlayerModel::Wide));
                let wide_name = skin_slot_name(game, &wide);
                assert_eq!(draw(game, &wide_name).0, vanilla_hand);

                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(slim.clone(), PlayerModel::Slim));
                let slim_name = skin_slot_name(game, &slim);
                let slim_hand = draw(game, &slim_name).0;
                assert!(!slim_hand.is_empty(), "the new texture draws on the arm");
                assert_ne!(
                    slim_hand, vanilla_hand,
                    "the slim arm has different geometry"
                );

                game.apply_camera_type(CameraType::ThirdPersonBack);
                game.screen = Screen::Inventory;
                let (_, body, preview) = draw(game, &slim_name);
                assert!(body, "the world avatar uses the selection");
                assert!(preview, "the inventory avatar uses that same selection");

                game.apply_camera_type(CameraType::FirstPerson);
                game.screen = Screen::Playing;
                choice.lock().unwrap().selection = None;
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "clearing restores vanilla"
                );
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(wrong_size, PlayerModel::Slim));
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "wrong size keeps both values"
                );
                choice.lock().unwrap().selection =
                    Some(PlayerSkinOverride::new(slim.clone(), PlayerModel::Slim));
                {
                    let _call = edit_texture();
                    game.release_dynamic_texture(&slim).unwrap();
                }
                assert_eq!(
                    draw(game, vanilla).0,
                    vanilla_hand,
                    "freed id keeps both values"
                );
            });
        },
    );
}
