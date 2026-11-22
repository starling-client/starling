//! A mod menu whose recipe book layout is `Mod`: its part answers the click (`recipe_book_place`)
//! and places with vanilla's placer through `place_recipe_in_grid`.
use super::{Game, native_book_backings::Backings, native_book_scope::MenuSnapshot};
use sunrise_client::{
    menu::{
        Env, Menu,
        place::{
            PostPlace,
            custom_plan::{MenuPlacement, PlacementRefusal},
        },
    },
    recipe_book::{Settings, placement_sync::Refusal},
};

// no wire: the detached placement a part is answering, alive only while its hook runs.
pub(super) struct Session {
    pub(super) recipe: String,
    pub(super) plan: MenuPlacement,
    pub(super) menu: Box<Menu>,
    pub(super) graph: Backings,
    pub(super) before: MenuSnapshot,
    pub(super) known: Settings,
    pub(super) limited: bool,
    pub(super) use_max: bool,
    /// `place_recipe_in_grid` has placed this click's recipe: it places once.
    pub(super) placed: bool,
}

fn reason(refusal: PlacementRefusal) -> &'static str {
    match refusal {
        PlacementRefusal::NotPlaceable => "the recipe cannot be placed",
        PlacementRefusal::InvalidLayout => "the grid does not fit the recipe or the menu",
        PlacementRefusal::Unavailable => "the menu is not available",
        PlacementRefusal::Denied => "a slot refuses its pickup",
        PlacementRefusal::NoRoom => "the player has no room for the cleared items",
        PlacementRefusal::Changed => "the player or the menu changed",
    }
}

impl Game<'_> {
    /// Ask the part of the open menu's type to place `session`'s recipe, with the player swapped in.
    /// The placement comes back with the answer, changed by what the part placed.
    pub(super) fn ask_mod_book_place(
        &mut self,
        session: Session,
    ) -> Result<(Session, PostPlace), Refusal> {
        let menu_id = session.before.kind.id;
        if !mod_api::__private::menu_listens(mod_api::HookId::RecipeBookPlace, menu_id) {
            return Err(Refusal::Unsupported);
        }
        let uuid = session.before.uuid;
        let creative = session.before.game_mode == 1;
        let (recipe, use_max_items) = (session.recipe.clone(), session.use_max);
        self.native_net.placement.session = Some(session);
        let mut ghost = false;
        let ran = self.native_as_player(uuid, |game| {
            game.native_recipe_book_place(mod_api::__private::RecipeBookPlaceCall {
                menu: menu_id,
                recipe: &recipe,
                use_max_items,
                creative,
                ghost: &mut ghost,
            });
        });
        let session = self
            .native_net
            .placement
            .session
            .take()
            .ok_or(Refusal::Changed)?;
        if !ran {
            return Err(Refusal::Changed);
        }
        let post = if ghost {
            PostPlace::Ghost
        } else {
            PostPlace::Nothing
        };
        Ok((session, post))
    }

    /// `place_recipe_in_grid`: only while a `recipe_book_place` part runs. The placement is worked
    /// out on the detached menu; the game compares the player and the blocks again before
    /// it publishes anything.
    pub(super) fn place_recipe_in_grid(
        &mut self,
        recipe: &str,
        grid_width: u16,
        grid_height: u16,
        grid_slots: Vec<u16>,
        clear_slots: Vec<u16>,
        use_max_items: bool,
    ) -> Result<bool, String> {
        let mut session = self
            .native_net
            .placement
            .session
            .take()
            .ok_or("place_recipe_in_grid is only available inside recipe_book_place")?;
        let result = self.place_in_session(
            &mut session,
            recipe,
            (grid_width, grid_height),
            &grid_slots,
            &clear_slots,
            use_max_items,
        );
        self.native_net.placement.session = Some(session);
        result
    }

    fn place_in_session(
        &self,
        session: &mut Session,
        recipe: &str,
        (width, height): (u16, u16),
        grid: &[u16],
        clear: &[u16],
        use_max: bool,
    ) -> Result<bool, String> {
        if session.placed {
            return Err("place_recipe_in_grid was already called for this click".into());
        }
        if recipe != session.recipe {
            return Err(format!("the click is for {}, not {recipe}", session.recipe));
        }
        let plan = session
            .plan
            .in_grid(width, height, grid, clear, session.before.size)?;
        for &slot in plan.cleared_slots() {
            if !session.graph.stores(slot) {
                return Err(format!("slot {slot} has no storage"));
            }
        }
        let env = Env {
            player: Some(session.before.uuid),
            player_entity_id: Some(session.before.entity_id),
            can_equip: None,
            known_recipes: Some(&session.known),
            slot_rules: None,
            stacked: None,
            limited_crafting: session.limited,
            ..self.menu_env()
        };
        let creative = session.before.game_mode == 1;
        let post = session
            .menu
            .place_custom_recipe(&env, &plan, use_max, creative, || true)
            .map_err(|refusal| reason(refusal).to_owned())?;
        session.placed = true;
        Ok(post == PostPlace::Ghost)
    }
}
