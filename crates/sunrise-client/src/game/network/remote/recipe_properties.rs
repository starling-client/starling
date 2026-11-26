use super::*;

#[cfg(test)]
mod client_logging_out_tests;
#[cfg(test)]
mod tests;

impl Game<'_> {
    /// Minecraft.disconnect's `ClientPlayerNetworkEvent.LoggingOut`: on
    /// leaving a world, starting a new world or server connection, and
    /// quitting the game (including from the title screen). Fires before
    /// teardown with the live local player handle and game mode when present,
    /// or `(None, None)` on no-player transitions; once per connection,
    /// whichever way out runs first.
    pub(in crate::game) fn client_logging_out(&mut self) {
        if !self.options.is_client_start() {
            return;
        }
        if let Some(remote) = self.remote.as_mut() {
            if remote.logged_out {
                return;
            }
            remote.logged_out = true;
            remote.camera = None;
            remote.detached_camera = None;
        }
        let player = self
            .remote
            .as_ref()
            .filter(|remote| remote.has_level())
            .and_then(|_| self.local_player());
        let game_mode = player.map(|_| self.mode);
        self.clear_spectated_entity_post_effect();
        self.reset_client_item_use();
        // Camera.reset clears initialization while keeping the last aligned position.
        self.view.initialized = false;
        self.native_client_player_logging_out(player, game_mode);
        if self.remote.is_none() {
            return;
        }
        if self.mod_screen.is_some() {
            self.remote_close_local();
        }
        self.unload_client_level();
        self.native_lifecycle.begin_close();
        self.native_lifecycle_world_closed();
        self.recipe_ui.server_properties = Default::default();
        self.recipe_ui.server_stations = None;
        self.refresh_recipe_property_context();
    }

    /// `Minecraft.setLevel` / `clearClientLevel`: once per installed level,
    /// while it is still the active level and its chunks and handles resolve.
    pub(super) fn unload_client_level(&mut self) {
        if let Some(dimension) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&dimension);
        }
    }

    pub(super) fn remote_recipe_properties(
        &mut self,
        packet: sunrise_protocol::packets::recipes::UpdateRecipes,
    ) {
        self.recipe_ui.server_properties.replace(packet);
        self.refresh_remote_stonecutter();
        self.refresh_recipe_property_context();
    }

    pub(in crate::game) fn refresh_remote_stonecutter(&mut self) {
        self.recipe_ui.server_stations = self.remote.as_ref().map(|remote| {
            let item = |id: i32| {
                remote
                    .builtin
                    .get("minecraft:item")?
                    .name(id)
                    .map(str::to_owned)
            };
            let category = |_: i32| None;
            let tag = |name: &str| {
                let name = sunrise_client::crafting::qualified(name);
                remote
                    .registries
                    .tag("minecraft:item", &name)
                    .into_iter()
                    .flatten()
                    .filter_map(|&id| item(id))
                    .collect()
            };
            let names = sunrise_client::recipe_book::PacketNames {
                item: &item,
                category: &category,
                component: &|_| None,
                tag: &tag,
            };
            self.stations().recipes.with_stonecutting(
                names.stonecutting(&self.recipe_ui.server_properties.stonecutter, |stack| {
                    self.decode_remote_stack(stack)
                }),
            )
        });
        if let Some(station) = self
            .station
            .as_mut()
            .filter(|station| station.kind == crate::game::stations::Kind::Stonecutter)
        {
            // The table and the menu's cached indices are one generation. Rebuild even
            // when the input stack has not changed since the previous packet.
            station.reset_stonecutter_generation();
            self.remote_station_slot_changed(crate::game::stations::INPUT);
        }
    }
}
