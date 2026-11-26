//! The creative build-event declaration read by the mod API generator.

pub trait Hooks {
    /// Edit a creative mode tab's ordered parent and search lists after its generator and before
    /// feature-flag filtering (NeoForge's `BuildCreativeModeTabContentsEvent`). Runs once for
    /// every registered tab, including Search, Hotbar and Inventory, on the logical side that
    /// requests the build. The event gives the tab id, enabled feature flags, permission profile
    /// and lends [`CreativeTabOutput`] for this tab's build. Entries borrow the output and
    /// edits are visible to later listeners in dispatch order.
    ///
    /// Sunrise keeps one last-build cache per logical side and world; opening the screen or
    /// reading through `ItemsHost::creative_tabs` may build it. Within a build, a same-parameter
    /// Host read sees completed tabs and the prior contents of tabs yet to be built, including
    /// the current tab. Different parameters are refused. Tag-only reloads do not rebuild.
    /// Invalid edits return typed errors and are logged. Both-list edits check and change parent
    /// before search without undoing parent on a search error. A panicking listener is reported
    /// and disabled for this session; completed edits stand, the next listener still runs, and
    /// the action that requested the catalog is not aborted.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "listen",
        version = 4,
        event_per_listener = true,
        neoforge = "BuildCreativeModeTabContentsEvent"
    )]
    fn build_creative_mode_tab_contents(
        tab: &str,
        features: &[String],
        permissions: mod_api::CreativeTabPermissions,
        output: &mut dyn mod_api::CreativeTabOutput,
    );
}
