//! Player-owned custom recipe book settings, separate from vanilla ordinals.
use super::Game;
use mod_api::{
    PlayerRef,
    item::recipe_book::{RecipeBookSettings, RecipeBookTypeKey},
};
use sunrise_client::recipe_book::{BookFlags, placement_sync::ResourceId};

impl Game<'_> {
    pub(super) fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        self.native_local_status(player)?;
        self.custom_book_settings(book)
    }
    pub(super) fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        self.native_local_status(player).is_some()
            && self.request_custom_book_settings(book, settings)
    }

    pub(super) fn player_set_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool {
        if !book.is_registered() || !self.native_player_current(player) {
            return false;
        }
        let flags = BookFlags {
            open: settings.open,
            filtering: settings.filtering,
        };
        let id = ResourceId::from_registered_book(book);
        let mut publish = false;
        if !self.native_as_player_handle(player, |game| {
            if game.level.recipe_book.custom.get(id.as_str()) != flags {
                let deferred = game.journal_player_book(player, book);
                publish = game.level.recipe_book.custom.set(&id, flags) && !deferred;
            }
        }) {
            return false;
        }
        if publish {
            self.publish_player_book_settings(player, book.clone(), flags);
        }
        true
    }

    pub(super) fn publish_player_book_settings(
        &mut self,
        player: PlayerRef,
        book: RecipeBookTypeKey,
        flags: BookFlags,
    ) {
        self.sync_server_book_write(player, &book, flags);
        if mod_api::__private::listened(mod_api::HookId::PlayerRecipeBookSettingsChange)
            && let Some(name) = self.player_name(player).map(str::to_owned)
        {
            self.native_player_recipe_book_settings_change(
                player.uuid(),
                &name,
                mod_api::RecipeBookIdentity::Custom(book),
                flags.open,
                flags.filtering,
            );
        }
    }

    pub(super) fn player_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings> {
        if !book.is_registered() || !self.native_player_current(player) {
            return None;
        }
        let settings = match self.native_player_resting(player) {
            Some(guest) => &guest.recipe_book,
            None => &self.level.recipe_book,
        };
        let flags = settings.custom.get(book.as_str());
        Some(RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }
}
