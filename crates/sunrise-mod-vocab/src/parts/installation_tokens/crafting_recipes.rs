//! Cold engine lookup of registered crafting recipe owners.
use crate::__private::InstalledRecipeSerializer;

/// Enabled kind IDs, including kinds whose last recipe disappeared on reload.
pub fn crafting_recipe_kinds() -> Vec<String> {
    crate::parts::installed_crafting_recipe_kinds()
}

/// A registered serializer's produced recipe type and retained owner.
pub fn recipe_serializer(name: &str) -> Option<(String, InstalledRecipeSerializer)> {
    crate::parts::installed_recipe_serializer(name)
}

/// All enabled serializers and the independently registered types they produce.
pub fn recipe_serializers() -> Vec<(String, String)> {
    crate::parts::installed_recipe_serializers()
}
