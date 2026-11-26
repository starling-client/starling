use super::*;
use crate::{ClientHost, ItemStack};

// no wire: in-process behavior called with a borrowed stack and client host.
/// Client-only main-hand input. Capturing it does not make the player use or block with the
/// item. For ordinary item use on both sides, use [`ItemPart::use_item`] and
/// [`ItemPart::release_using`]. Register one owner for an item with
/// [`ItemsRegistrar::register_client_item_use`].
///
/// While the same item kind stays in the active hotbar slot (0–8), each tick and release
/// rebinds the stack to the live slot contents (matching `LivingEntity.updatingUsingItem` and
/// `LivingEntity.releaseUsingItem`). If the slot is emptied or replaced by a different item
/// kind, [`Self::on_stop_using`] receives the last valid stack of the captured item kind.
/// Track the capture in these callbacks, not through the living-item use reads.
/// A disabled owner receives no callbacks.
pub trait ClientItemUseBehavior: Send + Sync + 'static {
    /// Consume a main-hand use input before the client handles the crosshair target.
    fn use_item(&self, host: &mut dyn ClientHost, stack: &ItemStack, slot: u32);

    /// Run on input release (including keybinds that open a screen and release key mappings,
    /// matching `Minecraft.handleKeybinds`) while the same item kind is still selected in the
    /// active slot and the player can still use it. The game clears the capture before calling
    /// this. `ticks_held` counts elapsed held ticks.
    fn release_using(
        &self,
        _host: &mut dyn ClientHost,
        _stack: &ItemStack,
        _slot: u32,
        _ticks_held: u32,
    ) {
    }

    /// Clean up after [`Self::release_using`] on a valid release, or on its own when the
    /// capture resets (for example, when the player switches slots, replaces the item kind,
    /// dies or disconnects). The game clears the capture before the end callbacks.
    /// `ticks_held` counts elapsed held ticks.
    fn on_stop_using(
        &self,
        _host: &mut dyn ClientHost,
        _stack: &ItemStack,
        _slot: u32,
        _ticks_held: u32,
    ) {
    }
}

/// The bound owner of client input for a canonical item id, if registered.
pub fn client_item_use(
    item: &str,
) -> Option<(usize, &'static str, Arc<dyn ClientItemUseBehavior>)> {
    let attached = installed()?.keys.client_item_uses.get(item)?;
    Some((attached.mod_index, attached.mod_id, attached.part.clone()))
}

#[derive(Clone)]
pub(crate) struct PlaceProviderPart {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    pub(crate) provider: Arc<dyn PlaceProvider>,
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// A tooltip appender a mod registered, as the game reads it ([`tooltip_appenders`]).
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TooltipAppenderPart {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    pub(crate) component: Box<str>,
    pub(crate) placement: crate::TooltipPlacement,
    pub(crate) appender: Arc<dyn crate::TooltipAppender>,
    /// How many [`VANILLA_TOOLTIP_COMPONENTS`](crate::VANILLA_TOOLTIP_COMPONENTS) come before it
    /// in the sorted order.
    pub(crate) slot: usize,
}

impl TooltipAppenderPart {
    /// The data component whose stacks it is asked about.
    pub fn component(&self) -> &str {
        &self.component
    }
    /// Where its lines go among the other components' lines.
    pub fn placement(&self) -> &crate::TooltipPlacement {
        &self.placement
    }
    /// The appender that adds the lines.
    pub fn appender(&self) -> &dyn crate::TooltipAppender {
        self
    }
    /// How many vanilla tooltip components' lines come before its lines: its lines follow
    /// those of `VANILLA_TOOLTIP_COMPONENTS[slot - 1]` and precede `[slot]`'s.
    pub fn slot(&self) -> usize {
        self.slot
    }
    /// Whether its mod is enabled.
    pub fn enabled(&self) -> bool {
        crate::mod_index_on(self.mod_index)
    }
}

impl crate::TooltipAppender for TooltipAppenderPart {
    fn append(
        &self,
        host: &dyn crate::ClientHost,
        stack: &crate::ItemStack,
        context: &crate::TooltipAppendContext,
        lines: &mut Vec<(crate::Text, u32)>,
    ) {
        let _guard = crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
            id: self.mod_id,
            hook: "tooltip_appender",
            part: Some(std::any::type_name_of_val(&*self.appender)),
        });
        self.appender.append(host, stack, context, lines);
    }
}

/// The tooltip appenders the mods registered, in the order their lines go (see
/// [`TooltipPlacement`](crate::TooltipPlacement)).
pub fn tooltip_appenders() -> &'static [TooltipAppenderPart] {
    installed().map_or(&[], |installed| &installed.keys.tooltip_appenders)
}
/// What answers an item capability for one item: given where the stack lives, its handler, or
/// none to leave it to the next provider. A closure is one.
pub trait ItemCapabilityProvider<T: ?Sized>: Send + Sync {
    /// The handler for the stack at `at`, or none.
    fn provide(&self, host: &mut dyn ServerHost, at: &ItemPlace) -> Option<Box<T>>;
}
impl<T: ?Sized, F> ItemCapabilityProvider<T> for F
where
    F: Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, at: &ItemPlace) -> Option<Box<T>> {
        self(host, at)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// An item capability provider, as the registrar keeps it.
pub type ItemProvider<T> = dyn ItemCapabilityProvider<T>;

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// An item registration request that the loaded item registry cannot resolve.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemKeyError {
    /// The requesting mod.
    pub mod_id: &'static str,
    /// The canonical requested item id.
    pub id: String,
}

