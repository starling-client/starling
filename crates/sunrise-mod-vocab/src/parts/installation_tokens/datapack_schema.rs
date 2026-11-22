//! Pure value validation for the owning API registration boundary.
/// Check the schema without exposing its private implementation module.
pub fn validate_datapack_schema(
    fields: &[crate::DatapackField],
) -> Result<(), crate::DatapackRegistrationError> {
    crate::world::datapack_registry::check_schema(fields)
}
/// Normalise the schema's id and reference defaults at every depth (map keys included), as a
/// top-level default is.
pub fn normalize_datapack_defaults(
    fields: &mut [crate::DatapackField],
) -> Result<(), crate::DatapackRegistrationError> {
    crate::world::datapack_registry::normalize_defaults(fields)
}
/// Check a registry identity using the same rules as field references.
pub fn valid_datapack_id(value: &str) -> bool {
    crate::world::datapack_registry::valid_namespaced(value)
}
