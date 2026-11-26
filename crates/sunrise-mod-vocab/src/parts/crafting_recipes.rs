//! Startup registration and native values for mod crafting recipe kinds.
use super::*;
use crate::runtime::RetainedCallback;

/// An owned, untrimmed crafting grid. Cells are row-major; empty cells are None.
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct CraftingGrid {
    /// Grid width in cells.
    pub width: u32,
    /// Grid height in cells.
    pub height: u32,
    /// Exactly width times height cells.
    pub cells: Vec<Option<crate::ItemStack>>,
}
impl CraftingGrid {
    /// Retain the row-major cells when their count equals the checked area. A zero
    /// area keeps the supplied dimensions and has no cells.
    pub fn new(
        width: u32,
        height: u32,
        cells: Vec<Option<crate::ItemStack>>,
    ) -> Result<Self, String> {
        let Some(expected) = width
            .checked_mul(height)
            .and_then(|n| usize::try_from(n).ok())
        else {
            return Err("invalid crafting grid dimensions".into());
        };
        if cells.len() != expected {
            return Err("invalid crafting grid dimensions".into());
        }
        Ok(Self {
            width,
            height,
            cells,
        })
    }
}

// no wire: borrowed per-craft reader; its owning loop and callback retain its authority.
/// The frozen loop view plus the engine's stack-aware default for each input cell.
/// Only `remaining_items` receives this view. [`LoopContext::items_in_tag`](crate::LoopContext::items_in_tag)
/// borrows this generation's bound item order; a default remainder is an owned stack and may
/// allocate for component data. Reads perform no file I/O.
pub trait CraftingRemainders: crate::LoopContext {
    /// Default remainder for this row-major input cell's original pre-take stack,
    /// components included.
    /// Empty or out-of-range cells, and items without a remainder, answer None.
    /// This is the server's default rule including crafting_remainder Parts; client prediction
    /// uses its existing data-only default and accepts the server's correction.
    fn default_remainder(&self, cell: u32) -> Option<crate::ItemStack>;
}

/// A mod's JSON crafting serializer and pure matching callbacks.
///
/// Use `register_crafting_recipe_serializer` in `Hooks::register`. The loader
/// retains each decoded value with its owning serializer for the recipe generation.
/// The value stays in process and needs no `Wire` implementation. Refuse invalid
/// inputs through the callback's return value rather than panicking.
pub trait CraftingRecipeKind: Send + Sync + 'static {
    /// Mod-owned recipe data decoded once per generation and borrowed by callbacks.
    type Value: Send + Sync + 'static;
    /// Decode a recipe JSON object with its `type` field removed.
    /// Return an error to skip a malformed recipe without refusing other files.
    fn decode(&self, json: &str) -> Result<Self::Value, String>;
    /// Write an equivalent JSON object for Sunrise client synchronization.
    /// The game supplies the registered serializer ID as `type` and applies the packet limits.
    /// Return `Err` to omit this recipe without disabling the writer. A panic disables this
    /// serializer's writer for the session and reports the mod by name.
    fn write(&self, value: &Self::Value) -> Result<String, String>;
    /// Whether this value matches the owned grid snapshot. The context reads the
    /// owning frozen resource generation; unavailable tags should not match.
    fn matches(
        &self,
        context: &dyn crate::LoopContext,
        value: &Self::Value,
        grid: CraftingGrid,
    ) -> bool;
    /// Assemble the result after a successful match. Counts must be 1 through 99;
    /// unknown item keys and larger counts refuse the result.
    fn assemble(
        &self,
        context: &dyn crate::LoopContext,
        value: &Self::Value,
        grid: CraftingGrid,
    ) -> Option<crate::ItemStack>;
    /// One remainder per input cell; None uses the default item remainders.
    /// Asked once per server take, with the grid before consumption. Each count must
    /// be 1 through 99. An invalid key, count or vector length uses defaults.
    fn remaining_items(
        &self,
        context: &dyn CraftingRemainders,
        value: &Self::Value,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>> {
        let _ = (context, value, grid);
        None
    }
}

