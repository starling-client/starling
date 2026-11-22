//! The creative catalog and build-scoped ordered tab edits.
use crate::{Host, ItemStack, StackWriteError, Text, backend_of};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Wire)]
pub enum CreativeTabList {
    /// Stacks displayed in the tab itself.
    Parent,
    /// Stacks contributed to the global search tab.
    Search,
}

/// Which lists an edit targets (`CreativeModeTab.TabVisibility`).
// exhaustive: both, or one of the only two lists
#[derive(Clone, Copy, Debug, PartialEq, Eq, Wire)]
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
#[derive(Clone, Debug, Wire)]
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

/// Backend request for one edit, with the unconverted API stack. The editor
/// owns count and component comparison for this callback, not a world write.
// hidden: this is the generated backend request; mods use CreativeTabEdit.
// exhaustive: both backends dispatch the closed set of insert and remove requests.
#[doc(hidden)]
#[derive(Clone, Debug, Wire)]
pub enum CreativeTabChange {
    Insert {
        placement: CreativeTabPlacement,
        stack: ItemStack,
        visibility: CreativeTabVisibility,
    },
    Remove {
        stack: ItemStack,
        visibility: CreativeTabVisibility,
    },
}

/// Why a read or edit of the current tab was refused.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Wire)]
pub enum CreativeTabEditError {
    /// This tab's build event is no longer active on the supplied Host.
    Stale,
    /// An inserted stack must have count one.
    Count,
    /// A stack could not be resolved in this world.
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

impl crate::NoBackend for CreativeTabEditError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Edit access for the tab currently running `build_creative_mode_tab_contents`.
/// It works only during that tab's event on its originating logical side.
// exhaustive: its private identity cannot be built by mods
#[derive(Clone, Copy, Debug, PartialEq, Eq, Wire)]
pub struct CreativeTabEdit(u64);

impl CreativeTabEdit {
    pub(crate) fn of(id: u64) -> Self {
        Self(id)
    }

    /// Read the current ordered entries, including edits from earlier listeners.
    pub fn entries(
        self,
        host: &dyn Host,
        list: CreativeTabList,
    ) -> Result<Vec<ItemStack>, CreativeTabEditError> {
        backend_of(host)
            .ok_or(CreativeTabEditError::Stale)?
            .creative_tab_entries(self, list)
    }

    /// Insert a count-one stack. Duplicate and anchor checks use exact item/components.
    pub fn insert(
        self,
        host: &dyn Host,
        placement: CreativeTabPlacement,
        stack: ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        backend_of(host)
            .ok_or(CreativeTabEditError::Stale)?
            .creative_tab_change(
                self,
                CreativeTabChange::Insert {
                    placement,
                    stack,
                    visibility,
                },
            )
    }

    /// Remove an exact item/component match, ignoring count. Absent entries are a no-op.
    pub fn remove(
        self,
        host: &dyn Host,
        stack: &ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        backend_of(host)
            .ok_or(CreativeTabEditError::Stale)?
            .creative_tab_change(
                self,
                CreativeTabChange::Remove {
                    stack: stack.clone(),
                    visibility,
                },
            )
    }

    /// Remove the matches in each selected list, parent first. The predicate runs in the mod.
    pub fn remove_if(
        self,
        host: &dyn Host,
        visibility: CreativeTabVisibility,
        mut predicate: impl FnMut(&ItemStack) -> bool,
    ) -> Result<(), CreativeTabEditError> {
        for (list, only) in [
            (CreativeTabList::Parent, CreativeTabVisibility::Parent),
            (CreativeTabList::Search, CreativeTabVisibility::Search),
        ] {
            if visibility == only || visibility == CreativeTabVisibility::Both {
                for stack in self.entries(host, list)? {
                    if predicate(&stack) {
                        self.remove(host, &stack, only)?;
                    }
                }
            }
        }
        Ok(())
    }
}
