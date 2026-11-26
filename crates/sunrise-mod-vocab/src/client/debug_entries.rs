//! Named entries and world-space renderers a native mod adds to the debug screen (NeoForge's
//! `RegisterDebugEntriesEvent`, `RegisterDebugRenderersEvent`).

use crate::{ClientHost, Identifier, WorldDrawContext, WorldRenderView};

// no wire: this callable and its captures stay in the client process.
/// Appends lines for one registered F3 text entry. The host is borrowed for this draw.
pub type DebugEntryLines = dyn Fn(&dyn ClientHost, &mut Vec<String>) + Send + Sync + 'static;

// no wire: a text entry owns a process-local callback.
/// An F3 entry registered at client startup: text in a group or a renderer toggle.
// exhaustive: private storage keeps the ID with either a text callback or a renderer toggle.
pub struct DebugEntry {
    id: String,
    text: Option<(DebugGroup, Box<DebugEntryLines>)>,
    allowed_in_reduced_debug_info: bool,
}

impl DebugEntry {
    /// Declare a text entry under an unused `<namespace>:<path>` such as `mymod:tick_cost`.
    /// The client retains `lines` for the session and calls it as the registering mod only
    /// while the entry is visible. Append lines to the supplied vector; legacy colour codes (§)
    /// work. Appending nothing leaves the group unchanged. `group` is a vanilla group or one of
    /// the mod's own ([`DebugGroup::Named`]).
    ///
    /// Hidden while reduced debug info is active unless opted in with
    /// [`Self::with_reduced_debug_info`], matching vanilla's `DebugScreenEntry.isAllowed`.
    pub fn text(
        id: String,
        group: DebugGroup,
        lines: impl Fn(&dyn ClientHost, &mut Vec<String>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            id,
            text: Some((group, Box::new(lines))),
            allowed_in_reduced_debug_info: false,
        }
    }

    /// Declare a renderer toggle under a `<namespace>:<path>` identifier such as
    /// `mymod:pipe_network`. A toggle has no text group or line callback.
    ///
    /// Hidden while reduced debug info is active unless opted in with
    /// [`Self::with_reduced_debug_info`], matching vanilla's `DebugScreenEntry.isAllowed`.
    pub fn renderer(id: String) -> Self {
        Self {
            id,
            text: None,
            allowed_in_reduced_debug_info: false,
        }
    }

    /// Set whether this entry stays allowed when reduced debug info is active
    /// (`DebugScreenEntry.isAllowed`). Defaults to `false`.
    pub fn with_reduced_debug_info(mut self, allowed: bool) -> Self {
        self.allowed_in_reduced_debug_info = allowed;
        self
    }

    /// The full identifier used in the debug options and profile.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The group and line callback of a text entry, or `None` for a renderer toggle.
    pub fn text_lines(&self) -> Option<(&DebugGroup, &DebugEntryLines)> {
        self.text.as_ref().map(|(group, lines)| (group, &**lines))
    }

    /// Whether this entry may show while reduced debug info is active.
    pub fn allowed_in_reduced_debug_info(&self) -> bool {
        self.allowed_in_reduced_debug_info
    }

    /// Whether this entry is allowed under the current reduced-debug-info state
    /// (`DebugScreenEntry.isAllowed`).
    pub fn is_allowed(&self, reduced_debug_info: bool) -> bool {
        !reduced_debug_info || self.allowed_in_reduced_debug_info
    }
}

