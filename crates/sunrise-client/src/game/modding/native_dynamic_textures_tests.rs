#![cfg(test)]
//! A client mod's dynamic textures (`ClientRenderClientHost::create_dynamic_texture`, #1343):
//! the atlas's pool follows the loaded client mods, with no mixin (#4437), and
//! an image created as the mod whose hook runs draws under its id through `native_sprite` on
//! the real HUD pipeline, until it is freed.
use super::resource_tables::dynamic_texture_pool;
use mod_api::{DynamicTextureError, Hooks, Identifier, ModEntry};
use mod_platform::testing::with_mods;
use sunrise_client::{
    crash::{ModCall, enter_native_mod},
    gfx::entity_pixel_test::render_hud,
    hud::HudBatch,
};

const SIZE: [u32; 2] = [64, 64];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A mod with no hook and no mixin: it only uses the API.
struct ApiOnly;

#[mod_api::__subscriptions]
impl Hooks for ApiOnly {}

#[test]
fn the_pool_follows_the_loaded_client_mods() {
    assert!(
        !with_mods(Vec::new().into(), dynamic_texture_pool),
        "no mod"
    );
    let native = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    assert!(
        with_mods(native.into(), dynamic_texture_pool),
        "a native mod with no mixin"
    );
}

/// `f` run as mod `id`'s client tick.
fn as_mod<R>(id: &'static str, f: impl FnOnce() -> R) -> R {
    let _call = enter_native_mod(ModCall {
        id,
        hook: "client_tick_post",
        part: None,
    });
    f()
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

/// The game's own atlas, built with a native mod that has no mixin loaded, has the pool; the
/// mod's image goes into a slot with the frame's patches and draws by its id on the real HUD
/// pipeline, out of another mod's reach; once freed, its id names no sprite.
#[test]
#[ignore = "daily"]
fn a_mods_dynamic_texture_draws_under_its_id_until_it_is_freed() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            assert!(game.gui.atlas.dynamic_texture_pool.is_some());
            let id = Identifier::parse("api-only:dynamic/panel").unwrap();
            let name = "api-only:dynamic/panel";
            let green = GREEN.repeat(16 * 16);
            as_mod("api-only", || {
                game.create_dynamic_texture(&id, 16, 16, &green)
            })
            .unwrap();
            assert_eq!(
                as_mod("other", || game.free_dynamic_texture(&id)),
                Err(DynamicTextureError::NotOwned)
            );
            let mut batch = HudBatch::scratch(SIZE);
            game.dynamic_texture_patches(&mut batch.image_patches);
            assert_eq!(batch.image_patches.len(), 1, "one patch for the new image");
            let (src, dst) = ([0.0, 0.0, 16.0, 16.0], [16.0, 16.0, 32.0, 32.0]);
            assert!(game.native_sprite(&mut batch, name, src, dst, None));
            let pixels = render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan draw");
            assert_eq!(pixel(&pixels, 32, 32), GREEN, "the mod's image");
            assert_eq!(pixel(&pixels, 4, 4), BLACK, "outside the sprite");
            as_mod("api-only", || game.free_dynamic_texture(&id)).unwrap();
            let mut batch = HudBatch::scratch(SIZE);
            assert!(!game.native_sprite(&mut batch, name, src, dst, None));
        });
    });
}

/// A mod's live texture at an id of another namespace is `NotOwned`, even where the GUI atlas
/// lists the id (vanilla's `container/slot`): the owner check comes first.
#[test]
#[ignore = "daily"]
fn a_foreign_id_is_not_owned_before_the_gui_atlas_is_asked() {
    let mods = vec![ModEntry::new("api-only", Box::new(ApiOnly))];
    with_mods(mods.into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let green = GREEN.repeat(16 * 16);
            let make = |game: &mut crate::game::Game, id: &str| {
                let id = Identifier::parse(id).unwrap();
                as_mod("api-only", || {
                    game.create_dynamic_texture(&id, 16, 16, &green)
                })
            };
            assert_eq!(
                make(game, "minecraft:container/slot"),
                Err(DynamicTextureError::NotOwned)
            );
            assert_eq!(make(game, "api-only:dynamic/free"), Ok(()));
        });
    });
}
