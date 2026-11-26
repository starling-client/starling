//! Recipe lookup, placement, creative gives, and client synchronization. The client uses its
//! own recipes, with server-synced types taking precedence, and follows the server's book for
//! known displays.
use super::{
    Game,
    api_values::{TryIntoApi, TryIntoEngine},
};
use mod_api::{CreativeGiveError, LevelRef, RecipeInput, RecipeLookupError, RecipeMatch};
use std::sync::Arc;
use sunrise_client::items::ItemStack;

/// The most recipes a server's recipe content may carry.
const MAX_SYNCED_RECIPES: usize = 1_000_000;

/// The parts of one recipe update so far. A part whose serializer could not
/// be decoded refuses the whole update, once, when its last part comes: the
/// parts after it must not publish what is left, and the previous generation
/// stays.
#[derive(Default)]
struct IncomingRecipes {
    recipes: Vec<(String, serde_json::Value)>,
    refused: Option<String>,
}
/// A completed update: the recipes and the types they replace, or why it is
/// refused.
type RecipeUpdate = Result<(Vec<(String, serde_json::Value)>, Vec<String>), String>;
impl IncomingRecipes {
    /// Take one part; the finished update on the last.
    fn push(&mut self, part: sunrise_client::lan::recipes::ContentPart) -> Option<RecipeUpdate> {
        if self.refused.is_none() {
            self.refused = part.refused;
        }
        if self.refused.is_none() {
            self.recipes.extend(part.recipes);
            // The server is trusted to end its parts; one that never does is
            // dropped, not kept growing.
            if self.recipes.len() > MAX_SYNCED_RECIPES {
                self.recipes = Vec::new();
                self.refused = Some(format!(
                    "more than {MAX_SYNCED_RECIPES} recipes; the rest is dropped"
                ));
            }
        }
        let types = part.last?;
        let recipes = std::mem::take(&mut self.recipes);
        Some(match self.refused.take() {
            Some(why) => Err(why),
            None => Ok((recipes, types)),
        })
    }
}

#[derive(Default)]
pub(super) struct RecipeTables {
    pub(super) books: Option<
        Result<
            sunrise_client::recipe_book::ModBookIndex,
            sunrise_client::recipe_book::DisplayLoadError,
        >,
    >,
    known: (Option<u64>, Arc<[i32]>),
    /// The server's recipes of the types its native mods sent (`sunrise:
    /// recipe_content`) over the window's own: what `Game::recipes` reads on
    /// a client once they came.
    synced: Option<sunrise_client::crafting::Recipes>,
    /// The display numbers of those recipes: the server numbers its book's
    /// displays over its own recipe set, a world's data pack recipes
    /// included, so the book numbers them the same (`Game::display_index`).
    pub(super) synced_index: Option<sunrise_client::recipe_book::RecipeIndex>,
    /// The recipe content's parts so far, until its last.
    incoming: IncomingRecipes,
    /// The types that came before the player did: the mods hear of them
    /// once the helpers can answer (`recipes_received_now`).
    received: Option<Vec<String>>,
}

#[cfg(test)]
impl RecipeTables {
    pub(in crate::game) fn install_test_recipes(
        &mut self,
        recipes: sunrise_client::crafting::Recipes,
    ) {
        self.synced = Some(recipes);
    }
}

