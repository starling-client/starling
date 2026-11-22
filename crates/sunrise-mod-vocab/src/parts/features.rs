use super::*;
/// The feature types registered at startup, for the executable to hand to the engine's feature
/// decoding.
pub(crate) fn installed_feature_type_list() -> &'static [crate::world::feature_type::FeatureTypeInfo]
{
    installed().map_or(&[], |installed| installed.feature_types.as_slice())
}

impl dyn Registrar + '_ {
    /// Register feature type `name` (`slime_tree` or `mymod:slime_tree`), the way NeoForge's
    /// registry `FEATURE_TYPE` registers once at startup: a configured feature with
    /// `"type": "modid:slime_tree"` then places by `ty`'s code, wherever the game places a
    /// feature (decoration, nested features, growth, bone meal, `/place feature`).
    ///
    /// `config` is the configured feature's settings as a flat record, the schema datapack
    /// registries use: the game decodes each configured feature against it when the definition
    /// loads and refuses one that doesn't fit, naming it. A `Reference` field is refused; name
    /// other entries with `Identifier`.
    ///
    /// The type's ID must be in a namespace owned by the calling mod's manifest. An unowned
    /// namespace (including vanilla's), an empty name, a duplicate ID or an invalid schema
    /// refuses registration at startup and names the mod.
    pub fn register_feature_type<T: crate::FeatureType>(
        &mut self,
        name: &str,
        config: Vec<crate::DatapackField>,
        ty: T,
    ) {
        self.add_feature_type(name, config, Arc::new(ty));
    }
}

/// Registrar operations for features.
pub trait FeaturesRegistrar {
    /// Register feature type `name` with its config schema and `ty`; mods reach it through
    /// `register_feature_type`. Feature calls run on the worldgen workers.
    fn add_feature_type(
        &mut self,
        name: &str,
        config: Vec<crate::DatapackField>,
        ty: Arc<dyn crate::FeatureType>,
    );
}
