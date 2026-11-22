use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Collect the mods' map decoration types once at startup on each side.
    pub(in crate::game) fn native_map_decoration_types(
        context: &mod_api::RegistrationContext,
        types: &mut Vec<MapDecorationType>,
    ) {
        Self::fire_map_decoration_types(context, types);
    }
}
