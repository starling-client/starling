use super::*;

impl Game<'_> {
    /// The areas the game itself draws beside the open screen's window: the
    /// container build's, or the frame's own arithmetic with no build yet.
    pub(super) fn screen_own_exclusion(&self, batch: &HudBatch) -> Vec<[f32; 4]> {
        if !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            return Vec::new();
        }
        match self.built_container(batch) {
            Some(layout) => layout.exclusion(),
            None => self.container_layout(batch).exclusion(),
        }
    }
    pub(in crate::game) fn mod_data_root() -> PathBuf {
        Self::game_dir()
            .join(mod_list::DIR)
            .join(".sunrise-mod-data")
    }
    /// The game's `config/` folder beside `options.txt` (beside
    /// `server.properties` on a dedicated server), shared by every mod as on
    /// Fabric and NeoForge (`FMLPaths.CONFIGDIR`). Startup creates the folder;
    /// lending it performs no I/O.
    pub(in crate::game) fn config_dir(&self) -> &Path {
        &self.config_root
    }
    /// Whether a mod with the `sunrise-mod.toml` id `id` is installed: a
    /// native or content package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    pub(in crate::game) fn is_loaded(id: &str) -> bool {
        mod_api::__private::loaded_mods().get(id).is_some()
    }
    /// Shows these recipes in the panel instead of its items, as many as
    /// fit a page, under `title` and a back button, with a row of `tabs`
    /// (each an icon; `tab` is the selected one) between them. At most 256
    /// recipes and 16 tabs; no recipes shows the items again. The same title
    /// and tab again keep the page the player turned to.
    pub(in crate::game) fn native_panel_recipes(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<mod_api::Stack>,
        tab: u32,
        views: Vec<NativeRecipeView>,
    ) {
        let page = RecipePage::new(title, tabs, tab, views);
        self.panel.show_recipes(mod_id, page);
    }
    /// No world is open: a client without a player (the title screen). A
    /// server's `Game` (the server thread, a dedicated server, a LAN host)
    /// has no player of its own, and is always in its world.
    pub(super) fn outside_world(&self) -> bool {
        self.player.is_none() && self.options.execution_role.is_client()
    }
}
