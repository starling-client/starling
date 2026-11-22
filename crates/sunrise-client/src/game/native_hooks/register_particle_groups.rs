use super::*;

impl Game<'_> {
    /// Gather attributed group factories at startup, before the client builds particle engines.
    pub(in crate::game) fn native_register_particle_groups(
        context: &mod_api::RegistrationContext,
        groups: &mut Vec<Answered<ParticleGroupRegistration>>,
    ) {
        Self::fire_register_particle_groups(context, groups);
    }
}
