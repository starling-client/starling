use super::*;
/// Which block keys each scoped hook has a prepared subscriber for, and which keys' parts answer
/// `explosion_resistance` impurely: plain bits, read on any thread. It holds no part, no mod
/// handle and no callable, so a tick phase may hold an `Arc` of it for the phase's length while
/// the tick thread replaces the table between phases.
// no wire: engine-private routing snapshot; never a mod boundary value.
#[derive(Clone, Debug, Default)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ListenIndex {
    /// A bit per block key, for each hook that has a subscriber.
    hooks: Vec<Option<Box<[u64]>>>,
    /// A bit per block key that has a part whose `explosion_resistance` answer is not pure.
    explosion_impure: Box<[u64]>,
}

fn bit(bits: &[u64], key: crate::BlockKey) -> bool {
    let key = key.0 as usize;
    bits.get(key / 64)
        .is_some_and(|word| word >> (key % 64) & 1 != 0)
}

fn set(bits: &mut Vec<u64>, key: crate::BlockKey) {
    let key = key.0 as usize;
    if bits.len() <= key / 64 {
        bits.resize(key / 64 + 1, 0);
    }
    bits[key / 64] |= 1 << (key % 64);
}

impl ListenIndex {
    /// The index of the resolved hook tables (`block_keys` filled).
    pub(crate) fn of(hooks: &[Option<Box<HookParts>>]) -> Self {
        let listening = hooks
            .iter()
            .map(|table| {
                let mut bits = Vec::new();
                for key in table.iter().flat_map(|table| table.block_keys.keys()) {
                    set(&mut bits, *key);
                }
                (!bits.is_empty()).then(|| bits.into_boxed_slice())
            })
            .collect();
        let mut impure = Vec::new();
        let resistance = hooks
            .get(HookId::ExplosionResistance as usize)
            .and_then(Option::as_ref);
        for (key, entry) in resistance.iter().flat_map(|table| &table.block_keys) {
            if !entry
                .parts()
                .iter()
                .all(|a| a.part.explosion_resistance_is_pure())
            {
                set(&mut impure, *key);
            }
        }
        Self {
            hooks: listening,
            explosion_impure: impure.into_boxed_slice(),
        }
    }

    /// Whether `hook` has a prepared subscriber for the block key.
    pub fn listens(&self, hook: HookId, key: crate::BlockKey) -> bool {
        self.hooks
            .get(hook as usize)
            .and_then(Option::as_deref)
            .is_some_and(|bits| bit(bits, key))
    }

    /// Whether every part on the block key that answers `explosion_resistance` declares the
    /// answer pure (`BlockPart::explosion_resistance_is_pure`).
    pub fn explosion_resistance_pure(&self, key: crate::BlockKey) -> bool {
        !bit(&self.explosion_impure, key)
    }
}

/// The listen index of this thread's game parts, as they stand now: empty before the game
/// resolved its parts. A tick phase takes it once, when the phase starts.
pub fn game_listen_index() -> Arc<ListenIndex> {
    GAME_PARTS.with_borrow(|lookup| {
        lookup
            .parts
            .as_ref()
            .map_or_else(Default::default, |parts| parts.listens.clone())
    })
}

/// What a tick phase's workers answer "who listens" from: the listen index and each block state's
/// block key, plain data held by `Arc` so the phase keeps the tables it started with. A worker
/// thread has no game parts and no state index of its own; inside [`enter`](Self::enter) the
/// engine's routing questions (`scoped_state_listens`) are answered from this instead of
/// answering "no listener".
// no wire: engine-private routing snapshot; never a mod boundary value.
#[derive(Clone, Debug)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ListenScope {
    listens: Arc<ListenIndex>,
    blocks: Arc<[crate::BlockKey]>,
}

thread_local! {
    static LISTEN_SCOPE: RefCell<Option<ListenScope>> = const { RefCell::new(None) };
}

impl ListenScope {
    /// The scope of `listens`, for the block states `blocks` names by state number.
    pub fn new(listens: Arc<ListenIndex>, blocks: Arc<[crate::BlockKey]>) -> Self {
        Self { listens, blocks }
    }

    /// Whether `hook` has a subscriber for the block state (by number); false for a state with
    /// no block here.
    pub fn listens(&self, hook: HookId, state: u32) -> bool {
        self.blocks
            .get(state as usize)
            .is_some_and(|key| self.listens.listens(hook, *key))
    }

