//! Per-recipient recipe selection on the server thread.
use crate::game::Game;

impl Game<'_> {
    pub(in crate::game) fn prepare_recipe_join(&mut self, conn: u32) {
        self.ask_recipe_for_conn(conn);
        self.native_filter_commands(Some(conn));
    }

    /// Ask once per online player before rebuilding their recipe payloads.
    /// No online players means no calls.
    pub(in crate::game) fn sync_recipe_content(&mut self) {
        let connections: Vec<_> = self
            .lan
            .as_ref()
            .map(|host| {
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.placed && g.participates_online())
                    .map(|g| g.conn)
                    .collect()
            })
            .unwrap_or_default();
        for conn in connections {
            self.ask_recipe_for_conn(conn);
        }
    }

    fn ask_for(&mut self, player: mod_api::PlayerRef) -> Vec<String> {
        let mut recipe_types = Vec::new();
        self.native_datapack_sync(mod_api::__private::DatapackSyncCall {
            player,
            recipe_types: &mut recipe_types,
        });
        recipe_types
    }

    fn ask_recipe_for_conn(&mut self, conn: u32) {
        let (Some(epoch), Some(host)) = (self.native_lifecycle.epoch(), self.lan.as_ref()) else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let sunrise = g.brand == "sunrise";
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, g.entity_id as u32),
            g.uuid,
        );
        let requested = self.ask_for(player);
        let types = crate::crafting::mod_kinds::sync_recipe_types(&requested);
        // Use the published generation: a failed reload must not leak candidate files.
        let sources = if sunrise {
            self.recipes().recipe_content(&types)
        } else {
            Vec::new()
        };
        if let Some(host) = self.lan.as_mut() {
            host.hub.update_guest(conn, |g| {
                g.recipe_sync.replace(sunrise, types, &sources);
            });
        }
    }
}