// no wire: process-local registration owns a native JSON decoder.
// exhaustive: registration owns the opaque decoder.
/// Engine-owned registration of a recipe serializer.
#[derive(Clone)]
pub struct RegisteredRecipeSerializer(Arc<dyn RecipeDecoder>);

impl RegisteredRecipeSerializer {
    /// Bind the native serializer to the declaring mod and the registry's validated ID.
    pub fn bind(self, owner: &'static str, index: usize, id: &str) -> InstalledRecipeSerializer {
        InstalledRecipeSerializer {
            decoder: self.0,
            writer: RetainedCallback::new(owner, index, "recipe_serializer_write", id),
        }
    }
}

// no wire: a registered serializer retains process-local code and the owning mod's dispatch slot.
// exhaustive: only the registry binds a serializer to its owner.
/// Engine-owned serializer and its bound write callback.
#[derive(Clone)]
pub struct InstalledRecipeSerializer {
    decoder: Arc<dyn RecipeDecoder>,
    writer: RetainedCallback,
}

impl InstalledRecipeSerializer {
    /// Decode a compact JSON object whose type field the loader has removed.
    pub fn decode(&self, json: &str) -> Result<LoadedCraftingRecipe, String> {
        Ok(LoadedCraftingRecipe {
            recipe: Arc::clone(&self.decoder).load(json)?,
            writer: self.writer.clone(),
        })
    }
}

trait RecipeDecoder: Send + Sync {
    fn load(self: Arc<Self>, json: &str) -> Result<Arc<dyn RecipeDispatch>, String>;
}

impl<K: CraftingRecipeKind> RecipeDecoder for K {
    fn load(self: Arc<Self>, json: &str) -> Result<Arc<dyn RecipeDispatch>, String> {
        let value = self.decode(json)?;
        Ok(Arc::new(TypedRecipe { kind: self, value }))
    }
}

// no wire: a recipe and its serializer are native values; only their JSON is synced.
// exhaustive: only decoding constructs the opaque serializer and retained value pair.
/// Engine-owned dispatch to one loaded recipe and its owning serializer.
#[derive(Clone)]
pub struct LoadedCraftingRecipe {
    recipe: Arc<dyn RecipeDispatch>,
    writer: RetainedCallback,
}

impl LoadedCraftingRecipe {
    /// Write the retained value for synchronization, or skip it when shared dispatch refused
    /// the invocation, reported a panic or already disabled the writer.
    pub fn write(&self) -> Result<Option<String>, String> {
        self.writer.call(|| self.recipe.write()).ok().transpose()
    }

    /// Test the retained value against the grid snapshot.
    pub fn matches(&self, context: &dyn crate::LoopContext, grid: CraftingGrid) -> bool {
        self.recipe.matches(context, grid)
    }

    /// Assemble the result for this recipe.
    pub fn assemble(
        &self,
        context: &dyn crate::LoopContext,
        grid: CraftingGrid,
    ) -> Option<crate::ItemStack> {
        self.recipe.assemble(context, grid)
    }

    /// Return custom remainders for the pre-take grid, or request defaults.
    pub fn remaining_items(
        &self,
        context: &dyn CraftingRemainders,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>> {
        self.recipe.remaining_items(context, grid)
    }
}

trait RecipeDispatch: Send + Sync {
    fn write(&self) -> Result<String, String>;
    fn matches(&self, context: &dyn crate::LoopContext, grid: CraftingGrid) -> bool;
    fn assemble(
        &self,
        context: &dyn crate::LoopContext,
        grid: CraftingGrid,
    ) -> Option<crate::ItemStack>;
    fn remaining_items(
        &self,
        context: &dyn CraftingRemainders,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>>;
}

struct TypedRecipe<K: CraftingRecipeKind> {
    kind: Arc<K>,
    value: K::Value,
}

impl<K: CraftingRecipeKind> RecipeDispatch for TypedRecipe<K> {
    fn write(&self) -> Result<String, String> {
        self.kind.write(&self.value)
    }