    /// Whether every part on the state's block that answers `explosion_resistance` declares the
    /// answer pure; false for a state with no block here.
    pub fn explosion_resistance_pure(&self, state: u32) -> bool {
        self.blocks
            .get(state as usize)
            .is_some_and(|key| self.listens.explosion_resistance_pure(*key))
    }

    /// Run `f` on this thread with the scope in force, restoring the previous one after, even
    /// when `f` panics.
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Restore(Option<ListenScope>);
        impl Drop for Restore {
            fn drop(&mut self) {
                LISTEN_SCOPE.with_borrow_mut(|scope| *scope = self.0.take());
            }
        }
        let _restore = Restore(LISTEN_SCOPE.with_borrow_mut(|scope| scope.replace(self.clone())));
        f()
    }
}

/// The scope's answer to [`ListenScope::listens`], or `None` outside a scope.
pub fn scoped_state_listens(hook: HookId, state: u32) -> Option<bool> {
    LISTEN_SCOPE.with_borrow(|scope| scope.as_ref().map(|scope| scope.listens(hook, state)))
}

/// The scope's answer to [`ListenScope::explosion_resistance_pure`], or `None` outside a scope.
pub fn scoped_explosion_resistance_pure(state: u32) -> Option<bool> {
    LISTEN_SCOPE.with_borrow(|scope| {
        scope
            .as_ref()
            .map(|scope| scope.explosion_resistance_pure(state))
    })
}

/// Whether this hook has a prepared subscriber for an API block key.
/// The caller owns validation of the routing key.
pub fn block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    parts(hook, "block routing").listens.listens(hook, key)
}

/// The ids of the mods whose parts on this API block key answer `hook`, in dispatch order: what a
/// diagnostic about the block's parts names.
pub fn block_key_owners(hook: HookId, key: crate::BlockKey) -> Vec<&'static str> {
    parts(hook, "block routing")
        .for_hook(hook)
        .map_or_else(Vec::new, |table| {
            table
                .block_key(Some(key))
                .iter()
                .map(|attached| attached.mod_id)
                .collect()
        })
}

/// Whether every part on this API block key that answers `explosion_resistance` declares the
/// answer pure (`BlockPart::explosion_resistance_is_pure`), so a batched read may ask once a state.
pub fn explosion_resistance_key_pure(key: crate::BlockKey) -> bool {
    parts(HookId::ExplosionResistance, "block routing")
        .listens
        .explosion_resistance_pure(key)
}

/// Whether the feature-pinned published table subscribes to this worldgen block key.
pub fn worldgen_block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    worldgen_parts()
        .for_hook(hook)
        .is_some_and(|table| !table.block_key(Some(key)).is_empty())
}

/// A resolved block's subscribers for one hook, in dispatch order.
#[derive(Clone)]
pub(crate) struct BlockHookParts {
    parts: Vec<Attached<dyn BlockPart>>,
    /// The content owner's first subscribed Part, regardless of whether it is currently enabled.
    owner_slot: Option<usize>,
}

impl BlockHookParts {
    pub(crate) fn new(id: &str, parts: &[Attached<dyn BlockPart>]) -> Self {
        let owner_slot = crate::runtime::content_owner_index(id)
            .and_then(|owner| parts.iter().position(|part| part.mod_index == owner));
        Self {
            parts: parts.to_vec(),
            owner_slot,
        }
    }

    pub(crate) fn parts(&self) -> &[Attached<dyn BlockPart>] {
        &self.parts
    }

    pub(crate) fn disabled_owner_slot(&self) -> Option<usize> {
        self.owner_slot
            .filter(|&slot| !crate::mod_index_on(self.parts[slot].mod_index))
    }
}

impl HookParts {
    /// Prepared subscribers for an API block key; missing keys have no subscribers.
    pub fn block_key(&self, key: Option<crate::BlockKey>) -> &[Attached<dyn BlockPart>] {
        key.and_then(|key| self.block_keys.get(&key))
            .map_or(&[], BlockHookParts::parts)
    }
    /// Parts for a checked API state. Key lookup does not depend on world state numbers.
    pub fn block_value(&self, state: crate::BlockState) -> &[Attached<dyn BlockPart>] {
        self.block_entry(state).map_or(&[], BlockHookParts::parts)
    }

    pub(crate) fn block_entry(&self, state: crate::BlockState) -> Option<&BlockHookParts> {
        let blocks = crate::__private::block_definitions()?;
        let key = state.block();
        let definition = blocks.definitions.get(key.0 as usize)?;
        definition.property(state.ordinal(), "").ok()?;
        self.block_keys.get(&key)
    }
}

