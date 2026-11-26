//! The creative catalog and build-scoped ordered tab edits.
use crate::{Host, ItemStack, ItemStackView, StackWriteError, Text, backend_of};
use sunrise_mod_api_macros::Wire;

/// The creative catalog's permission parameter, not permission to use any item.
// exhaustive: vanilla's hasPermissions is a boolean
#[derive(Clone, Copy, Debug, PartialEq, Eq, Wire)]
pub enum CreativeTabPermissions {
    /// Leave operator-only items out of the generators.
    Normal,
    /// Run the generators with `hasPermissions`.
    Operator,
}

/// Why the creative catalog could not be read.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Wire)]
pub enum CreativeTabReadError {
    /// This Host has no active holder provider, or the catalog could not be loaded.
    Unavailable,
    /// A build is in progress for different parameters on this logical side.
    Reentrant,
}

impl crate::NoBackend for CreativeTabReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// One registered tab's contents, as [`ItemsHost::creative_tabs`](crate::prelude::ItemsHost::creative_tabs) reads them.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Wire)]
pub struct CreativeModeTabInfo {
    /// The tab's registry id (`minecraft:ingredients`, or `<mod id>:<name>`).
    pub id: String,
    /// Whether this is an item tab, rather than Search, Hotbar or Inventory.
    pub category: bool,
    /// `CreativeModeTab.getDisplayName`: the tab's title, still a translatable component.
    /// A vanilla tab's key is its `itemGroup.*` one; a mod tab's is `itemGroup.<mod id>.<name>`
    /// from its id. Both carry the tab's title text as the fallback.
    pub display_name: Text,
    /// `getDisplayItems`: what this tab shows, in order.
    pub display_items: Vec<ItemStack>,
    /// `getSearchTabDisplayItems`: this tab's contribution to search, in order.
    pub search_items: Vec<ItemStack>,
}

impl CreativeModeTabInfo {
    /// The ordered lists of one tab in the registry.
    pub fn new(
        id: String,
        category: bool,
        display_name: Text,
        display_items: Vec<ItemStack>,
        search_items: Vec<ItemStack>,
    ) -> Self {
        Self {
            id,
            category,
            display_name,
            display_items,
            search_items,
        }
    }

    /// `CreativeModeTab.contains`: exact item and components in the search list, ignoring count.
    /// Use the originating world's Host to interpret the query against its item prototypes.
    pub fn contains(&self, host: &dyn Host, stack: &ItemStack) -> bool {
        backend_of(host)
            .and_then(|backend| backend.creative_stack_key(stack))
            .is_some_and(|key| {
                self.search_items
                    .iter()
                    .any(|listed| listed.is_stackable_with(&key))
            })
    }
}

/// An immutable snapshot of the creative catalog built on one logical side.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Wire)]
pub struct CreativeModeTabs {
    /// The build's permission parameter.
    pub permissions: CreativeTabPermissions,
    /// Every registered tab in registry order, including empty and special tabs.
    pub tabs: Vec<CreativeModeTabInfo>,
}

impl CreativeModeTabs {
    /// The tabs built with these permissions.
    pub fn new(permissions: CreativeTabPermissions, tabs: Vec<CreativeModeTabInfo>) -> Self {
        Self { permissions, tabs }
    }

    /// The tab with registry id `id`.
    pub fn tab(&self, id: &str) -> Option<&CreativeModeTabInfo> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The item tabs whose display list contains the exact item and components, ignoring count.
    /// Use the originating world's Host to interpret the query against its item prototypes.
    pub fn tabs_showing<'a>(
        &'a self,
        host: &dyn Host,
        stack: &ItemStack,
    ) -> impl Iterator<Item = &'a CreativeModeTabInfo> + use<'a> {
        let key = backend_of(host).and_then(|backend| backend.creative_stack_key(stack));
        self.tabs.iter().filter(move |tab| {
            tab.category
                && key.as_ref().is_some_and(|key| {
                    tab.display_items
                        .iter()
                        .any(|shown| shown.is_stackable_with(key))
                })
        })
    }
}

