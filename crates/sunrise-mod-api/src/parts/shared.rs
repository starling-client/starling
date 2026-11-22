//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

pub(crate) fn insert_part<P: ?Sized>(
    table: &mut HashMap<Box<str>, Vec<Attached<P>>>,
    id: &str,
    part: Attached<P>,
    refused: &mut Vec<String>,
) -> bool {
    let list = table.entry(id.into()).or_default();
    if let Some(other) = list.iter().find(|other| other.type_name == part.type_name) {
        refused.push(format!(
            "{} is attached to {id} twice, by {} and {}",
            part.type_name, other.mod_id, part.mod_id
        ));
        return false;
    }
    list.push(part);
    true
}

thread_local! {
    // A reference to the registrar's canonical requests while Hooks::register is in flight.
    // The table is moved, not copied, into Installed when registration finishes.
    pub(crate) static REGISTERING_KEYS: RefCell<Option<Rc<RefCell<sunrise_mod_runtime::registry_keys::Requests>>>> = const { RefCell::new(None) };
}

pub(crate) fn with_registering_keys<T>(
    read: impl FnOnce(Option<&sunrise_mod_runtime::registry_keys::Requests>) -> T,
) -> T {
    REGISTERING_KEYS.with(|keys| match keys.borrow().as_ref() {
        Some(keys) => read(Some(&keys.borrow())),
        None => read(None),
    })
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// The game's [`Registrar`]: every mod's registrations, checked as they come.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PartRegistrar {
    pub(crate) biome_modifiers: Vec<BiomeModifierDeclaration>,
    pub(crate) biome_regions: Vec<BiomeRegionDeclaration>,
    pub(crate) datapack_registries: Vec<crate::DatapackRegistryDeclaration>,
    pub(crate) consume_effects: Vec<RegisteredConsumeEffect>,
    pub(crate) menu_screen_factories: HashMap<String, MenuScreenRegistration>,
    pub(crate) menu_recipe_books: HashMap<String, MenuBookDeclaration>,
    /// Loading warnings for each owning mod, in registration order.
    pub(crate) warnings: Vec<(&'static str, Vec<crate::LoadingMessage>)>,
    /// Refusal reasons recorded for the mod currently running `Hooks::register`.
    pub(crate) current_refusals: Vec<String>,
    /// Loading warnings recorded for the mod currently running `Hooks::register`.
    pub(crate) current_warnings: Vec<crate::LoadingMessage>,
    pub(crate) recipe_book_types: HashMap<String, String>,
    pub(crate) recipe_book_fields: HashMap<String, (String, String)>,
    pub(crate) recipe_book_filters: HashMap<String, crate::RecipeBookFilterLook>,
    pub(crate) game_rule_categories: Vec<String>,
    pub(crate) game_rules: Vec<crate::GameRuleDecl>,
    pub(crate) attachment_uses: Vec<(String, String, String)>,
    pub(crate) attachment_types: HashMap<String, HashMap<String, bool>>,
    pub(crate) attachment_owners: HashMap<String, HashMap<String, &'static str>>,
    pub(crate) custom_stats: Vec<String>,
    pub(crate) custom_stat_owners: Vec<&'static str>,
    /// The entity types each named to `entity_tick_each`, in order: `None` for every type.
    pub(crate) entity_tick_each: Vec<Option<String>>,
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// Parts registered so far, by every mod: the next part's `order`.
    pub(crate) registered: usize,
    pub(crate) parts: Parts,
    /// Aggregate selector metadata bytes during this registration pass.
    pub(crate) entity_selector_bytes: usize,
    pub(crate) tags: TagParts,
    pub(crate) block_entity_types: Vec<(&'static str, BlockEntityTypeDecl)>,
    pub(crate) block_entity_transforms: HashMap<String, Arc<dyn crate::BlockEntityTransform>>,
    pub(crate) entity_attributes: Vec<EntityAttributeDeclaration>,
    pub(crate) world_fluids: Vec<(&'static str, (String, crate::WorldFluid))>,
    pub(crate) fluid_interactions: Vec<crate::FluidInteraction>,
    pub(crate) entity_attribute_modifications: Vec<EntityAttributeModification>,
    pub(crate) attributes: Vec<AttributeDeclaration>,
    pub(crate) attribute_widenings: Vec<AttributeBoundsWidening>,
    pub(crate) general_entity_types: Vec<GeneralEntityTypeDeclaration>,
    pub(crate) spawn_placements: Vec<SpawnPlacementDeclaration>,
    /// The entity Part state declarations, for the game to check their entity types.
    pub(crate) entity_part_states: Vec<mod_vocab::__private::EntityPartStateDeclaration>,
    pub(crate) registry_keys: Rc<RefCell<sunrise_mod_runtime::registry_keys::Requests>>,
    pub(crate) effects: OnceLock<sunrise_mod_runtime::registry_keys::Effects>,
    pub(crate) controllers: Vec<ControllerInfo>,
    pub(crate) arm_poses: Vec<PoseInfo>,
    pub(crate) structure_types: Vec<StructureTypeInfo>,
    pub(crate) feature_types: Vec<FeatureTypeInfo>,
    pub(crate) loot_modifier_serializers: Vec<LootModifierSerializerInfo>,
    pub(crate) loot_function_types: Vec<LootFunctionTypeInfo>,
    pub(crate) loot_condition_types: Vec<LootConditionTypeInfo>,
    pub(crate) dimensions: Vec<DimensionInfo>,
    pub(crate) dimension_shape: fn(&crate::DimensionSettings) -> bool,
    pub(crate) chunk_generators: Vec<ChunkGeneratorInfo>,
    pub(crate) biome_sources: Vec<BiomeSourceInfo>,
    pub(crate) refused: Vec<String>,
    /// The Part state keys this mod declared, with their type names.
    pub(crate) part_states: HashMap<(&'static str, &'static str), &'static str>,
}

impl PartRegistrar {
    pub(crate) fn new(mod_id: &'static str) -> Self {
        let _ = crate::__private::initialize();
        Self {
            menu_screen_factories: HashMap::new(),
            menu_recipe_books: HashMap::new(),
            warnings: Vec::new(),
            current_refusals: Vec::new(),
            current_warnings: Vec::new(),
            biome_modifiers: Vec::new(),
            biome_regions: Vec::new(),
            datapack_registries: Vec::new(),
            consume_effects: Vec::new(),
            recipe_book_types: HashMap::new(),
            recipe_book_fields: HashMap::new(),
            recipe_book_filters: HashMap::new(),
            game_rule_categories: Vec::new(),
            game_rules: Vec::new(),
            attachment_uses: Vec::new(),
            attachment_types: HashMap::new(),
            attachment_owners: HashMap::new(),
            custom_stats: Vec::new(),
            custom_stat_owners: Vec::new(),
            entity_tick_each: Vec::new(),
            mod_id,
            mod_index: 0,
            registered: 0,
            parts: Self::capability_parts(),
            entity_selector_bytes: 0,
            tags: TagParts::default(),
            block_entity_types: Vec::new(),
            block_entity_transforms: HashMap::new(),
            entity_attributes: Vec::new(),
            world_fluids: Vec::new(),
            fluid_interactions: Vec::new(),
            entity_attribute_modifications: Vec::new(),
            attributes: Vec::new(),
            attribute_widenings: Vec::new(),
            general_entity_types: Vec::new(),
            spawn_placements: Vec::new(),
            entity_part_states: Vec::new(),
            registry_keys: Default::default(),
            effects: OnceLock::new(),
            controllers: Vec::new(),
            arm_poses: Vec::new(),
            structure_types: Vec::new(),
            feature_types: Vec::new(),
            loot_modifier_serializers: Vec::new(),
            loot_function_types: Vec::new(),
            loot_condition_types: Vec::new(),
            dimensions: Vec::new(),
            dimension_shape: |_| false,
            chunk_generators: Vec::new(),
            biome_sources: Vec::new(),
            refused: Vec::new(),
            part_states: HashMap::new(),
        }
    }
    /// `part`, of kind `kind`, for tag `tag`; none if it is refused.
    pub(crate) fn tagged<P: ?Sized>(
        &mut self,
        kind: &'static str,
        tag: &str,
        part: Arc<P>,
    ) -> Option<(Box<str>, Attached<P>)> {
        if tag.starts_with('#') || !namespaced(tag) {
            self.refused.push(format!(
                "{} attached {kind} to tag {tag:?}, which is not a namespaced tag name (write it without #)",
                self.mod_id
            ));
            return None;
        }
        self.registered += 1;
        Some((
            tag.into(),
            Attached {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                order: self.registered,
                type_name: kind,
                part,
            },
        ))
    }

    /// Attach `part`, of kind `kind`, to `id` in `table`; false if it is refused.
    pub(crate) fn attach<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        if !namespaced(id) {
            self.refused.push(format!(
                "{} attached {kind} to {id:?}, which is not a namespaced id",
                self.mod_id
            ));
            return false;
        }
        self.attach_known(table, kind, id, part)
    }

    pub(crate) fn attach_known<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        let part = Attached {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.registered + 1,
            type_name: kind,
            part,
        };
        let inserted = insert_part(table(&mut self.parts), id, part, &mut self.refused);
        if inserted {
            self.registered += 1;
        }
        inserted
    }
}

pub(crate) fn namespaced(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

/// Every mod's [`Hooks::register`](crate::Hooks::register) run against one registrar, in dispatch order.
pub(crate) fn register_all(
    mods: &[ModEntry],
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> PartRegistrar {
    let mut registrar = PartRegistrar::new(mods.first().map_or("sunrise", |entry| {
        mod_vocab::__private::parts_installation::mod_entry_id(entry)
    }));
    registrar.dimension_shape = dimension_shape;
    struct RestoreKeys(Option<Rc<RefCell<sunrise_mod_runtime::registry_keys::Requests>>>);
    impl Drop for RestoreKeys {
        fn drop(&mut self) {
            REGISTERING_KEYS.set(self.0.take());
        }
    }
    let _active = ActiveRegistration::enter();
    // `register` runs with startup registration open: a declaration of a frozen registry
    // (a general entity type) is refused once it closes.
    let _registration = crate::__private::RegistrationGuard::open();
    let _scope = RestoreKeys(REGISTERING_KEYS.replace(Some(registrar.registry_keys.clone())));
    let context =
        <RegistrationContext as mod_vocab::__private::game::RegistrationContextNew>::new();
    for (index, hooks) in mod_vocab::__private::entered_registration(mods, "register").enumerate() {
        registrar.mod_id = mod_vocab::__private::parts_installation::mod_entry_id(&mods[index]);
        registrar.mod_index = index;
        registrar.current_warnings.clear();
        let first_refusal = registrar.refused.len();
        let rollback = registrar.registration_checkpoint();
        let cx = mod_vocab::__private::game::bind_registration_context(&context, registrar.mod_id);
        let completed = mod_vocab::__private::invoke_registration(registrar.mod_id, || {
            hooks.register(&mut registrar, &cx);
        });
        let refusals = std::mem::take(&mut registrar.current_refusals);
        if !completed {
            // Report the failed batch's ordinary refusals without turning the contained
            // panic into a fatal admission error for all the other registrations.
            for reason in registrar.refused[first_refusal..].iter().chain(&refusals) {
                log::warn!("mod {} hook register refused: {reason}", registrar.mod_id);
            }
            rollback(&mut registrar);
            continue;
        }
        let warnings = std::mem::take(&mut registrar.current_warnings);
        if !warnings.is_empty() {
            registrar.warnings.push((registrar.mod_id, warnings));
        }
        if !refusals.is_empty() {
            rollback(&mut registrar);
            for reason in refusals {
                registrar.refused.push(format!(
                    "mod {} refused registration: {reason}",
                    registrar.mod_id
                ));
            }
        }
    }
    registrar.expand_block_entity_parts();
    registrar.refuse_unknown_generators_and_biome_sources();
    registrar
}

/// Builds the parts from every mod's [`Hooks::register`](crate::Hooks::register), in dispatch order. Once, at startup;
/// the error lists every part refused, naming its mods.
pub fn install_parts(
    mods: &[ModEntry],
    schema: &mod_vocab::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> Result<(), Vec<String>> {
    install_registrar(register_all(mods, dimension_shape), schema)
}

pub(crate) fn install_registrar(
    mut registrar: PartRegistrar,
    schema: &mod_vocab::__private::RuleSchema<'_>,
) -> Result<(), Vec<String>> {
    let slots = schema.validate(&registrar.game_rule_categories, &registrar.game_rules);
    if let Err(errors) = &slots {
        registrar.refused.extend(errors.iter().cloned());
    }
    for (id, warnings) in &registrar.warnings {
        for warning in warnings {
            log::warn!(
                "mod {id} loading warning: {} {:?}",
                warning.key(),
                warning.args()
            );
        }
    }
    registrar.validate_recipe_types();
    registrar.skip_absent_effect_parts();
    if !registrar.refused.is_empty() {
        return Err(registrar.refused);
    }
    registrar.validate_attachment_admission()?;
    let game_rule_slots = slots.expect("every schema refusal was collected");
    let installed = Installed {
        dependencies: crate::load_state::effective_dependencies(),
        effect_ticks: Default::default(),
        biome_modifiers: registrar.biome_modifiers,
        biome_regions: {
            let mut regions = registrar.biome_regions;
            regions.sort_by(|a, b| (&a.mod_id, &a.id).cmp(&(&b.mod_id, &b.id)));
            regions
        },
        datapack_registries: registrar.datapack_registries,
        consume_effects: registrar.consume_effects,
        menu_screen_factories: registrar.menu_screen_factories,
        menu_recipe_books: registrar.menu_recipe_books,
        recipe_book_types: registrar.recipe_book_types,
        recipe_book_fields: registrar.recipe_book_fields,
        recipe_book_filters: registrar.recipe_book_filters,
        game_rule_slots,
        game_rules: registrar.game_rules,
        game_rule_categories: registrar.game_rule_categories,
        attachment_types: registrar.attachment_types,
        attachment_owners: registrar.attachment_owners,
        custom_stats: registrar.custom_stats,
        custom_stat_owners: registrar.custom_stat_owners,
        entity_tick_each: mod_vocab::__private::EntityTickEach::of(&registrar.entity_tick_each),
        keys: registrar.parts,
        tags: registrar.tags,
        block_entity_types: registrar.block_entity_types,
        block_entity_transforms: registrar.block_entity_transforms,
        entity_attributes: registrar.entity_attributes,
        world_fluids: registrar.world_fluids,
        fluid_interactions: registrar.fluid_interactions,
        attributes: registrar.attributes,
        attribute_widenings: registrar.attribute_widenings,
        general_entity_types: registrar.general_entity_types,
        entity_attribute_modifications: registrar.entity_attribute_modifications,
        spawn_placements: registrar.spawn_placements,
        entity_part_states: registrar.entity_part_states,
        resolved_block_entity_types: OnceLock::new(),
        registry_keys: Rc::try_unwrap(registrar.registry_keys)
            .unwrap_or_else(|_| unreachable!("registration scope has ended"))
            .into_inner(),
        effects: registrar.effects,
        fluids: OnceLock::new(),
        resolved_items: OnceLock::new(),
        resolved_entity_types: OnceLock::new(),
        resolved_game_events: OnceLock::new(),
        resolved_blocks: OnceLock::new(),
        particles: OnceLock::new(),
        controllers: registrar.controllers,
        arm_poses: registrar.arm_poses,
        structure_types: registrar.structure_types,
        feature_types: registrar.feature_types,
        loot_modifier_serializers: registrar.loot_modifier_serializers,
        loot_function_types: registrar.loot_function_types,
        loot_condition_types: registrar.loot_condition_types,
        dimensions: registrar.dimensions,
        chunk_generators: registrar.chunk_generators,
        biome_sources: registrar.biome_sources,
        warnings: registrar.warnings,
    };
    mod_vocab::__private::parts_installation::install(installed);

    Ok(())
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use super::super::items::tests::Cell;
    use super::mark_game_thread;
    use super::*;
    use crate::parts::tests::*;
    use crate::*;

    use std::sync::Arc;

    include!(concat!(env!("OUT_DIR"), "/shared_fixtures.rs"));
}
