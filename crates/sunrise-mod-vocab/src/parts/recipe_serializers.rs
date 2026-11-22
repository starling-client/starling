//! Read-only serializers share registration and native values with crafting serializers.
use super::*;
/// A JSON serializer for a separately registered recipe type that the mod reads.
///
/// Use `register_read_only_recipe_serializer` in `Hooks::register`. The game
/// publishes the source JSON through `recipes_of_type` and never matches it.
/// The loader retains each decoded value with its owning serializer for the
/// recipe generation. The value stays in process and needs no `Wire` implementation.
pub trait RecipeSerializer: Send + Sync + 'static {
    /// Mod-owned recipe data decoded once per generation and borrowed when writing JSON.
    type Value: Send + Sync + 'static;
    /// Decode a recipe JSON object with its `type` field removed.
    /// Return an error to skip a malformed recipe without refusing other files.
    fn decode(&self, json: &str) -> Result<Self::Value, String>;
    /// Write an equivalent JSON object for Sunrise client synchronization.
    /// The game supplies the registered serializer ID as `type` and applies the packet limits.
    /// Return `Err` to omit this recipe without disabling the writer. A panic disables this
    /// serializer's writer for the session and reports the mod by name.
    fn write(&self, value: &Self::Value) -> Result<String, String>;
}
struct ReadOnly<S>(S);
impl<S: RecipeSerializer> crate::CraftingRecipeKind for ReadOnly<S> {
    type Value = S::Value;
    fn decode(&self, json: &str) -> Result<Self::Value, String> {
        self.0.decode(json)
    }
    fn write(&self, value: &Self::Value) -> Result<String, String> {
        self.0.write(value)
    }
    fn matches(&self, _: &dyn crate::LoopContext, _: &Self::Value, _: crate::CraftingGrid) -> bool {
        false
    }
    fn assemble(
        &self,
        _: &dyn crate::LoopContext,
        _: &Self::Value,
        _: crate::CraftingGrid,
    ) -> Option<crate::ItemStack> {
        None
    }
}
struct Raw;
impl RecipeSerializer for Raw {
    type Value = String;
    fn decode(&self, json: &str) -> Result<String, String> {
        Ok(json.to_owned())
    }
    fn write(&self, value: &String) -> Result<String, String> {
        Ok(value.clone())
    }
}
impl dyn Registrar + '_ {
    /// Register a serializer for an independently declared read-only recipe type.
    pub fn register_read_only_recipe_serializer<S: RecipeSerializer>(
        &mut self,
        name: &str,
        recipe_type: &str,
        serializer: S,
    ) {
        let kind = ReadOnly(serializer);
        let handler = crate::parts::crafting_recipes::typed_handler(kind);
        self.add_recipe_serializer(name, recipe_type, handler);
    }
    /// Register an object-preserving JSON serializer for a mod-read machine recipe type.
    /// Declare `recipe_type` with `register_recipe_type` at startup, in either order.
    pub fn register_raw_recipe_serializer(&mut self, name: &str, recipe_type: &str) {
        self.register_read_only_recipe_serializer(name, recipe_type, Raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_recipe_accepts_json_larger_than_one_mib() {
        let padding = "x".repeat(1024 * 1024 + 64);
        let large = format!(r#"{{"value":"{padding}"}}"#);
        assert_eq!(Raw.decode(&large).unwrap(), large);
    }
}