/// One of a tab's ordered item lists.
// exhaustive: these are the two collections the event edits
// no wire: selects one list of the borrowed creative tab output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreativeTabList {
    /// Stacks displayed in the tab itself.
    Parent,
    /// Stacks contributed to the global search tab.
    Search,
}

/// Which lists an edit targets (`CreativeModeTab.TabVisibility`).
// exhaustive: both, or one of the only two lists
// no wire: selects edits on the borrowed creative tab output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreativeTabVisibility {
    /// Edit the parent list only.
    Parent,
    /// Edit the search list only.
    Search,
    /// Edit parent first, then search; a search error leaves any parent edit in place.
    Both,
}

/// Where to put an inserted stack, independently in each target list.
// exhaustive: all four positions allowed by the build event
// no wire: describes an insertion into the borrowed creative tab output.
#[derive(Clone, Debug)]
pub enum CreativeTabPlacement {
    /// Append at the end.
    Last,
    /// Insert at the front.
    First,
    /// Insert just before an exact item/component match (anchor count is ignored).
    Before(ItemStack),
    /// Insert just after an exact item/component match (anchor count is ignored).
    After(ItemStack),
}

/// Why an operation on the current tab's output was refused.
// no wire: reports a refusal on the borrowed creative tab output.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreativeTabEditError {
    /// An inserted stack must have count one.
    Count,
    /// An input stack or an entry could not be represented in this build.
    Stack(StackWriteError),
    /// The target list already contains the inserted stack, ignoring count.
    Duplicate(CreativeTabList),
    /// The target list does not contain the requested anchor, ignoring count.
    MissingAnchor(CreativeTabList),
}

impl From<StackWriteError> for CreativeTabEditError {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

/// The ordered parent and search lists for a single creative mode tab build.
/// [`crate::BuildCreativeModeTabContentsEvent::output`] lends this editor while the
/// listener runs. Later listeners see the edits; the editor cannot be saved for later.
///
/// ```compile_fail
/// use mod_api::{BuildCreativeModeTabContentsEvent, CreativeTabOutput};
/// fn keep_editor(event: &mut BuildCreativeModeTabContentsEvent<'_>) -> &'static mut dyn CreativeTabOutput {
///     event.output
/// }
/// ```
// no wire: borrows the live tab build and lends its entries in the current callback.
pub trait CreativeTabOutput {
    /// Visit the current entries of one list in order, including earlier listeners' edits.
    /// Reading an entry's item, count or damage does not copy its components. To keep an
    /// entry or inspect its components, call [`ItemStackView::project`]. If an entry cannot
    /// be lent, this returns its error without visiting the remaining entries.
    fn entries(
        &self,
        list: CreativeTabList,
        visit: &mut dyn FnMut(ItemStackView<'_>),
    ) -> Result<(), CreativeTabEditError>;

    /// Insert a count-one stack. Checks the anchor and rejects duplicates by exact item and
    /// components, ignoring count. When editing both lists, a search error leaves a successful
    /// parent insertion in place.
    fn insert(
        &mut self,
        placement: CreativeTabPlacement,
        stack: ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError>;

    /// Remove an exact item and component match, ignoring count. An absent entry is a no-op.
    fn remove(
        &mut self,
        stack: &ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError>;

    /// Remove the matches in one pass per selected list, parent first, preserving the order
    /// of survivors. The predicate receives a borrowed entry and runs once per entry in each
    /// list. If an entry cannot be lent, earlier removals stand and the rest are left untouched.
    fn remove_if(
        &mut self,
        visibility: CreativeTabVisibility,
        predicate: &mut dyn FnMut(ItemStackView<'_>) -> bool,
    ) -> Result<(), CreativeTabEditError>;
}
