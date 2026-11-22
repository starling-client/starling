/// Registration requests for the level's damage-type registry.
pub trait DamageTypesRegistrar {
    /// Request a damage type by registry ID. Registration retains the name, not a
    /// world's registry index. A contact resolves it in the level where it runs;
    /// a world without that type does not apply the hurt.
    fn damage_type_key(&mut self, id: &str) -> crate::DamageTypeKey;
}
