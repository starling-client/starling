use super::*;
/// The world fluids the mods declared in code, once they have all registered.
pub fn declared_world_fluids() -> &'static [(&'static str, (String, crate::WorldFluid))] {
    installed().map_or(&[], |installed| &installed.world_fluids)
}

/// The fluid interactions the mods declared, in registration order.
pub fn declared_fluid_interactions() -> &'static [crate::FluidInteraction] {
    installed().map_or(&[], |installed| &installed.fluid_interactions)
}

impl HookParts {
    pub(crate) fn fluid<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn FluidPart>] {
        attached(&self.fluids, id)
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to the world fluid `fluid`, a mod's, by the id it was declared with.
    pub fn fluid_part<P: FluidPart>(&mut self, fluid: &str, part: P) {
        self.attach_fluid_part(fluid, type_name::<P>(), Arc::new(part));
    }
}

pub(crate) fn resolved_fluids() -> &'static OnceLock<sunrise_mod_runtime::registry_keys::Names> {
    static EMPTY: OnceLock<sunrise_mod_runtime::registry_keys::Names> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.fluids)
}

/// Registrar operations for fluids. Bare declarations use the calling mod's ID as their
/// namespace. Explicit declarations may use a namespace owned by its manifest or by a content
/// package it serves via `served_by`.
pub trait FluidsRegistrar {
    /// Request a fluid by canonical registry id, checked when the registry loads.
    fn fluid_key(&mut self, id: &str) -> crate::FluidKey;

    /// Declare a tank-only fluid and return its key.
    fn declare_fluid(&mut self, path: &str) -> crate::FluidKey;

    /// Declare a fluid that exists in the world (NeoForge's `RegisterEvent` for a `FluidType`),
    /// with its block and, as `fluid` says, its bucket. Its
    /// `data/<namespace>/sunrise/fluid/<path>.json`, if any, says how it looks.
    fn declare_world_fluid(&mut self, path: &str, fluid: crate::WorldFluid) -> crate::FluidKey;

    /// Declare what a liquid block of `interaction.source` turns into beside another fluid or
    /// block (NeoForge's `FluidInteractionRegistry.addInteraction`), run when the liquid is placed
    /// and when a neighbour changes. Mods' interactions follow vanilla's two lava rules, in load
    /// order; the first match wins. The source may be another mod's fluid or `minecraft:lava`.
    fn fluid_interaction(&mut self, interaction: crate::FluidInteraction);

    /// Attach `part`, of kind `kind`, to the world fluid `fluid`.
    fn attach_fluid_part(&mut self, fluid: &str, kind: &'static str, part: Arc<dyn FluidPart>);
}