/// A stack's components beyond its item's, as vanilla's item syntax writes
/// them (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`), or "".
pub(super) fn components(s: &ItemStack) -> String {
    use sunrise_core::nbt::Tag;
    let Some(Tag::Compound(entries)) = sunrise_client::item_data::component_snapshot(s) else {
        return String::new();
    };
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.to_string_lossy(),
                sunrise_client::snbt::to_snbt(v)
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

impl Game<'_> {
    /// The engine's matchers, shared with crafting and station menus; no mod-side rematching.
    pub(super) fn level_recipe_for(
        &self,
        level: LevelRef,
        recipe_type: &mod_api::Identifier,
        inputs: &RecipeInput,
    ) -> Result<Option<RecipeMatch>, RecipeLookupError> {
        use RecipeLookupError::*;
        self.native_level_world(level).ok_or(UnavailableLevel)?;
        if recipe_type.namespace() != "minecraft" {
            return Err(NotMatchable);
        }
        let kind = recipe_type.path();
        let make = |id: &str, output: &ItemStack, remainders, experience, processing_time| {
            Ok(Some(RecipeMatch::new(
                mod_api::Identifier::parse(id).map_err(|_| InvalidRecipe)?,
                output.try_into_api().map_err(|_| InvalidItem)?,
                remainders,
                experience,
                processing_time,
            )))
        };
        let read_stack = |stack: &mod_api::ItemStack| {
            super::api_values::recipe_inputs::stack(stack, self.items()).map_err(|_| InvalidItem)
        };
        match kind {
            "crafting" => {
                let RecipeInput::Crafting {
                    width,
                    height,
                    slots,
                } = inputs
                else {
                    return Err(InvalidInput);
                };
                let (w, h) = (*width as usize, *height as usize);
                if w.checked_mul(h) != Some(slots.len()) {
                    return Err(InvalidInput);
                }
                let mut grid = Vec::with_capacity(slots.len());
                for stack in slots {
                    #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                    {
                        grid.push(stack.as_ref().map(&read_stack).transpose()?);
                    }
                }
                let scale = |id: i32| self.maps.data.get(&id).map(|m| m.scale);
                let recipes = self.recipes();
                let matched =
                    recipes.find_with_dimensions(&self.loop_context, &grid, w, w, h, &scale);
                let Some((id, output)) = matched.filter(|(_, output)| output.count > 0) else {
                    return Ok(None);
                };
                let remainders = recipes
                    .remainders_rect(&self.loop_context, id, &grid, w, w, h, &|s| {
                        self.crafting_remainder(s)
                    })
                    .into_iter()
                    .map(|stack| stack.try_into_api().map_err(|_| InvalidItem))
                    .collect::<Result<Vec<_>, _>>()?;
                make(id, &output, remainders, 0.0, 0)
            }
            "smelting" | "blasting" | "smoking" | "campfire_cooking" => {
                let RecipeInput::Single(input) = inputs else {
                    return Err(InvalidInput);
                };
                let input = read_stack(input)?;
                let full_kind = match kind {
                    "smelting" => "minecraft:smelting",
                    "blasting" => "minecraft:blasting",
                    "smoking" => "minecraft:smoking",
                    _ => "minecraft:campfire_cooking",
                };
                let Some(recipe) = self.recipes().cook(&self.loop_context, full_kind, &input)
                else {
                    return Ok(None);
                };
                make(
                    &recipe.id,
                    &recipe.result,
                    vec![None],
                    recipe.experience,
                    recipe.time,
                )
            }
            "stonecutting" => {
                let RecipeInput::Single(input) = inputs else {
                    return Err(InvalidInput);
                };
                let input = read_stack(input)?;
                let Some(recipe) = self
                    .stations()
                    .recipes
                    .stonecutting
                    .iter()
                    .find(|r| r.input.test_in(&self.loop_context, &input))
                else {
                    return Ok(None);
                };
                make(&recipe.id, &recipe.result, vec![None], 0.0, 0)
            }
            "smithing" => {
                let RecipeInput::Smithing {
                    template,
                    base,
                    addition,
                } = inputs
                else {
                    return Err(InvalidInput);
                };
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let template = template.as_ref().map(&read_stack).transpose()?;
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let base = base.as_ref().map(&read_stack).transpose()?;
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let addition = addition.as_ref().map(&read_stack).transpose()?;
                let stations = self.stations();
                let Some((id, output)) = stations.recipes.smithing_recipe(
                    &self.loop_context,
                    &stations.enchantments,
                    template.as_ref(),
                    base.as_ref(),
                    addition.as_ref(),
                ) else {
                    return Ok(None);
                };
                make(id, &output, vec![None; 3], 0.0, 0)
            }
            _ => Err(NotMatchable),
        }
    }
    pub(super) fn registered_recipe_book_category(
        &mut self,
        kind: &str,
    ) -> Option<mod_api::item::recipe_book::RecipeBookCategory> {
        self.ensure_mod_book_displays();
        self.recipe_tables
            .books
            .as_ref()?
            .as_ref()
            .ok()?
            .categories()
            .find(|category| category.recipe_type == kind)
            .cloned()
    }
    /// `place_recipe`: refused as `recipes.place` refuses it.
    pub(super) fn recipe_place(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        if !self.mod_recipe_screen() {
            return Err("no screen that takes recipes is open".into());
        }
        if !self.known_displays().contains(&book_id) {
            return Err(format!(
                "recipe {book_id} is not in the player's recipe book"
            ));
        }
        self.mod_place_recipe(book_id, all);
        Ok(())
    }
    /// `give_creative`: `Inventory.add` of `stack` for a creative player, components kept and up
    /// to the stack's own maximum, then `MultiPlayerGameMode.handleCreativeModeItemAdd` for
    /// every slot it changed, sent now: the end of the tick's sync skips creative slots while
    /// a server menu is open. What did not fit comes back and is not dropped.
    pub(super) fn creative_give_stack(
        &mut self,
        stack: &mod_api::ItemStack,
    ) -> Result<Option<mod_api::ItemStack>, CreativeGiveError> {
        if self.is_logical_server() || self.player.is_none() || self.res.rest.get().is_none() {
            return Err(CreativeGiveError::Unavailable);
        }
        if !self.creative {
            return Err(CreativeGiveError::NotCreative);
        }
        let mut stack = stack.try_into_engine(self.items())?;
        if self.remote.is_some()
            && !self
                .creative_screen
                .catalog()
                .item_enabled(&stack, self.enabled_feature_ids())
        {
            return Err(CreativeGiveError::FeatureDisabled);
        }
        let max = self.stack_max_size(&stack);
        let inventory = self.inventory[..36].to_vec();
        let offhand = self.offhand.clone();
        self.insert_stack_up_to(&mut stack, max);
        for i in 0..36 {
            if self.inventory[i] != inventory[i] {
                self.remote_creative_slot_add(i);
            }
        }
        if self.offhand != offhand {
            self.remote_creative_slot_add(36);
        }
        if stack.count == 0 {
            return Ok(None);
        }
        stack
            .try_into_api()
            .map(Some)
            .map_err(|error| error.into_write_error().into())
    }
    /// The `book_id`s the player's recipe book holds now.
    pub(super) fn known_displays(&mut self) -> Arc<[i32]> {
        if self.res.rest.get().is_none() {
            return Arc::from([]);
        }
        let (version, known) = self.recipe_book_displays();
        if self.recipe_tables.known.0 != Some(version) {
            self.recipe_tables.known = (Some(version), known.into());
        }
        self.recipe_tables.known.1.clone()
    }

    /// `RecipesClientHost::recipe_book_ids`: the display ids the recipe book
    /// gives `recipe_id`, as `transfer_recipe` looks them up.
    pub(in crate::game) fn recipe_book_ids(&mut self, recipe_id: &str) -> Arc<[i32]> {
        if self.player.is_none() || self.res.rest.get().is_none() {
            return Arc::from([]);
        }
        self.viewer_book_ids(recipe_id)
    }
}

