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

/// Whether any mod attached an entity part, so `Mob` construction asks for
/// goals, the mob tick builds a goal read view, and the general entity tick
/// asks its dispatch.
pub fn has_entity_parts() -> bool {
    !crate::mods().is_empty()
        && installed().is_some_and(|installed| !installed.keys.entities.is_empty())
}

/// The entity parts attached to entity type `id` (`minecraft:zombie`,
/// `mymod:robot`), in dispatch order, for the goal registration at every
/// `Mob` construction.
pub fn entity_parts() -> PartsRef {
    let Some(installed) = installed() else {
        return PartsRef::Shared(&NO_PARTS);
    };
    let (game_parts, game_thread) =
        GAME_PARTS.with_borrow(|lookup| (lookup.parts.clone(), lookup.game_thread));
    if let Some(parts) = game_parts {
        return PartsRef::Game(parts);
    }
    if !game_thread {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        if crate::dispatch_scope::backend()
            .is_none_or(|backend| (backend.diagnostics.first_off_thread_warning)())
        {
            log::warn!(
                "Native entity parts asked off a game thread ({thread}): a mob built there keeps no mod goals"
            );
        }
    }
    PartsRef::Shared(&installed.keys)
}

impl Parts {
    /// The entity parts attached to entity type `id` (`minecraft:zombie`), in
    /// dispatch order, for the goal registration at every `Mob` construction
    ///.
    pub fn entity_parts(&self, id: &str) -> &[Attached<dyn EntityPart>] {
        self.entities.get(id).map_or(&[], Vec::as_slice)
    }
}

impl HookParts {
    pub(crate) fn entity<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn EntityPart>] {
        attached(&self.entities, id)
    }
}

/// Whether any mod registered a provider of entity capability `key`, for any entity type: one
/// table lookup, no allocation, so the game's own lookups skip all their work where none did.
pub fn has_entity_providers<T: ?Sized + 'static, C: 'static>(
    key: &EntityCapabilityKey<T, C>,
) -> bool {
    installed()
        .and_then(|installed| installed.keys.entity_capabilities.get(key.name))
        .is_some_and(|by_kind| !by_kind.is_empty())
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
    let name = entity.kind(host)?.id(host)?;
    let parts = by_kind.get(name.as_str())?;
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
    pub(crate) parts: &'static [CapabilityPart<EntityProviderOf>],
    pub(crate) entity: EntityRef,
    pub(crate) context: C,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static, C: Copy + 'static> EntityProviders<T, C> {
    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let (entity, context) = (self.entity, self.context);
        let mut index = 0;
        match ask_providers(
            host,
            || next_provider(self.parts, &mut index),
            |provider, host| provider.get::<T, C>()?.provide(host, entity, context),
            |_| game,
        ) {
            Answer::Mod(handler, owner) => Some(Capability {
                handler,
                owner: Some(owner),
            }),
            Answer::Game | Answer::Nobody => None,
        }
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
    /// of type `entity` keeps one `P::Data`, under `P::DATA`. Refused, and reported as the
    /// mod's registration refusal, when `P::DATA` syncs, copies on death, holds a resource
    /// handle, is outside the mod's namespaces, or names a key this mod already declared with
    /// another type. The entity type is checked once every type is known, when the game
    /// loads: only mob types (vanilla and mod) and mod general entity types keep Part state,
    /// and `minecraft:player`, any other vanilla family and an unknown id refuse the load,
    /// naming the type. `entity_part` itself checks no type.
    pub fn entity_part_with_state<P: crate::EntityPartState>(
        &mut self,
        entity: &str,
        part: P,
    ) -> Result<(), crate::EntityPartStateError> {
        self.declare_entity_part_state(
            entity,
            type_name::<P>(),
            crate::PartStateDeclaration::of(P::DATA),
        )?;
        self.entity_part(entity, part);
        Ok(())
    }
    /// Answer entity capability `key` for entity type `kind` (`mymod:robot`,
    /// `minecraft:player`) with `provider`, which gives a handler for one entity, or none:
    /// `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |_, robot, ()| Some(Box::new(Cargo(robot))))`.
    /// Asked as `block_entity_capability`'s are.
    pub fn entity_capability<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_entity_capability(
            key.name,
            kind,
            false,
            EntityProviderOf::of(Arc::new(provider)),
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
        self.provide_entity_capability(
            key.name,
            kind,
            true,
            EntityProviderOf::of(Arc::new(provider)),
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
    /// it (as the mod's registration refusal too): see `PartStateDeclaration::check`, a key
    /// outside this mod's namespaces, or one it declared with another Rust type name. Mods reach
    /// it through `entity_part_with_state`.
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
        provider: EntityProviderOf,
    );

    /// Tell `entity_tick_pre` and `entity_tick_post` for each entity of type `entity`
    /// (`minecraft:zombie`, `mymod:golem`) alone, right before and right after its own tick
    /// (NeoForge's `EntityTickEvent` timing), instead of once with its family. Every mod that
    /// hooks them hears those entities alone. Name only the types that need their own moment:
    /// each entity of a named type costs a call of its own every tick. A type that is not a
    /// namespaced lowercase id refuses the mod's registration.
    fn entity_tick_each(&mut self, entity: &str);

    /// [`Self::entity_tick_each`] for every entity type the tick hooks observe,
    /// as a port of a NeoForge mod that listens to `EntityTickEvent` needs.
    fn entity_tick_each_entity(&mut self);
}

/// The entity types whose ticks are told one entity at a time, from every mod's
/// [`EntitiesRegistrar::entity_tick_each`], resolved when the mods are installed.
// hidden: the game reads it once at load to mark its entity types.
// no wire: resolved on the host from the registrar's `entity_tick_each` calls, which cross on
// their own; the set itself is never sent to a mod.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[doc(hidden)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EntityTickEach {
    all: bool,
    /// Sorted, without repeats.
    kinds: Vec<String>,
}

