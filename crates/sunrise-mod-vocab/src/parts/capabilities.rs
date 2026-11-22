use super::*;

pub(crate) type Capabilities<P> = HashMap<&'static str, HashMap<Box<str>, Vec<CapabilityPart<P>>>>;

/// A capability provider a mod registered: a [`BlockProviderOf`], [`ItemProviderOf`] or
/// [`EntityProviderOf`].
#[derive(Clone)]
pub(crate) struct CapabilityPart<P> {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// The handler type it gives, for refusing a capability name answered with two.
    pub(crate) type_name: Arc<str>,
    /// Asked ahead of the game's own provider.
    pub(crate) before_game: bool,
    /// Its place among every mod's registrations, which orders a block's providers and those of
    /// the block entity type standing there as one list.
    pub(crate) order: usize,
    /// Panic containment state, shared by the installed snapshots of this registration.
    pub(crate) poisoned: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) provider: P,
}

/// A capability provider whose handler type is a mod's own (a key a library's `[api]` crate
/// publishes), erased: only mods linked with that crate know its handler type.
// hidden: the registrar's erased provider, not part of the mod surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers the handler name and a retained provider resource id, reporting unsupported handlers rather than serializing the native Any object.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnProvider {
    /// The `Arc` of the provider trait object, which a lookup of that handler type downcasts.
    pub(crate) provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: Arc<str>,
}

/// The registrar's table, as the game reads it.
pub(crate) struct InstalledProviders;

pub(crate) fn next<P: Clone>(
    table: &Capabilities<P>,
    name: &str,
    id: &str,
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    next_provider(table.get(name)?.get(id)?, index)
}

/// Merge the block and block-entity lists without copying provider owners. Each list is
/// already ordered with `before_game` first, then by registration order.
pub(crate) fn next_merged<'a>(
    [first, second]: &mut [&'a [CapabilityPart<BlockProviderOf>]; 2],
) -> Option<(&'a CapabilityPart<BlockProviderOf>, &'static str, bool)> {
    let rank = |part: &CapabilityPart<BlockProviderOf>| (!part.before_game, part.order);
    let take_second = match (first.first(), second.first()) {
        (Some(a), Some(b)) => rank(b) < rank(a),
        (None, _) => true,
        _ => false,
    };
    let source = if take_second { second } else { first };
    let (part, rest) = source.split_first()?;
    *source = rest;
    Some((part, part.mod_id, part.before_game))
}

/// A provider by its handler type: one case for each handler type `mod_api`'s keys name
/// (generated from them), and [`OwnProvider`] for a mod's own.
#[allow(missing_docs)]
pub(crate) mod providers_of {
    use super::*;
    // The keys' handler types and contexts, as the generated cases name them.
    #[allow(unused_imports)]
    use crate::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/capability_providers_generated.rs"
    ));
}

/// Who answers a capability: a mod's provider, the game's own, or nobody.
pub(crate) enum Answer<T> {
    Mod(T, &'static str),
    Game,
    Nobody,
}

/// The handler `answer` names: a mod's, or `game`, the game's own when it answered.
pub(crate) fn answered<T: ?Sized>(
    answer: Answer<Box<T>>,
    game: Option<Box<T>>,
) -> Option<Capability<T>> {
    match answer {
        Answer::Mod(handler, owner) => Some(Capability {
            handler,
            owner: Some(owner),
        }),
        Answer::Game => game.map(|handler| Capability {
            handler,
            owner: None,
        }),
        Answer::Nobody => None,
    }
}

/// Asks the providers `next` gives (advancing a registration cursor in dispatch order: those
/// registered `before_game`, then the others), and the game's own (`game`: whether it answers) between the
/// two, in the fork's chosen order (NeoForge's javadoc doesn't state one between mods'
/// providers and the game's); the first to answer wins. `call` also resolves the
/// provider's handler type; a type the asker isn't linked with is passed over.
pub(crate) fn ask_providers<C: ?Sized, O, T>(
    context: &mut C,
    mut next: impl FnMut() -> Option<(O, &'static str, bool)>,
    mut call: impl FnMut(O, &mut C) -> Option<T>,
    game: impl FnOnce(&mut C) -> bool,
) -> Answer<T> {
    let mut game = Some(game);
    while let Some((provider, owner, before_game)) = next() {
        if !before_game
            && let Some(game) = game.take()
            && game(context)
        {
            return Answer::Game;
        }
        if let Some(handler) = call(provider, context) {
            return Answer::Mod(handler, owner);
        }
    }
    match game.is_some_and(|game| game(context)) {
        true => Answer::Game,
        false => Answer::Nobody,
    }
}

/// CapabilityProviders operations for capabilities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait CapabilitiesCapabilityProviders {
    /// Whether any mod registered capability `name` for the things `by` names. This presence
    /// check includes disabled mods; enumeration skips their providers.
    fn provided(&self, by: ProvidedBy, name: &str) -> bool;
    /// Known block capability names in declaration/discovery order.
    fn block_capability_names(&self) -> Vec<String>;
    /// Whether this capability was explicitly marked safe to forward.
    fn block_capability_proxyable(&self, name: &str) -> bool;
    /// Known block capability names explicitly marked safe to forward, in discovery order.
    fn block_capability_proxyable_names(&self) -> Vec<String>;
    /// How many times a mod was switched on or off, as the game counts it: a kept capability
    /// compares it to tell that a provider may have come or gone (`BlockCapabilityCache`).
    fn mods_switched(&self) -> u64;
}

impl CapabilitiesCapabilityProviders for InstalledProviders {
    fn provided(&self, by: ProvidedBy, name: &str) -> bool {
        installed().is_some_and(|installed| match by {
            ProvidedBy::BlockEntity => installed.keys.capabilities.contains_key(name),
            ProvidedBy::Block => installed.keys.block_capabilities.contains_key(name),
            ProvidedBy::Item => installed.keys.item_capabilities.contains_key(name),
            ProvidedBy::Entity => installed.keys.entity_capabilities.contains_key(name),
        })
    }
    fn block_capability_names(&self) -> Vec<String> {
        match installed() {
            Some(installed) => installed
                .keys
                .block_capability_order
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            None => providers_of::builtin_block_capabilities()
                .into_iter()
                .map(|(name, _)| name.to_owned())
                .collect(),
        }
    }
    fn block_capability_proxyable(&self, name: &str) -> bool {
        installed()
            .and_then(|installed| installed.keys.block_capability_metadata.get(name))
            .is_some_and(|(_, proxyable)| *proxyable == Some(true))
    }
    fn block_capability_proxyable_names(&self) -> Vec<String> {
        let Some(installed) = installed() else {
            return Vec::new();
        };
        installed
            .keys
            .block_capability_order
            .iter()
            .filter(|name| self.block_capability_proxyable(name))
            .map(|name| (*name).to_owned())
            .collect()
    }
    fn mods_switched(&self) -> u64 {
        crate::runtime::mods_switched()
    }
}
