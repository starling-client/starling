use super::*;
/// The structure types registered at startup, for the executable to hand to
/// the engine's `StructureRegistry`.
pub(crate) fn installed_structure_type_list()
-> &'static [crate::world::structure_type::StructureTypeInfo] {
    installed().map_or(&[], |installed| installed.structure_types.as_slice())
}

/// The pool element types registered at startup, for the executable to hand
/// to the engine's element type table.
pub(crate) fn installed_pool_element_type_list()
-> &'static [crate::world::pool_element::PoolElementTypeInfo] {
    installed().map_or(&[], |installed| installed.pool_element_types.as_slice())
}

/// The structure processor types registered at startup, for the executable to
/// hand to the engine's processor type table.
pub(crate) fn installed_structure_processor_type_list()
-> &'static [crate::world::structure_processor::StructureProcessorTypeInfo] {
    installed().map_or(&[], |installed| {
        installed.structure_processor_types.as_slice()
    })
}

/// The rule test types registered at startup, for the executable to hand to
/// the engine's rule test type table.
pub(crate) fn installed_rule_test_type_list() -> &'static [crate::world::rule_test::RuleTestTypeInfo]
{
    installed().map_or(&[], |installed| installed.rule_test_types.as_slice())
}

/// The position rule test types registered at startup, for the executable to
/// hand to the engine's position rule test type table.
pub(crate) fn installed_pos_rule_test_type_list()
-> &'static [crate::world::rule_test::PosRuleTestTypeInfo] {
    installed().map_or(&[], |installed| installed.pos_rule_test_types.as_slice())
}

impl dyn Registrar + '_ {
    /// Register structure type `name` (`tower` or `mymod:tower`), the way
    /// NeoForge's registries `STRUCTURE_TYPE` and `STRUCTURE_PIECE` register
    /// once at startup: a data pack structure with `"type": "modid:tower"`
    /// then generates by `ty`'s code — its own start and pieces, placed by the
    /// game's own spacing, biome and terrain-adaptation checks. A bare name uses
    /// this mod's namespace; an explicit valid `namespace:path` keeps its
    /// namespace. A malformed name, a built-in structure type and an id
    /// registered twice refuse the mods at startup, naming the mod. `ty`'s
    /// `load_piece` is its saved pieces' loader: keep the type for as long as the
    /// game runs.
    pub fn register_structure_type<T: crate::StructureType>(&mut self, name: &str, ty: T) {
        self.add_structure_type(name, Arc::new(ty));
    }
    /// Register pool element type `name` (`corridor` or `mymod:corridor`),
    /// the way NeoForge's registry `STRUCTURE_POOL_ELEMENT` registers once at
    /// startup: a template pool entry with `"element_type": "modid:corridor"`
    /// then decodes, lays out and places by `ty`'s code, in any structure
    /// that draws from that pool. A bare name uses this mod's namespace; an
    /// explicit valid `namespace:path` keeps its namespace. A malformed name and
    /// an id registered twice refuse the mods at startup, naming the mod.
    pub fn register_pool_element_type<T: crate::PoolElementType>(&mut self, name: &str, ty: T) {
        self.add_pool_element_type(name, Arc::new(ty));
    }
    /// Register structure processor type `name` (`weathering` or
    /// `mymod:weathering`), the way NeoForge's registry `STRUCTURE_PROCESSOR`
    /// registers once at startup: a processor list entry with
    /// `"processor_type": "modid:weathering"` then decodes and runs by `ty`'s
    /// code beside vanilla's processors. A bare name uses this mod's
    /// namespace; an explicit valid `namespace:path` keeps its namespace. A
    /// malformed name and an id registered twice refuse the mods at startup,
    /// naming the mod.
    pub fn register_structure_processor_type<T: crate::StructureProcessorType>(
        &mut self,
        name: &str,
        ty: T,
    ) {
        self.add_structure_processor_type(name, Arc::new(ty));
    }
    /// Register rule test type `name` (`matter_phase` or
    /// `mymod:matter_phase`), the way NeoForge's registry `RULE_TEST`
    /// registers once at startup: a processor rule or ore/replacement target
    /// entry with `"predicate_type": "modid:matter_phase"` then decodes once
    /// at load and runs by `ty`'s code beside vanilla's built-in rule tests.
    /// A bare name uses this mod's namespace; an explicit valid
    /// `namespace:path` keeps its namespace. A malformed name and an id
    /// registered twice refuse the mods at startup, naming the mod.
    pub fn register_rule_test_type<T: crate::RuleTestType>(&mut self, name: &str, ty: T) {
        self.add_rule_test_type(name, Arc::new(ty));
    }
    /// Register position rule test type `name` (`y_value` or
    /// `mymod:y_value`), the way NeoForge's registry `POS_RULE_TEST` registers
    /// once at startup: a processor rule's `position_predicate` entry with
    /// `"predicate_type": "modid:y_value"` then decodes once at load and runs
    /// by `ty`'s code beside vanilla's built-in position rule tests. A bare
    /// name uses this mod's namespace; an explicit valid `namespace:path` keeps
    /// its namespace. A malformed name and an id registered twice refuse the
    /// mods at startup, naming the mod.
    pub fn register_pos_rule_test_type<T: crate::PosRuleTestType>(&mut self, name: &str, ty: T) {
        self.add_pos_rule_test_type(name, Arc::new(ty));
    }
}

/// Registrar operations for structures.
pub trait StructuresRegistrar {
    /// Register structure type `name` with `ty`; mods reach it through
    /// `register_structure_type`.
    fn add_structure_type(&mut self, name: &str, ty: Arc<dyn crate::StructureType>);
    /// Register pool element type `name` with `ty`; mods reach it through
    /// `register_pool_element_type`.
    fn add_pool_element_type(&mut self, name: &str, ty: Arc<dyn crate::PoolElementType>);
    /// Register structure processor type `name` with `ty`; mods reach it
    /// through `register_structure_processor_type`.
    fn add_structure_processor_type(
        &mut self,
        name: &str,
        ty: Arc<dyn crate::StructureProcessorType>,
    );
    /// Register rule test type `name` with `ty`; mods reach it through
    /// `register_rule_test_type`.
    fn add_rule_test_type(&mut self, name: &str, ty: Arc<dyn crate::RuleTestType>);
    /// Register position rule test type `name` with `ty`; mods reach it
    /// through `register_pos_rule_test_type`.
    fn add_pos_rule_test_type(&mut self, name: &str, ty: Arc<dyn crate::PosRuleTestType>);
}