impl EntityTickEach {
    /// The set the declarations name, in any order: `None` names every type.
    pub fn of(declarations: &[Option<String>]) -> Self {
        let all = declarations.iter().any(Option::is_none);
        let mut kinds: Vec<String> = if all {
            Vec::new()
        } else {
            declarations.iter().flatten().cloned().collect()
        };
        kinds.sort_unstable();
        kinds.dedup();
        Self { all, kinds }
    }
    /// Whether a mod named any type.
    pub fn any(&self) -> bool {
        self.all || !self.kinds.is_empty()
    }
    /// Whether a mod named every type.
    pub fn all(&self) -> bool {
        self.all
    }
    /// Whether entities of type `kind` are told one at a time.
    pub fn names(&self, kind: &str) -> bool {
        self.all
            || self
                .kinds
                .binary_search_by(|k| k.as_str().cmp(kind))
                .is_ok()
    }
}

/// The installed mods' [`EntityTickEach`], or `None` before the mods are installed, when no type
/// is named.
// hidden: the game reads it once at load.
#[doc(hidden)]
pub fn entity_tick_each() -> Option<&'static EntityTickEach> {
    installed().map(|parts| &parts.entity_tick_each)
}

/// CapabilityProviders operations for entities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait EntitiesCapabilityProviders {
    /// The next enabled provider of entity capability `name` for entity type `kind`, advancing `index`.
    fn entity_provider(
        &self,
        name: &str,
        kind: &str,
        index: &mut u32,
    ) -> Option<(EntityProviderOf, &'static str, bool)>;
}

impl EntitiesCapabilityProviders for InstalledProviders {
    fn entity_provider(
        &self,
        name: &str,
        kind: &str,
        index: &mut u32,
    ) -> Option<(EntityProviderOf, &'static str, bool)> {
        next(&installed()?.keys.entity_capabilities, name, kind, index)
    }
}
