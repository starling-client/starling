use super::*;
/// What answers an entity capability for one entity type: given the entity and the lookup's
/// context (its [`EntityCapabilityKey`]'s `C`), its handler, or none to leave it to the next
/// provider. A closure is one.
pub trait EntityCapabilityProvider<T: ?Sized, C>: Send + Sync {
    /// The handler for `entity`, asked with `context`, or none.
    fn provide(&self, host: &mut dyn ServerHost, entity: EntityRef, context: C) -> Option<Box<T>>;
}
impl<T: ?Sized, C, F> EntityCapabilityProvider<T, C> for F
where
    F: Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, entity: EntityRef, context: C) -> Option<Box<T>> {
        self(host, entity, context)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// An entity capability provider, as the registrar keeps it.
pub type EntityProvider<T, C = ()> = dyn EntityCapabilityProvider<T, C>;

impl HookParts {
    pub(crate) fn entity<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn EntityPart>] {
        attached(&self.entities, id)
    }
}

/// Whether any mod registered a provider for this typed capability, for any entity type.
/// The game's own lookups skip work when no mod registered one.
pub fn has_entity_providers<T: ?Sized + 'static, C: 'static>(
    key: &EntityCapabilityKey<T, C>,
) -> bool {
    installed().is_some_and(|installed| {
        let keys = &installed.keys;
        keys.entity_capabilities.contains_key(key.name)
            && keys
                .entity_capability_metadata
                .get(key.name)
                .is_some_and(|known| known.binding.0 == key.binding().0)
    })
}

/// The providers mods registered for `key` and the type of `entity`, for the game's own
/// lookups (a hopper's): none where no mod registered one, so a vanilla container entity's
/// lookup costs one table check and allocates nothing, and the game needn't work out its own
/// answer. [`EntityProviders::ask`] then asks them, as the game's lookups for block entities do.
pub fn entity_providers<T: ?Sized + 'static, C: Copy + 'static>(
    key: &EntityCapabilityKey<T, C>,
    host: &dyn Host,
    entity: EntityRef,
    context: C,
) -> Option<EntityProviders<T, C>> {
    let by_kind = installed()?.keys.entity_capabilities.get(key.name)?;
    let kind = entity
        .kind(host)
        .and_then(crate::content::registry_keys::entity_type_name)?;
    let parts = by_kind
        .get(kind)
        .map(Vec::as_slice)
        .filter(|parts| !parts.is_empty())?;
    Some(EntityProviders {
        parts,
        entity,
        context,
        handler: PhantomData,
    })
}

// no wire: provider lookup borrows static capability dispatch tables and process-local type identity.
/// An entity's providers of one capability ([`entity_providers`]).
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityProviders<T: ?Sized + 'static, C: 'static> {
    pub(crate) parts: &'static [CapabilityPart<NativeCapabilityProvider>],
    pub(crate) entity: EntityRef,
    pub(crate) context: C,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static, C: Copy + 'static> EntityProviders<T, C> {
    pub(crate) fn answer<'host>(
        self,
        host: &mut (dyn ServerHost + 'host),
        game: impl FnOnce(&mut (dyn ServerHost + 'host)) -> bool,
    ) -> Answer<Box<T>> {
        let (entity, context) = (self.entity, self.context);
        ask_providers(
            host,
            self.parts.iter(),
            ("capability provider", "capability_provider"),
            |provider, host| {
                provider
                    .get::<Arc<EntityProvider<T, C>>>()?
                    .provide(host, entity, context)
            },
            game,
        )
    }

    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        answered(self.answer(host, |_| game), None)
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to the entity type `entity` (`minecraft:zombie`): its
    /// [`EntityPart::register_goals`] runs at every `Mob` construction of that
    /// type, spawning or loading, and its goals join the mob's selectors
    /// beside vanilla's. Its `general_entity_tick` runs for one general row
    /// of the type (#792).
    pub fn entity_part<P: EntityPart>(&mut self, entity: &str, part: P) {
        self.attach_entity_part(entity, type_name::<P>(), Arc::new(part));
    }
    /// [`Self::entity_part`] for a Part with typed state (`EntityPartState`): each entity
    /// of type `entity` keeps one `P::Data`, under `part.state_key()`. Refused, and reported
    /// as the mod's registration refusal, when the key syncs, copies on death, or was not
    /// issued in this registration pass. The entity type is checked once every type is known,
    /// when the game loads: only mob types (vanilla and mod) and mod general entity types keep
    /// Part state, and `minecraft:player`, any other vanilla family and an unknown id refuse
    /// the load, naming the type. `entity_part` itself checks no type.
    pub fn entity_part_with_state<P: crate::EntityPartState>(
        &mut self,
        entity: &str,
        part: P,
    ) -> Result<(), crate::EntityPartStateError> {
        self.declare_entity_part_state(
            entity,
            type_name::<P>(),
            crate::PartStateDeclaration::of(part.state_key()),
        )?;
        self.entity_part(entity, part);
        Ok(())
    }
    /// Answer entity capability `key` for entity type `kind` (`mymod:robot`,
    /// `minecraft:player`) with `provider`, which gives a handler for one entity, or none:
    /// `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |_, robot, ()| Some(Box::new(Cargo(robot))))`.
    /// Asked as `block_entity_capability`'s are. The capability name and entity type id must be
    /// fully qualified resource identifiers. The entity type can be declared later in
    /// registration; an unknown type refuses load. A key with a different handler type `T` or
    /// context type `C` from the canonical binding refuses registration.
    pub fn entity_capability<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<EntityProvider<T, C>> = Arc::new(provider);
        self.provide_entity_capability(
            key.name,
            kind,
            false,
            NativeCapabilityProvider::of::<T, _>(provider),
        );
    }
    /// `entity_capability`, asked ahead of the game's own provider,
    /// as a mod that replaces how a player's inventory is reached.
    pub fn entity_capability_before_game<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<EntityProvider<T, C>> = Arc::new(provider);
        self.provide_entity_capability(
            key.name,
            kind,
            true,
            NativeCapabilityProvider::of::<T, _>(provider),
        );
    }
}

/// Registrar operations for entities.
pub trait EntitiesRegistrar {
    /// Attach `part`, of kind `kind`, to the entity type `entity`
    /// (`minecraft:zombie`, `mymod:golem`): its `register_goals` runs at every
    /// `Mob` construction of that type, and its `general_entity_tick` for one
    /// general row's authoritative tick (#792).
    fn attach_entity_part(&mut self, entity: &str, kind: &'static str, part: Arc<dyn EntityPart>);

    /// Record the state declaration of entity Part `part` on entity type `entity`, or refuse
    /// it (as the mod's registration refusal too): see `PartStateDeclaration::check` and
    /// `PartStateRefusal::StaleKey`. Mods reach it through `entity_part_with_state`.
    fn declare_entity_part_state(
        &mut self,
        entity: &str,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::EntityPartStateError>;

    /// Answer entity capability `name` for entity type `kind` with `provider`; mods reach it
    /// through `entity_capability`.
    fn provide_entity_capability(
        &mut self,
        name: &'static str,
        kind: &str,
        before_game: bool,
        provider: NativeCapabilityProvider,
    );
}