impl HookParts {
    pub(crate) fn item<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn ItemPart>] {
        attached(&self.items, id)
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to every item in tag `tag` (`minecraft:axes`, without `#`).
    pub fn item_part_tag<P: ItemPart>(&mut self, tag: &str, part: P) {
        self.attach_item_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to item `item` (`mymod:wrench`).
    pub fn item_part<P: ItemPart>(&mut self, item: crate::ItemKey, part: P) {
        self.attach_item_part(item, type_name::<P>(), Arc::new(part));
    }
    /// Answer item capability `key` for item `item` (`mymod:backpack`) with `provider`, which
    /// gives a handler for the stack at one place, or none: `r.item_capability(&ITEM_ITEMS,
    /// "mymod:backpack", |_, _| Some(Box::new(Backpack)))`. Asked as
    /// `block_entity_capability`'s are. The capability name and item id must be fully qualified
    /// resource identifiers. The item can be declared later in registration; an unknown item
    /// refuses load. The key must match the canonical handler type for the capability name.
    pub fn item_capability<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<ItemProvider<T>> = Arc::new(provider);
        self.provide_item_capability(
            key.name,
            item,
            false,
            NativeCapabilityProvider::of::<T, _>(provider),
        );
    }
    /// `item_capability`, asked ahead of the game's own provider.
    pub fn item_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &str,
        provider: impl Fn(&mut dyn ServerHost, &ItemPlace) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        let provider: Arc<ItemProvider<T>> = Arc::new(provider);
        self.provide_item_capability(
            key.name,
            item,
            true,
            NativeCapabilityProvider::of::<T, _>(provider),
        );
    }
}

/// Whether this item's resolved Parts subscribe to `hook`, before stack conversion.
pub fn item_listens(hook: HookId, id: &str) -> bool {
    if !has_part_hook(hook) {
        return false;
    }
    parts(hook, "item routing")
        .for_hook(hook)
        .is_some_and(|table| !table.item(|| Some(id)).is_empty())
}

/// Registrar operations for items.
pub trait ItemsRegistrar {
    /// Request an item by registry id, validated after all declarations.
    fn item_key(&mut self, id: &str) -> crate::ItemKey;

    /// Register the sole client input behavior for `item` in [`crate::Hooks::register`]. An
    /// enabled owner consumes main-hand use input before crosshair handling, without sending a
    /// use or release packet. This intercepts only main-hand input; it does not enter ordinary
    /// item use or blocking state on either side. Any registered item may be used; a second
    /// registration for the same item refuses startup, even from the same mod.
    fn register_client_item_use(
        &mut self,
        item: crate::ItemKey,
        behavior: Arc<dyn ClientItemUseBehavior>,
    );

    /// Attach `part`, of kind `kind`, to item `item`.
    fn attach_item_part(
        &mut self,
        item: crate::ItemKey,
        kind: &'static str,
        part: Arc<dyn ItemPart>,
    );

    /// Attach `part`, of kind `kind`, to every item in tag `tag`.
    fn attach_item_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn ItemPart>);

    /// Answer item capability `name` for item `item` with `provider`; mods reach it through
    /// `item_capability`.
    fn provide_item_capability(
        &mut self,
        name: &'static str,
        item: &str,
        before_game: bool,
        provider: NativeCapabilityProvider,
    );

    /// Read and write the item places `ItemPlace::Mod { provider: name, .. }` with `provider`,
    /// as an accessory mod exposes its slots. One provider a name.
    fn place_provider(&mut self, name: &'static str, provider: Arc<dyn PlaceProvider>);

    /// Register `appender` as the lines data component `component` (`mymod:charge`) adds to a
    /// stack's tooltip, where `placement` puts them (NeoForge's `RegisterTooltipAppendersEvent`).
    /// Only this mod's appender is asked, for a stack that carries the component and whose
    /// tooltip shows it, and only while the mod is enabled. As NeoForge's event, one appender a
    /// component: a component id that isn't namespaced, one the game or an earlier appender
    /// already adds lines for, a `Before`/`After` anchor that is neither a vanilla tooltip
    /// component nor registered by an appender, or anchors that form a cycle, refuse the mods
    /// at startup, naming them.
    fn register_tooltip_appender(
        &mut self,
        component: &str,
        placement: crate::TooltipPlacement,
        appender: Arc<dyn crate::TooltipAppender>,
    );
}

/// CapabilityProviders operations for items.
// hidden: per-kind lookup implementation for native provider bindings
#[doc(hidden)]
pub trait ItemsCapabilityProviders {
    /// The enabled provider of the item places named `name`, with its mod.
    fn place_provider(&self, name: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)>;
}

impl ItemsCapabilityProviders for InstalledProviders {
    fn place_provider(&self, name: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)> {
        let part = installed()?.keys.place_providers.get(name)?;
        crate::mod_index_on(part.mod_index).then(|| (part.provider.clone(), part.mod_id))
    }
}