impl Game<'_> {
    /// The server's recipes a client was sent, over its own; None on a
    /// server and until a server sends any.
    pub(super) fn synced_recipes(&self) -> Option<&sunrise_client::crafting::Recipes> {
        self.recipe_tables.synced.as_ref()
    }
    /// The display numbers the recipe book reads: the server's recipes',
    /// once a server sent them, else the world's own.
    pub(super) fn display_index(&self) -> &sunrise_client::recipe_book::RecipeIndex {
        match &self.recipe_tables.synced_index {
            Some(index) => index,
            None => &self.stations().recipe_index,
        }
    }
    /// A new play session (`Login`): what the last server sent is gone.
    pub(super) fn forget_synced_recipes(&mut self) {
        self.recipe_tables.synced = None;
        self.recipe_tables.synced_index = None;
        self.recipe_tables.incoming = IncomingRecipes::default();
        self.recipe_tables.received = None;
        self.forget_book();
    }
    /// `native_recipes_received` for the types that came, once the window
    /// has its player: the recipes of a join come before it, and the helpers
    /// answer nothing until then. Once a frame.
    pub(super) fn recipes_received_now(&mut self) {
        if self.player.is_some()
            && let Some(types) = self.recipe_tables.received.take()
        {
            self.native_recipes_received(&types);
            self.build_screen();
        }
    }
    /// A `sunrise:recipe_content` part the server sent the window. On the
    /// last, the window's tables take the server's recipes of those types in
    /// place of its own, and the mods hear of it (`RecipesReceivedEvent`).
    pub(super) fn remote_recipe_content(&mut self, data: &[u8]) {
        let part = match sunrise_client::lan::recipes::ContentPart::decode(data) {
            Ok(part) => part,
            Err(e) => {
                log::warn!("Sunrise: bad recipe content from the server: {e}");
                return;
            }
        };
        let (received, types) = match self.recipe_tables.incoming.push(part) {
            None => return,
            Some(Ok(update)) => update,
            Some(Err(why)) => {
                log::warn!("Sunrise: the server's recipes were not taken: {why}");
                return;
            }
        };
        // No type any more: back on the window's own recipes.
        if types.is_empty() {
            self.recipe_tables.synced_index = None;
            if self.recipe_tables.synced.take().is_some() {
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            return;
        }
        let built = self
            .res
            .recipes()
            .with_recipe_content(self.items(), received, &types);
        match built {
            Ok((recipes, index)) => {
                self.recipe_tables.synced = Some(recipes);
                self.recipe_tables.synced_index = Some(index);
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            Err(e) => log::warn!("Sunrise: the server's recipes were not taken: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    /// A part with a malformed serializer refuses the whole update: the
    /// valid last part after it publishes nothing, one refusal is reported
    /// when the update completes, and the next update is taken whole.
    #[test]
    fn a_bad_part_refuses_the_whole_update_at_its_last_part() {
        use sunrise_client::lan::recipes::ContentPart;
        let part = |text: &str| ContentPart::decode(text.as_bytes()).unwrap();
        let mut incoming = super::IncomingRecipes::default();
        assert!(
            incoming
                .push(part(
                    r#"{"recipes":[["a:one",{"type":"smelting"}]],"last":null}"#
                ))
                .is_none()
        );
        assert!(
            incoming
                .push(part(
                    r#"{"recipes":[["a:bad",{"type":"x:y:z"}]],"last":null}"#
                ))
                .is_none()
        );
        let refused = incoming.push(part(
            r#"{"recipes":[["a:two",{"type":"smelting"}]],"last":["minecraft:smelting"]}"#,
        ));
        assert!(refused.unwrap().unwrap_err().contains("a:bad"));
        let (recipes, types) = incoming
            .push(part(
                r#"{"recipes":[["a:three",{"type":"smelting"}]],"last":["minecraft:smelting"]}"#,
            ))
            .unwrap()
            .unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].0, "a:three");
        assert_eq!(types, ["minecraft:smelting"]);
    }

    use super::super::native_entity_refs::tests::{with_game, with_game_at};
    use mod_api::{ItemStack, RecipeInput, RecipeLookupError};

    fn stack(item: &str) -> Option<ItemStack> {
        ItemStack::new(
            mod_api::__private::item_key(&format!("minecraft:{item}")).unwrap(),
            1,
        )
    }

    /// With the server's crafting recipes synced, the book still holds the
    /// vanilla ones under their own display ids (a viewer's "+" reads them,
    /// `recipe_book_ids`), and a recipe only the server has gets one too.
    #[test]
    #[ignore = "daily"]
    fn synced_vanilla_recipes_keep_their_book_ids() {
        fn book_ids(game: &mut crate::game::Game<'_>, id: &str) -> Vec<i32> {
            (game.recipe_book_entries().into_iter())
                .filter(|e| e.id == id && e.display_id >= 0)
                .map(|e| e.display_id)
                .collect()
        }
        with_game(|game| {
            let ids = [
                "minecraft:stick",
                "minecraft:chest",
                "minecraft:iron_ingot_from_smelting_raw_iron",
            ];
            let own = ids.map(|id| book_ids(game, id));
            assert!(own.iter().all(|ids| !ids.is_empty()), "{own:?}");
            let types = ["minecraft:crafting", "minecraft:smelting"].map(str::to_owned);
            let pack = serde_json::json!({
                "type": "minecraft:crafting_shaped",
                "pattern": ["#", "#"],
                "key": {"#": "#minecraft:logs"},
                "result": {"id": "minecraft:stick", "count": 16},
            });
            let overlay = sunrise_world::loot::PackOverlay::NONE;
            let mut files =
                sunrise_items::recipe_sources::sources(&game.options.resources, &overlay).unwrap();
            files.push(("zzz:sticks_from_logs".to_owned(), pack));
            let server = sunrise_client::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &overlay,
                files,
                game.res.core.item_data.context(),
            )
            .unwrap();
            let sources = server.recipe_content(&types);
            for part in sunrise_client::lan::recipes::encode_content(&types, &sources, false) {
                game.remote_recipe_content(&part);
            }
            assert!(game.synced_recipes().is_some());
            assert_eq!(ids.map(|id| book_ids(game, id)), own);
            assert_eq!(book_ids(game, "zzz:sticks_from_logs").len(), 1);
        });
    }

    /// A content mod that ships files under `minecraft` (tags, as most do)
    /// doesn't make vanilla's namespace one of the root's content namespaces:
    /// listed, `recipe_sources::sources` read every vanilla recipe twice, as
    /// `stick` and `minecraft:stick`, and the book held two entries of each,
    /// numbered `n` and `n + 1`, so a viewer's "+" sent the neighbour's id. A
    /// vanilla recipe has exactly one book id, the server's, synced or not.
    #[test]
    #[ignore = "daily"]
    fn a_synced_vanilla_recipe_has_one_book_id_the_servers() {
        use std::path::{Path, PathBuf};
        fn book_ids(game: &mut crate::game::Game<'_>, id: &str) -> Vec<i32> {
            (game.recipe_book_entries().into_iter())
                .filter(|e| e.id == id && e.display_id >= 0)
                .map(|e| e.display_id)
                .collect()
        }
        let base = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let scratch =
            std::env::temp_dir().join(format!("sunrise-one-book-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let dir = scratch.join("tagger");
        for (name, text) in [
            (
                "sunrise-mod.toml",
                "id = \"tagger\"\nname = \"tagger\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
            ),
            (
                "data/minecraft/tags/item/tagger_extra.json",
                r#"{"values": ["minecraft:stick"]}"#,
            ),
        ] {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = sunrise_mod_package::Package::read(&dir).unwrap();
        let tagger = sunrise_client::mod_content::ContentMod {
            id: package.manifest.id.clone(),
            content: sunrise_mod_package::content::Content::read(&dir, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        let root = scratch.join("root");
        let inventory = sunrise_mod_package::enabled::EnabledMods::new([(
            "tagger".to_owned(),
            mod_api::Side::Both,
        )]);
        sunrise_client::mod_content::build(&base, Path::new(""), &[tagger], &root, &inventory)
            .unwrap();
        assert_eq!(sunrise_client::mod_content::namespaces(&root), ["tagger"]);
        with_game_at(root, |game| {
            let ids = [
                "minecraft:stick",
                "minecraft:stick_from_bamboo_item",
                "minecraft:chest",
                "minecraft:iron_ingot_from_smelting_raw_iron",
            ];
            // The integrated server numbers the book's displays by its index.
            let server = ids.map(|id| {
                let index = &game.stations().recipe_index;
                vec![index.first_display(id).unwrap()]
            });
            assert_eq!(ids.map(|id| book_ids(game, id)), server);
            let types = ["minecraft:crafting", "minecraft:smelting"].map(str::to_owned);
            let sources = game.recipes().recipe_content(&types);
            for part in sunrise_client::lan::recipes::encode_content(&types, &sources, false) {
                game.remote_recipe_content(&part);
            }
            assert!(game.synced_recipes().is_some());
            assert_eq!(ids.map(|id| book_ids(game, id)), server);
        });
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_matches_the_grid_and_returns_what_each_slot_keeps() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let kind = mod_api::Identifier::parse("minecraft:crafting").unwrap();
            let lookup = |width, height, slots: &[Option<mod_api::ItemStack>]| {
                level.recipe_for(
                    game,
                    &kind,
                    &RecipeInput::Crafting {
                        width,
                        height,
                        slots: slots.to_vec(),
                    },
                )
            };
            let cake = [
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("sugar"),
                stack("egg"),
                stack("sugar"),
                stack("wheat"),
                stack("wheat"),
                stack("wheat"),
            ];
            let made = lookup(3, 3, &cake).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:cake").unwrap(), 1)
            );
            let bucket = stack("bucket");
            let mut kept = vec![bucket.clone(), bucket.clone(), bucket];
            kept.extend([None, None, None, None, None, None]);
            assert_eq!(made.remainders, kept);

            let planks = [stack("oak_planks"), stack("oak_planks")];
            let made = lookup(1, 2, &planks).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, [None, None]);
            let mut corner = vec![None; 9];
            corner[5] = stack("birch_planks");
            corner[8] = stack("birch_planks");
            let made = lookup(3, 3, &corner).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, vec![None; 9]);

            let rocket = [stack("paper"), stack("gunpowder")];
            let made = lookup(2, 1, &rocket).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (
                    mod_api::__private::item_key("minecraft:firework_rocket").unwrap(),
                    3
                )
            );

            assert!(lookup(1, 1, &[stack("dirt")]).unwrap().is_none());
            assert!(lookup(1, 1, &[None]).unwrap().is_none());
            assert!(lookup(4, 1, &[None, None, None, None]).unwrap().is_none());
            assert!(lookup(0, 0, &[]).unwrap().is_none());
            assert!(lookup(1, 2, &planks[..1]) == Err(RecipeLookupError::InvalidInput));
        });
    }

    /// Vanilla expected outputs, ordered smithing, and a data-pack recipe parsed by the
    /// production recipe loader. These assertions do not derive expectations from matcher output.
    #[test]
    #[ignore = "daily"]
    fn recipe_for_dispatches_cooking_and_stonecutting_and_pack_recipes() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = |name| mod_api::Identifier::parse(name).unwrap();
                // A read-only query is not an attempted resident inventory write.
                let raw = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:raw_iron").unwrap(),
                    65,
                )
                .unwrap();
                for (name, ticks) in [("minecraft:smelting", 200), ("minecraft:blasting", 200)] {
                    let made = level
                        .recipe_for(game, &kind(name), &RecipeInput::Single(raw.clone()))
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        made.output.item(),
                        mod_api::__private::item_key("minecraft:iron_ingot").unwrap()
                    );
                    assert_eq!(made.output.count(), 1);
                    assert_eq!(made.processing_time, ticks);
                    assert_eq!(made.experience, 0.7);
                }
                for (name, ticks) in [
                    ("minecraft:smoking", 200),
                    ("minecraft:campfire_cooking", 600),
                ] {
                    let made = level
                        .recipe_for(
                            game,
                            &kind(name),
                            &RecipeInput::Single(stack("beef").unwrap()),
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        made.output.item(),
                        mod_api::__private::item_key("minecraft:cooked_beef").unwrap()
                    );
                    assert_eq!(
                        (made.output.count(), made.processing_time, made.experience),
                        (1, ticks, 0.35)
                    );
                }
                let cut = level
                    .recipe_for(
                        game,
                        &kind("minecraft:stonecutting"),
                        &RecipeInput::Single(stack("mossy_stone_bricks").unwrap()),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    cut.output.item(),
                    mod_api::__private::item_key("minecraft:mossy_stone_brick_slab").unwrap()
                );
                assert_eq!(cut.output.count(), 2);
                // Same resolved source path as a world's merged data packs, with a vanilla type.
                let recipes = sunrise_client::crafting::Recipes::from_sources(
                    &game.options.resources,
                    game.items(),
                    &sunrise_world::loot::PackOverlay::NONE,
                    vec![(
                        "test:pack_smelting".into(),
                        serde_json::json!({
                            "type":"minecraft:smelting", "ingredient":"minecraft:dirt",
                            "result":{"id":"minecraft:diamond"}, "experience":0.25, "cookingtime":42
                        }),
                    )],
                    game.res.core.item_data.context(),
                )
                .unwrap();
                game.recipe_tables.synced = Some(recipes);
                let made = level
                    .recipe_for(
                        game,
                        &kind("minecraft:smelting"),
                        &RecipeInput::Single(stack("dirt").unwrap()),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(made.recipe, kind("test:pack_smelting"));
                assert_eq!(
                    made.output.item(),
                    mod_api::__private::item_key("minecraft:diamond").unwrap()
                );
                assert_eq!((made.experience, made.processing_time), (0.25, 42));
                assert_eq!(
                    level.recipe_for(
                        game,
                        &kind("minecraft:blasting"),
                        &RecipeInput::Single(stack("dirt").unwrap())
                    ),
                    Ok(None)
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_preserves_smithing_order_and_damage() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = |name| mod_api::Identifier::parse(name).unwrap();
                let mut sword = stack("diamond_sword").unwrap();
                sword.set_damage(17);
                let input = RecipeInput::Smithing {
                    template: stack("netherite_upgrade_smithing_template"),
                    base: Some(sword),
                    addition: stack("netherite_ingot"),
                };
                let made = level
                    .recipe_for(game, &kind("minecraft:smithing"), &input)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    made.output.item(),
                    mod_api::__private::item_key("minecraft:netherite_sword").unwrap()
                );
                assert_eq!(made.output.damage(), 17);
                let wrong_order = RecipeInput::Smithing {
                    template: stack("netherite_ingot"),
                    base: stack("diamond_sword"),
                    addition: stack("netherite_upgrade_smithing_template"),
                };
                assert_eq!(
                    level.recipe_for(game, &kind("minecraft:smithing"), &wrong_order),
                    Ok(None)
                );
                assert_eq!(
                    level.recipe_for(game, &kind("minecraft:smelting"), &input),
                    Err(RecipeLookupError::InvalidInput)
                );
                assert_eq!(
                    level.recipe_for(
                        game,
                        &kind("minecraft:smithing"),
                        &RecipeInput::Smithing {
                            template: None,
                            base: None,
                            addition: None
                        }
                    ),
                    Ok(None)
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_refuses_mod_owned_types_instead_of_empty_matches() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = mod_api::Identifier::parse("test:crusher").unwrap();
                assert_eq!(
                    level.recipe_for(game, &kind, &RecipeInput::Single(stack("dirt").unwrap())),
                    Err(RecipeLookupError::NotMatchable)
                );
            })
        });
    }

    /// What one `level.recipe_for` costs, for benchmarks/craft-match-2026-11-04: a grid that matches
    /// early, one that matches late, a special recipe and a grid no recipe takes (the whole list).
    #[test]
    #[ignore = "a benchmark: run it in release with SUNRISE_TEST_RESOURCES"]
    fn craft_cost() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let mut cake = vec![stack("milk_bucket"); 3];
            cake.extend([stack("sugar"), stack("egg"), stack("sugar")]);
            cake.extend(vec![stack("wheat"); 3]);
            let grids: [(&str, u32, u32, Vec<Option<ItemStack>>); 4] = [
                ("cake (3x3)", 3, 3, cake),
                ("sticks (1x2)", 1, 2, vec![stack("oak_planks"); 2]),
                (
                    "firework rocket (2x1, special)",
                    2,
                    1,
                    vec![stack("paper"), stack("gunpowder")],
                ),
                ("dirt (1x1, no match)", 1, 1, vec![stack("dirt")]),
            ];
            println!("{} crafting recipes", game.recipes().crafting.len());
            const CALLS: u32 = 10_000;
            let kind = mod_api::Identifier::parse("minecraft:crafting").unwrap();
            for (name, width, height, grid) in grids {
                let input = RecipeInput::Crafting {
                    width,
                    height,
                    slots: grid,
                };
                let start = std::time::Instant::now();
                for _ in 0..CALLS {
                    let _ = std::hint::black_box(level.recipe_for(game, &kind, &input));
                }
                println!(
                    "{name}: {:.1} µs a call",
                    start.elapsed().as_secs_f64() * 1e6 / f64::from(CALLS)
                );
            }
        });
    }
}
