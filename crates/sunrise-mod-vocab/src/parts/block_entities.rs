use super::*;
use crate::{
    BlockCapabilityOffer, BlockCapabilitySite, BlockEntityRef, BlockState, LevelHost, backend_of,
};
/// The read-only discovery callback behind `Registrar::block_capability`. Call
/// [`BlockCapabilityOffer::offer`] with a handler constructor to answer, or leave it untouched
/// to decline. Discovery inputs are the same during presence checks and handler acquisition.
pub trait BlockCapabilityProvider<T: ?Sized>: Send + Sync {
    /// Decide whether to offer a handler for this site's state, optional entity and side.
    fn provide(
        &self,
        level: &dyn LevelHost,
        at: BlockSide,
        state: BlockState,
        entity: Option<BlockEntityRef>,
        offer: &mut BlockCapabilityOffer<'_, T>,
    );
}
impl<T: ?Sized, F> BlockCapabilityProvider<T> for F
where
    F: Fn(
            &dyn LevelHost,
            BlockSide,
            BlockState,
            Option<BlockEntityRef>,
            &mut BlockCapabilityOffer<'_, T>,
        ) + Send
        + Sync,
{
    fn provide(
        &self,
        level: &dyn LevelHost,
        at: BlockSide,
        state: BlockState,
        entity: Option<BlockEntityRef>,
        offer: &mut BlockCapabilityOffer<'_, T>,
    ) {
        self(level, at, state, entity, offer);
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// A block capability provider, as the registrar keeps it.
pub type BlockProvider<T> = dyn BlockCapabilityProvider<T>;

/// One retained structure callback, with its owner and session-wide panic state.
// no wire: native callback and process-local enablement.
// exhaustive: internal engine registration record in parts::block_entities.
#[derive(Clone)]
pub struct RegisteredBlockEntityTransform {
    /// The mod that registered this transform and its quarantine state.
    pub owner: crate::callback_owner::CallbackOwner,
    /// The placement-local block entity transform.
    pub step: Arc<dyn crate::BlockEntityTransform>,
}

/// The state-only selection supplied by the owning type builder.
// no wire: native registration callback.
// hidden: mods give the selector through the type builder; the engine stores this form.
#[doc(hidden)]
pub type BlockEntityTickerSelector =
    dyn Fn(&dyn crate::BlockMetadata, BlockState, crate::LogicalSide) -> bool + Send + Sync;

/// One retained native selector, dispatched through its registering mod's callback owner.
// no wire: process-local registration.
// hidden: the engine's installed selectors.
// exhaustive: only installation builds it, and the engine reads every field.
#[doc(hidden)]
pub struct RegisteredBlockEntityTickerSelector {
    pub kind: crate::BlockEntityTypeKey,
    pub owner: crate::callback_owner::CallbackOwner,
    pub select: Arc<BlockEntityTickerSelector>,
}

// hidden: the engine reads the installed selectors.
#[doc(hidden)]
pub fn block_entity_ticker_selectors() -> &'static [RegisteredBlockEntityTickerSelector] {
    installed().map_or(&[], |parts| &parts.block_entity_ticker_selectors)
}

/// Snapshot the callbacks once at registry preparation for worldgen workers.
pub fn block_entity_transforms() -> std::collections::HashMap<String, RegisteredBlockEntityTransform>
{
    installed().map_or_else(Default::default, |table| {
        table.block_entity_transforms.clone()
    })
}

/// Owned declarations of the block entity types mods registered.
/// The game converts these snapshots to its content model when registration freezes.
pub fn block_entity_types() -> &'static [(&'static str, BlockEntityTypeDecl)] {
    installed().map_or(&[], |installed| &installed.block_entity_types)
}

pub(crate) fn resolved_block_entity_types()
-> &'static OnceLock<sunrise_mod_runtime::registry_keys::Names> {
    static EMPTY: OnceLock<sunrise_mod_runtime::registry_keys::Names> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.resolved_block_entity_types)
}

/// Whether this type has a Part subscribing to `hook`.
pub fn block_entity_type_listens(hook: HookId, id: &str) -> bool {
    has_part_hook(hook)
        && parts(hook, "block entity routing")
            .for_hook(hook)
            .is_some_and(|table| !table.block_entity_kind(|| Some(id)).is_empty())
}

impl HookParts {
    pub(crate) fn block_entity<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entities, id)
    }
    pub(crate) fn block_entity_kind<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entity_kinds, id)
    }
}

/// The custom providers for this capability at a resident position. None if no mod registered
/// for its block or its block entity type. No provider callback is run until [`BlockProviders::ask`].
pub fn block_providers<T: ?Sized + 'static>(
    key: &CapabilityKey<T>,
    host: &dyn Host,
    at: BlockSide,
) -> Option<BlockProviders<T>> {
    let tables = &installed()?.keys;
    if !tables.capabilities.contains_key(key.name)
        && !tables.block_capabilities.contains_key(key.name)
    {
        return None;
    }
    let mut providers = None;
    backend_of(host)?
        .level_with_capability_site(at, &mut |_, site| {
            providers = BlockProviders::at(key, site);
        })
        .ok()?;
    providers
}

