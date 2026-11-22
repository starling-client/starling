//! Extra geometry in chunk section meshes, as a client mod registers it in `add_section_geometry`
//! (NeoForge's `AddSectionGeometryEvent` and its `AdditionalSectionGeometryRenderer`).
//!
//! A mod's renderer is held by the game and asked on the meshing workers each time a section
//! meshes, like a block tint source: it gets no `Host` and keeps no mutable state, since two
//! workers can mesh neighbouring sections at once. The quads it adds are part of the section's
//! mesh, so they are rebuilt with it, never added twice, and gone when the section unloads.
use super::BlockAndTintGetter;
use std::sync::Arc;

/// A mod's extra section geometry: NeoForge's `AdditionalSectionGeometryRenderer`.
pub trait SectionGeometry: Send + Sync + 'static {
    /// `render(context)`: adds the quads of the section whose lowest corner is block `origin` (a
    /// multiple of 16 on each axis) through `context`. Asked on a meshing worker every time that
    /// section meshes, also when it holds only air; a renderer with nothing for the section adds
    /// nothing. What it adds is taken whole or refused whole, logged once: see
    /// [`SectionGeometryContext::add`]. A renderer that panics is asked no more.
    fn add(&self, origin: [i32; 3], context: &mut dyn SectionGeometryContext);
}

/// One section a [`SectionGeometry`] adds quads to. The section's quads, its own blocks' and
/// every renderer's together, share one budget: 262144 quads and 32 MiB of mesh data. All
/// sections meshing at once share another, 128 MiB of added quads not yet in their meshes. A
/// quad is checked against them before it is kept.
pub trait SectionGeometryContext {
    /// Reads the section's chunk and its eight neighbours.
    fn level(&self) -> &dyn BlockAndTintGetter;
    /// Adds `quad`, drawn with the block atlas sprite `sprite` (`minecraft:block/stone`, a mod's
    /// `mymod:block/glow`). False when this renderer's quads for the section are refused, and
    /// then none of them are drawn and every later `add` is false too: a quad past a budget, a
    /// corner or UV out of range or not a number, or a sprite the atlas lacks refuses them all.
    fn add(&mut self, sprite: &str, quad: SectionQuad) -> bool;
    /// How many more quads the section takes before its budget refuses one.
    fn remaining(&self) -> u32;
}

/// One quad a [`SectionGeometry`] adds. Its corners go counter-clockwise seen from the side it
/// faces; a solid quad is culled from behind, as a block's faces are, so a quad seen from both
/// sides is two quads.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: mods build quads with struct literals
pub struct SectionQuad {
    /// The corners, in blocks from the section's lowest corner, each 0 to 16: a quad stays inside
    /// its section.
    pub positions: [[f32; 3]; 4],
    /// Each corner's texture coordinate within the sprite, 0 to 1.
    pub uvs: [[f32; 2]; 4],
    /// ARGB the sprite's texels are multiplied by; `0xFFFFFFFF` leaves them as they are.
    pub color: u32,
    /// Sky and block light, 0 to 15 each (`BlockAndTintGetter::brightness` reads the world's).
    pub light: [u8; 2],
    /// Drawn in the translucent pass (sorted, blended) rather than the solid one (alpha cutout).
    pub translucent: bool,
}

/// The renderer registry `add_section_geometry` hands out. A panic or phase refusal
/// in registration discards only that callback's renderers.
pub trait SectionGeometryRenderers {
    /// `addRenderer(renderer)`: `renderer` adds its quads to every section the client meshes, in
    /// registration order after the section's own blocks.
    fn register(&mut self, renderer: Arc<dyn SectionGeometry>);
}

/// The game's [`SectionGeometryRenderers`]: the renderers in registration order.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
// no wire: process-local renderer storage; renderers run on the meshing workers, not serialized.
pub struct SectionGeometryRegistry {
    renderers: Vec<Arc<dyn SectionGeometry>>,
}

impl SectionGeometryRenderers for SectionGeometryRegistry {
    fn register(&mut self, renderer: Arc<dyn SectionGeometry>) {
        self.renderers.push(renderer);
    }
}

impl SectionGeometryRegistry {
    /// Discard only the current listener's renderer registrations if it fails.
    // hidden: the engine stages one listener at a time; mods receive only the registrar trait.
    #[doc(hidden)]
    pub fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn SectionGeometryRenderers) -> bool,
    ) {
        let checkpoint = self.renderers.len();
        if !listen(self) {
            let site = (crate::HookId::AddSectionGeometry, "add_section_geometry");
            let owner = crate::content::crash::current_native_mod();
            for renderer in self.renderers.drain(checkpoint..) {
                crate::runtime::discard_registration(site, owner, renderer);
            }
        }
    }

    /// The renderers, in registration order.
    pub fn into_renderers(self) -> Vec<Arc<dyn SectionGeometry>> {
        self.renderers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddSectionGeometryEvent, Hooks, ModEntry, RegistrationContext};

    #[test]
    fn a_panicking_section_registration_does_not_install_its_renderer() {
        struct Renderer(u8);
        impl SectionGeometry for Renderer {
            fn add(&self, _: [i32; 3], _: &mut dyn SectionGeometryContext) {
                std::hint::black_box(self.0);
            }
        }
        struct Register(Arc<dyn SectionGeometry>, bool);
        #[mod_api::__subscriptions]
        impl Hooks for Register {
            fn add_section_geometry(
                &self,
                _: &RegistrationContext,
                event: &mut AddSectionGeometryEvent<'_>,
            ) {
                event.renderers.register(self.0.clone());
                assert!(
                    !self.1,
                    "section registration failed after adding a renderer"
                );
            }
        }
        let healthy: Arc<dyn SectionGeometry> = Arc::new(Renderer(1));
        crate::testing::with_dispatch(
            vec![
                ModEntry::new("failing", Box::new(Register(Arc::new(Renderer(0)), true))),
                ModEntry::new("healthy", Box::new(Register(healthy.clone(), false))),
            ]
            .into(),
            || {
                let mut registry = SectionGeometryRegistry::default();
                crate::dispatch_add_section_geometry(&RegistrationContext::new(), |listen| {
                    registry.register_listener(listen);
                });
                let renderers = registry.into_renderers();
                assert_eq!(renderers.len(), 1);
                assert!(Arc::ptr_eq(&renderers[0], &healthy));
            },
        );
    }
}