#[cfg(test)]
#[test]
fn cached_slot_tracks_the_first_owner_in_dispatch_order_and_live_toggles() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Quiet {}
    struct Probe;
    #[mod_api::sunrise_part]
    impl BlockPart for Probe {}

    crate::testing::with_dispatch(
        vec![
            ModEntry::new("other", Box::new(Quiet)),
            ModEntry::new("owner", Box::new(Quiet)),
        ]
        .into(),
        || {
            let attach = |mod_index, mod_id, order, type_name| Attached::<dyn BlockPart> {
                mod_id,
                mod_index,
                order,
                type_name,
                part: Arc::new(Probe),
            };
            let other = attach(0, "other", 0, "Other");
            let owner = attach(1, "owner", 0, "Owner");
            let second = attach(1, "owner", 1, "Second");
            let first = BlockHookParts::new("other:first", &[other.clone(), owner.clone()]);
            let later = BlockHookParts::new("owner:later", &[other.clone(), owner, second]);
            let absent = BlockHookParts::new("owner:absent", &[other]);

            assert_eq!(later.disabled_owner_slot(), None);
            assert!(crate::set_mod_enabled("owner", false));
            assert_eq!(first.disabled_owner_slot(), None);
            assert_eq!(later.disabled_owner_slot(), Some(1));
            assert_eq!(absent.disabled_owner_slot(), None);
            assert!(crate::set_mod_enabled("other", false));
            assert_eq!(first.disabled_owner_slot(), Some(0));
            assert_eq!(absent.disabled_owner_slot(), None);
            assert!(crate::set_mod_enabled("owner", true));
            assert_eq!(later.disabled_owner_slot(), None);
        },
    );
}

impl dyn Registrar + '_ {
    /// Attach `part` to block `block` (`mymod:shaft`, `minecraft:oak_log`).
    pub fn block_part<P: BlockPart>(&mut self, block: crate::BlockKey, part: P) {
        self.attach_block_part(block, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to every block in tag `tag` (`minecraft:logs`, without `#`), as each game's
    /// tags have it when its world opens and after `/reload`.
    pub fn block_part_tag<P: BlockPart>(&mut self, tag: &str, part: P) {
        self.attach_block_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
    /// Answer capability `key` for block `block` with `provider`, whether or not a block entity
    /// stands there (NeoForge's `RegisterCapabilitiesEvent.registerBlock`): a trash can that voids
    /// what it is given, a machine's port block passing a pipe on to its controller. `provider`
    /// is given the position and side asked about, the block state there and the block entity,
    /// if any, and a read-only level. Call `offer.offer(|| Box::new(Voids))` to answer, or
    /// leave the offer untouched to decline. Asked
    /// in one list with the providers of the block entity type there, as
    /// [`LevelRef::capability`](crate::LevelRef::capability) orders them.
    pub fn block_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        block: &crate::BlockKey,
        provider: impl Fn(
            &dyn crate::LevelHost,
            BlockSide,
            crate::BlockState,
            Option<crate::BlockEntityRef>,
            &mut crate::BlockCapabilityOffer<'_, T>,
        ) + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_capability(
            key.name,
            block,
            false,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
    /// `block_capability`, asked ahead of the game's own provider.
    pub fn block_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        block: &crate::BlockKey,
        provider: impl Fn(
            &dyn crate::LevelHost,
            BlockSide,
            crate::BlockState,
            Option<crate::BlockEntityRef>,
            &mut crate::BlockCapabilityOffer<'_, T>,
        ) + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_capability(
            key.name,
            block,
            true,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
}

/// Registrar operations for blocks.
pub trait BlocksRegistrar {
    /// Request a block by canonical registry id. All mods' declarations are checked at load.
    fn block_key(&mut self, id: &str) -> crate::BlockKey;

    /// Attach `part`, of kind `kind` (its type's name), to block `block`.
    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    );

    /// Attach `part`, of kind `kind`, to every block in tag `tag`.
    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>);

    /// Answer block capability `name` for block `block` with `provider`, asked ahead of the
    /// game's own when `before_game`; mods reach it through `block_capability`.
    fn provide_block_capability(
        &mut self,
        name: &'static str,
        block: &crate::BlockKey,
        before_game: bool,
        provider: BlockProviderOf,
    );
}

/// `table`'s providers of capability `name` for `id`: none for no id.
pub(crate) fn providers_at<'a, P>(
    table: &'a Capabilities<P>,
    name: &str,
    id: Option<&str>,
) -> &'a [CapabilityPart<P>] {
    id.and_then(|id| table.get(name)?.get(id))
        .map_or(&[], Vec::as_slice)
}
