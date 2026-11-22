//! The creative build-event declaration read by the mod API generator.

pub trait Hooks {
    /// Edit a creative mode tab's ordered parent and search lists after its generator and before
    /// feature-flag filtering (NeoForge's `BuildCreativeModeTabContentsEvent`). Runs once for
    /// every registered tab, including Search, Hotbar and Inventory, on the logical side that
    /// requests the build. The event gives the tab id, enabled feature flags, permission profile
    /// and a build-scoped edit handle. Edits are visible to later listeners in dispatch order.
    ///
    /// Sunrise keeps one last-build cache per logical side and world; opening the screen or
    /// reading through `ItemsHost::creative_tabs` may build it. Within a build, a same-parameter
    /// Host read sees completed tabs and the prior contents of tabs yet to be built, including
    /// the current tab. Different parameters are refused. Tag-only reloads do not rebuild.
    /// Unlike NeoForge, invalid edits return typed errors, and reads of the mutable lists give
    /// snapshots. Both-list edits check and change parent before search without undoing parent
    /// on a search error.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "listen",
        version = 3,
        neoforge = "BuildCreativeModeTabContentsEvent"
    )]
    fn build_creative_mode_tab_contents(
        tab: &str,
        features: Vec<String>,
        permissions: mod_api::CreativeTabPermissions,
        edit: mod_api::CreativeTabEdit,
    );
}