type DebugRendererDraw = dyn Fn(&dyn ClientHost, &WorldRenderView<'_>, &mut dyn WorldDrawContext)
    + Send
    + Sync
    + 'static;

// no wire: the renderer owns a process-local draw callback.
/// A world-space debug renderer owned by the client until its renderer list next refreshes.
// exhaustive: private storage keeps the draw callback opaque.
pub struct DebugRenderer {
    callback_type: std::any::TypeId,
    draw: Box<DebugRendererDraw>,
}

impl DebugRenderer {
    /// Submit a world-space debug renderer on a renderer-list refresh. The client calls `draw`
    /// after custom geometry in each world view, as the registering mod, until the next refresh.
    /// Capture any renderer-specific state in the callback. To gate a renderer on a debug
    /// options switch, check [`crate::RegisterDebugRenderersEvent::is_debug_entry_enabled`]
    /// before pushing it.
    pub fn new<F>(draw: F) -> Self
    where
        F: Fn(&dyn ClientHost, &WorldRenderView<'_>, &mut dyn WorldDrawContext)
            + Send
            + Sync
            + 'static,
    {
        Self {
            callback_type: std::any::TypeId::of::<F>(),
            draw: Box::new(draw),
        }
    }

    /// Run this renderer's callback with the current world view and drawing context.
    pub fn draw(
        &self,
        host: &dyn ClientHost,
        view: &WorldRenderView<'_>,
        draw: &mut dyn WorldDrawContext,
    ) {
        (self.draw)(host, view, draw);
    }
}

/// Engine-internal callback identity used to keep a panicked debug renderer disabled across
/// renderer-list refreshes for the session.
pub fn debug_renderer_callback_type(renderer: &DebugRenderer) -> std::any::TypeId {
    renderer.callback_type
}

// no wire: engine-side read-only view borrowed by `RegisterDebugRenderersEvent`.
/// Read-only view of the client's current debug-entry selection during renderer-list refresh.
// hidden: listeners query enabled switches through `RegisterDebugRenderersEvent::is_debug_entry_enabled`.
#[doc(hidden)]
pub trait DebugEntryStates {
    /// Whether the vanilla or mod debug entry `id` is currently enabled
    /// (`DebugScreenEntryList.isCurrentlyEnabled`).
    fn is_enabled(&self, id: &str) -> bool;
}

impl crate::RegisterDebugRenderersEvent<'_> {
    /// Whether the vanilla or mod debug entry `id` is currently enabled under the active
    /// overlay and reduced-debug-info settings (`DebugScreenEntryList.isCurrentlyEnabled`).
    /// Accepts full `<namespace>:<path>` identifiers (such as `minecraft:chunk_borders` or
    /// `mymod:pipes`) and bare vanilla paths (such as `chunk_borders`).
    pub fn is_debug_entry_enabled(&self, id: &str) -> bool {
        self.entries.is_enabled(id)
    }
}

/// The group a debug entry's lines join (`DebugScreenDisplayer.addToGroup`): one of vanilla's
/// or one named by the mod. Each group's lines stay together, the groups in the order their
/// first lines were added, split half into the left column and half into the right with a blank
/// line after each, as vanilla splits them.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: vanilla's debug screen text groups and a mod's own, matched without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DebugGroup {
    /// `position`: the player's position, section and speed.
    Position,
    /// `light`: the light levels at the feet.
    Light,
    /// `memory`: the process's memory use.
    Memory,
    /// `looking_at_block`: the targeted block's state and tags.
    LookingAtBlock,
    /// `looking_at_entity`: the targeted entity and its tags.
    LookingAtEntity,
    /// `looking_at_fluid`: the targeted fluid's state and tags.
    LookingAtFluid,
    /// `chunk_generation`: the density of the chunk at the feet.
    ChunkGeneration,
    /// A group by identifier, such as a mod's own `mymod:debug_info`, which every entry naming
    /// it joins. Vanilla's groups are `minecraft:` identifiers, so `minecraft:memory` is
    /// [`Self::Memory`]'s group.
    Named(Identifier),
}

#[cfg(test)]
mod debug_entries_tests {
    use crate::*;

    #[test]
    fn debug_entry_reduced_debug_info_defaults_false_and_opts_in() {
        let default_text = DebugEntry::text("test:a".into(), DebugGroup::Memory, |_, _| {});
        assert!(!default_text.allowed_in_reduced_debug_info());
        assert!(default_text.is_allowed(false));
        assert!(!default_text.is_allowed(true));

        let allowed_text = DebugEntry::text("test:b".into(), DebugGroup::Memory, |_, _| {})
            .with_reduced_debug_info(true);
        assert!(allowed_text.allowed_in_reduced_debug_info());
        assert!(allowed_text.is_allowed(false));
        assert!(allowed_text.is_allowed(true));

        let default_renderer = DebugEntry::renderer("test:r".into());
        assert!(!default_renderer.allowed_in_reduced_debug_info());
        assert!(!default_renderer.is_allowed(true));

        let allowed_renderer = DebugEntry::renderer("test:r2".into()).with_reduced_debug_info(true);
        assert!(allowed_renderer.allowed_in_reduced_debug_info());
        assert!(allowed_renderer.is_allowed(true));
    }

    #[test]
    fn register_debug_renderers_event_queries_entry_states_and_accepts_unconditional_renderers() {
        struct States(&'static [&'static str]);
        impl crate::__private::DebugEntryStates for States {
            fn is_enabled(&self, id: &str) -> bool {
                self.0.contains(&id)
            }
        }

        struct Probes;
        #[crate::__subscriptions]
        impl Hooks for Probes {
            fn register_debug_renderers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterDebugRenderersEvent<'_>,
            ) {
                if event.is_debug_entry_enabled("test:on") {
                    event.renderers.push(DebugRenderer::new(|_, _, _| {}));
                }
                if event.is_debug_entry_enabled("test:off") {
                    event.renderers.push(DebugRenderer::new(|_, _, _| {}));
                }
                // Unconditional renderer requires no switch entry.
                event.renderers.push(DebugRenderer::new(|_, _, _| {}));
            }
        }

        testing::with_dispatch(
            vec![ModEntry::new("test_probes", Box::new(Probes))].into(),
            || {
                let mut renderers = Vec::new();
                dispatch_register_debug_renderers(
                    &RegistrationContext::new(),
                    &States(&["test:on"]),
                    &mut renderers,
                );
                assert_eq!(renderers.len(), 2);
                assert!(renderers.iter().all(|r| r.mod_id == "test_probes"));

                let mut renderers = Vec::new();
                dispatch_register_debug_renderers(
                    &RegistrationContext::new(),
                    &States(&[]),
                    &mut renderers,
                );
                assert_eq!(renderers.len(), 1);
            },
        );
    }
}
