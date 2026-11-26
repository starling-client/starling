use super::*;

impl Game<'_> {
    /// Collect the mods' render pipeline, custom render type and compute pipeline registrations once at startup.
    pub(in crate::game) fn native_register_render_pipelines(
        context: &mod_api::RegistrationContext,
        pipelines: &mut Vec<mod_api::RenderPipeline>,
        render_types: &mut Vec<mod_api::RenderSetup>,
        compute_pipelines: &mut Vec<mod_api::ComputePipeline>,
    ) {
        Self::fire_register_render_pipelines(context, pipelines, render_types, compute_pipelines);
    }
}