    fn matches(&self, context: &dyn crate::LoopContext, grid: CraftingGrid) -> bool {
        self.kind.matches(context, &self.value, grid)
    }

    fn assemble(
        &self,
        context: &dyn crate::LoopContext,
        grid: CraftingGrid,
    ) -> Option<crate::ItemStack> {
        self.kind.assemble(context, &self.value, grid)
    }

    fn remaining_items(
        &self,
        context: &dyn CraftingRemainders,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<crate::ItemStack>>> {
        self.kind.remaining_items(context, &self.value, grid)
    }
}

impl dyn Registrar + '_ {
    /// Register a crafting kind once at startup. Bare names use the declaring mod's
    /// namespace; explicit valid `namespace:path` keys keep their namespace. Duplicate
    /// names, built-in serializers and malformed ids refuse startup, naming the kind.
    pub fn register_crafting_recipe_serializer<K: CraftingRecipeKind>(
        &mut self,
        name: &str,
        kind: K,
    ) {
        let handler = typed_handler(kind);
        self.add_recipe_serializer(name, "minecraft:crafting", handler);
    }
}
/// Startup operations for crafting and read-only recipe serializers.
pub trait RecipeSerializersRegistrar {
    /// Register the engine's native serializer owner for a recipe type.
    /// Mods use `register_crafting_recipe_serializer` or `register_read_only_recipe_serializer`
    /// on `dyn Registrar` to supply a typed implementation.
    fn add_recipe_serializer(
        &mut self,
        name: &str,
        recipe_type: &str,
        handler: RegisteredRecipeSerializer,
    );
    /// Declare a namespaced read-only recipe type.
    fn register_recipe_type(&mut self, name: &str);
}

/// Enumerate enabled kinds at the cold synchronization boundary, including empty kinds.
pub(crate) fn installed_crafting_recipe_kinds() -> Vec<String> {
    let Some(installed) = installed() else {
        return Vec::new();
    };
    let mut kinds: Vec<_> = installed
        .keys
        .recipe_serializers
        .iter()
        .filter(|(_, (owner, recipe_type, _))| {
            crate::mod_index_on(*owner) && recipe_type == "minecraft:crafting"
        })
        .map(|(kind, _)| kind.to_string())
        .collect();
    kinds.sort();
    kinds
}

/// Resolve a serializer and its separately registered recipe type.
pub(crate) fn installed_recipe_serializer(
    name: &str,
) -> Option<(String, InstalledRecipeSerializer)> {
    let (owner, recipe_type, handler) = installed()?.keys.recipe_serializers.get(name)?;
    (crate::mod_index_on(*owner)
        && (recipe_type == "minecraft:crafting"
            || installed()?
                .keys
                .recipe_types
                .contains_key(recipe_type.as_str())))
    .then(|| (recipe_type.clone(), handler.clone()))
}

pub(super) fn typed_handler<K: CraftingRecipeKind>(kind: K) -> RegisteredRecipeSerializer {
    RegisteredRecipeSerializer(Arc::new(kind))
}

pub(crate) fn installed_recipe_serializers() -> Vec<(String, String)> {
    installed().map_or_else(Vec::new, |installed| {
        installed
            .keys
            .recipe_serializers
            .iter()
            .filter(|(_, (owner, _, _))| crate::mod_index_on(*owner))
            .map(|(id, (_, kind, _))| (id.to_string(), kind.clone()))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crafting_grid_accepts_empty_and_larger_dimensions_and_rejects_mismatches() {
        for (w, h) in [(0, 0), (0, u32::MAX), (u32::MAX, 0)] {
            let empty = CraftingGrid::new(w, h, Vec::new()).unwrap();
            assert_eq!((empty.width, empty.height, empty.cells.len()), (w, h, 0));
        }

        let large = CraftingGrid::new(4, 4, vec![None; 16]).unwrap();
        assert_eq!((large.width, large.height, large.cells.len()), (4, 4, 16));

        assert!(CraftingGrid::new(2, 2, vec![None; 3]).is_err());
        assert!(CraftingGrid::new(u32::MAX, u32::MAX, Vec::new()).is_err());
    }
}