// no wire: provider lookup borrows static dispatch tables and process-local type identity.
/// A position's providers of one capability ([`block_providers`]).
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockProviders<T: ?Sized + 'static> {
    entity: &'static [CapabilityPart<NativeCapabilityProvider>],
    block: &'static [CapabilityPart<NativeCapabilityProvider>],
    site: BlockCapabilitySite,
    handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> BlockProviders<T> {
    pub(crate) fn at(key: &CapabilityKey<T>, site: BlockCapabilitySite) -> Option<Self> {
        let tables = &installed()?.keys;
        let entity = providers_at(
            &tables.capabilities,
            key.name,
            site.kind
                .and_then(crate::content::registry_keys::block_entity_type_name),
        );
        let block = providers_at(
            &tables.block_capabilities,
            key.name,
            crate::content::registry_keys::block_id(site.state.block()),
        );
        if entity.is_empty() && block.is_empty() {
            return None;
        }
        Some(Self {
            entity,
            block,
            site,
            handler: PhantomData,
        })
    }

    /// A mod's handler in [`LevelRef::capability`]'s order. If the game has an answer,
    /// only a mod registered `before_game` can provide the returned handler.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let mut result = None;
        backend_of(host)?
            .level_with_capability_site(self.site.at, &mut |view, site| {
                if site == self.site {
                    result = answered(self.acquire(view, game, false), None);
                }
            })
            .ok()?;
        result
    }

    pub(crate) fn acquire(
        &self,
        view: &dyn LevelHost,
        game: bool,
        after_game: bool,
    ) -> Answer<Box<T>> {
        self.resolve(game, after_game, |provider| {
            let mut handler = None;
            provider.provide(
                view,
                self.site.at,
                self.site.state,
                self.site.entity,
                &mut BlockCapabilityOffer::construct(&mut handler),
            );
            handler
        })
    }

    pub(crate) fn present(&self, view: &dyn LevelHost) -> bool {
        !matches!(
            self.resolve(self.site.builtin_items, false, |provider| {
                let mut found = false;
                provider.provide(
                    view,
                    self.site.at,
                    self.site.state,
                    self.site.entity,
                    &mut BlockCapabilityOffer::presence(&mut found),
                );
                found.then_some(())
            }),
            Answer::Nobody
        )
    }

    fn resolve<R>(
        &self,
        game: bool,
        after_game: bool,
        mut call: impl FnMut(&dyn BlockCapabilityProvider<T>) -> Option<R>,
    ) -> Answer<R> {
        let [entity, block] = [self.entity, self.block].map(|slice| {
            if after_game {
                &slice[slice.partition_point(|part| part.before_game)..]
            } else {
                slice
            }
        });
        ask_providers(
            &mut (),
            MergedProviders::new(entity, block),
            ("block capability provider", "block_capability"),
            |provider, _| call(provider.get::<Arc<BlockProvider<T>>>()?.as_ref()),
            |_| game,
        )
    }
}

impl dyn Registrar + '_ {
    /// Declare a block entity type by key. A bare name uses this mod's namespace; an explicit
    /// valid `namespace:path` is kept. Existing type keys, including vanilla's, cannot be replaced.
    pub fn block_entity_type(&mut self, id: &str) -> crate::BlockEntityTypeBuilder<'_> {
        crate::BlockEntityTypeBuilder {
            id: id.to_owned(),
            registrar: self,
            transform: None,
            ticker: None,
            blocks: Vec::new(),
        }
    }
    /// Attach a Part to a registered native type, including one another mod declares later.
    /// Built-in types are usable for capabilities, but not for native payload Parts.
    ///
    /// A type declared by another package takes this package's Part only when its
    /// `[dependencies]` row for the declaring package applies on this installation side and
    /// survives instance overrides. Otherwise load is refused, naming the type, this package
    /// and "declare <package> as a dependency". A type no package declares is refused too.
    /// The Part is skipped only when the type's namespace is the id of an absent package
    /// with a surviving, applicable optional dependency row. An absent package's other
    /// namespaces are unknown; a present package must declare the type.
    /// The Part's saved state is the issued [`AttachmentKey`](crate::AttachmentKey) it declares
    /// (see [`Self::block_entity_part_with_state`]), registered during startup with
    /// [`Self::register_attachment_type`] (or looked up from another package via
    /// [`Self::attachment_key`]). It reaches the owner's state through the owner's declared
    /// attachments. Declaring the type, its
    /// transform and its initial data stay with the owner. If this package is later removed, the
    /// block entities keep what it saved, unread, and a package that returns reads it again.
    pub fn block_entity_part<P: BlockEntityPart>(
        &mut self,
        kind: &crate::BlockEntityTypeKey,
        part: P,
    ) -> Result<(), crate::BlockEntityPartError> {
        self.attach_block_entity_part(kind, type_name::<P>(), Arc::new(part))
    }
    /// [`Self::block_entity_part`] for a Part with typed state (`BlockEntityPartState`):
    /// each block entity of type `kind` keeps one `P::Data`, under `part.state_key()`.
    /// Refused, and reported as the mod's registration refusal, when the key syncs, copies on
    /// death, or was not issued in this registration pass.
    pub fn block_entity_part_with_state<P: crate::BlockEntityPartState>(
        &mut self,
        kind: &crate::BlockEntityTypeKey,
        part: P,
    ) -> Result<(), crate::BlockEntityStateError> {
        self.declare_block_entity_part_state(
            type_name::<P>(),
            crate::PartStateDeclaration::of(part.state_key()),
        )?;
        self.block_entity_part(kind, part)
            .map_err(crate::BlockEntityStateError::Part)
    }
    /// Answer capability `key` for a resident block entity of type `kind` (NeoForge
    /// `RegisterCapabilitiesEvent.registerBlockEntity`). The provider receives the restricted
    /// level, position and side, state and matching entity. Use `offer.offer(|| Box::new(handler))`
    /// to answer without building the handler during a presence check. A provider can also decline.
    /// See [`LevelRef::capability`] for priority across block and block entity registrations.
    pub fn block_entity_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(
            &dyn LevelHost,
            BlockSide,
            BlockState,
            BlockEntityRef,
            &mut BlockCapabilityOffer<'_, T>,
        ) + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_entity_capability(key.name, kind, false, entity_provider(provider));
    }
    /// [`Self::block_entity_capability`], asked ahead of the game's own provider.
    pub fn block_entity_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(
            &dyn LevelHost,
            BlockSide,
            BlockState,
            BlockEntityRef,
            &mut BlockCapabilityOffer<'_, T>,
        ) + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_entity_capability(key.name, kind, true, entity_provider(provider));
    }
}

fn entity_provider<T: ?Sized + 'static>(
    provider: impl Fn(
        &dyn LevelHost,
        BlockSide,
        BlockState,
        BlockEntityRef,
        &mut BlockCapabilityOffer<'_, T>,
    ) + Send
    + Sync
    + 'static,
) -> NativeCapabilityProvider {
    let provider: Arc<BlockProvider<T>> = Arc::new(
        move |level: &dyn LevelHost, at, state, entity, offer: &mut BlockCapabilityOffer<'_, T>| {
            if let Some(entity) = entity {
                provider(level, at, state, entity, offer);
            }
        },
    );
    NativeCapabilityProvider::of::<T, _>(provider)
}

/// Registrar operations for block entities.
pub trait BlockEntitiesRegistrar {
    /// Set the transform of a type declared by this mod.
    fn set_block_entity_transform(
        &mut self,
        key: crate::BlockEntityTypeKey,
        step: Arc<dyn crate::BlockEntityTransform>,
    );
    // hidden: the type builder calls it; mods select tickers on the builder.
    #[doc(hidden)]
    fn set_block_entity_ticker_selector(
        &mut self,
        kind: crate::BlockEntityTypeKey,
        select: Arc<BlockEntityTickerSelector>,
    );

    /// Request a type by registry ID. A missing type in the complete registry refuses load.
    fn block_entity_type_key(&mut self, id: &str) -> crate::BlockEntityTypeKey;

    /// Declare a new block entity type held by `blocks`. A bare type or block name uses this mod's
    /// namespace. Explicit keys keep their namespace. Vanilla blocks cannot be reassigned here;
    /// mods usually reach this through `block_entity_type`.
    fn declare_block_entity_type(
        &mut self,
        path: &str,
        blocks: Vec<String>,
    ) -> crate::BlockEntityTypeKey;

    /// Attach `part`, of kind `kind`, to the native block entity type `key`.
    /// The type is resolved by registry ID; a foreign type requires a dependency on its owner.
    /// An absent optional dependency skips this Part; other missing types refuse loading.
    fn attach_block_entity_part(
        &mut self,
        key: &crate::BlockEntityTypeKey,
        kind: &'static str,
        part: Arc<dyn BlockEntityPart>,
    ) -> Result<(), crate::BlockEntityPartError>;

    /// Record the state declaration of Part `part`, or refuse it (as the mod's registration
    /// refusal too): see `PartStateDeclaration::check` and `PartStateRefusal::StaleKey`. Mods
    /// reach it through `block_entity_part_with_state`.
    fn declare_block_entity_part_state(
        &mut self,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::BlockEntityStateError>;

    /// Answer block entity capability `name` for type `kind` with `provider`, asked ahead of the
    /// game's own when `before_game`; mods reach it through `block_entity_capability`.
    fn provide_block_entity_capability(
        &mut self,
        name: &'static str,
        kind: &crate::BlockEntityTypeKey,
        before_game: bool,
        provider: NativeCapabilityProvider,
    );
}
