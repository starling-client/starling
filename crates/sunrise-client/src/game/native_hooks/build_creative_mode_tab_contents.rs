//! One demand-driven creative catalog per logical side, and the scoped edit event.
use super::*;
use crate::game::api_values::{TryIntoApi, TryIntoEngine, stack_value};
use mod_api::{
    CreativeModeTabInfo, CreativeModeTabs, CreativeTabChange, CreativeTabEdit,
    CreativeTabEditError, CreativeTabList, CreativeTabPermissions, CreativeTabPlacement,
    CreativeTabReadError, CreativeTabVisibility,
};
use std::cell::{OnceCell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_items::creative_catalog::{
    Catalog, Holders, Kind, Parameters, TabContents, normalize_components,
};

use sunrise_items::items::Items;

pub(in crate::game) type Contents = Arc<[Arc<TabContents>]>;

struct BuildKey {
    features: Arc<[String]>,
    permissions: CreativeTabPermissions,
}

impl BuildKey {
    fn matches(&self, other: &Self) -> bool {
        self.permissions == other.permissions
            && (self.features == other.features
                || (other
                    .features
                    .iter()
                    .all(|flag| self.features.contains(flag))
                    && self
                        .features
                        .iter()
                        .all(|flag| other.features.contains(flag))))
    }
}

struct Published {
    key: BuildKey,
    lists: Contents,
    api: OnceCell<Arc<CreativeModeTabs>>,
    holders: Arc<Holders>,
    items: Arc<Items>,
}

struct ActiveBuild {
    key: BuildKey,
    partial: Contents,
    items: Arc<Items>,
    edit: RefCell<Option<EditSession>>,
}

struct EditSession {
    token: CreativeTabEdit,
    contents: TabContents,
}

/// Replaced with the holder provider, never with tags or presentation. Thus the
/// cache key need not carry a registry pointer that changes on a tag-only update.
#[derive(Default)]
pub(in crate::game) struct CreativeCache {
    published: Option<Published>,
    active: Option<ActiveBuild>,
}

impl Game<'_> {
    pub(in crate::game) fn reset_creative_contents(&mut self) {
        self.creative_contents = CreativeCache::default();
        self.creative_screen.reset_contents();
    }

    /// Fire after one tab's generator with the live edit session installed.
    fn native_build_creative_mode_tab_contents(
        &mut self,
        tab: &str,
        features: Vec<String>,
        permissions: mod_api::CreativeTabPermissions,
        edit: mod_api::CreativeTabEdit,
    ) {
        self.fire_build_creative_mode_tab_contents(tab, features, permissions, edit);
    }

    pub(in crate::game) fn creative_permissions(&self) -> CreativeTabPermissions {
        if self.can_use_game_master_blocks() && self.settings.operator_items_tab {
            CreativeTabPermissions::Operator
        } else {
            CreativeTabPermissions::Normal
        }
    }

    pub(in crate::game) fn build_creative(
        &mut self,
        permissions: CreativeTabPermissions,
    ) -> Result<Contents, CreativeTabReadError> {
        // A connection may exist in configuration without a live world provider.
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        let key = BuildKey {
            features: self.enabled_feature_ids().clone(),
            permissions,
        };
        if let Some(active) = &self.creative_contents.active {
            if active.key.matches(&key) {
                return Ok(active.partial.clone());
            }
            return Err(CreativeTabReadError::Reentrant);
        }
        if let Some(published) = &self.creative_contents.published
            && published.key.matches(&key)
        {
            return Ok(published.lists.clone());
        }
        let holders = self
            .creative_holders()
            .ok_or(CreativeTabReadError::Unavailable)?;
        let items = match (&self.server_items, &self.world_tables) {
            (Some(items), _) => Arc::clone(items),
            (_, Some(tables)) => Arc::clone(&tables.items),
            _ => self.res.shared_items(),
        };
        let catalog = self.creative_screen.catalog().clone();
        let prior = self
            .creative_contents
            .published
            .as_ref()
            .map(|p| p.lists.clone());
        let listen = mod_api::__private::listened(mod_api::HookId::BuildCreativeModeTabContents);
        self.creative_contents.active = Some(ActiveBuild {
            key: BuildKey {
                features: key.features.clone(),
                permissions,
            },
            partial: Arc::from([]),
            items: items.clone(),
            edit: RefCell::new(None),
        });
        let lists = catalog.build(
            Parameters {
                features: &key.features,
                operator: permissions == CreativeTabPermissions::Operator,
                holders: &holders,
                items: &items,
            },
            prior.as_deref(),
            |tab, partial, current| {
                if listen {
                    static NEXT_EDIT: AtomicU64 = AtomicU64::new(0);
                    let token = mod_api::__private::creative_tab_edit(
                        NEXT_EDIT.fetch_add(1, Ordering::Relaxed),
                    );
                    let active = self
                        .creative_contents
                        .active
                        .as_mut()
                        .expect("active creative build");
                    active.partial = Arc::from(partial);
                    *active.edit.get_mut() = Some(EditSession {
                        token,
                        contents: std::mem::take(current),
                    });
                    self.native_build_creative_mode_tab_contents(
                        &tab.id,
                        key.features.to_vec(),
                        permissions,
                        token,
                    );
                    *current = self
                        .creative_contents
                        .active
                        .as_mut()
                        .expect("active creative build")
                        .edit
                        .get_mut()
                        .take()
                        .expect("active tab edit")
                        .contents;
                }
            },
        );
        self.creative_contents.active = None;
        self.creative_contents.published = Some(Published {
            key,
            lists: lists.clone(),
            api: OnceCell::new(),
            holders: Arc::new(holders),
            items,
        });
        Ok(lists)
    }

    fn creative_holders(&self) -> Option<Holders> {
        match &self.remote {
            Some(remote) => Some(sunrise_client::creative::live_holders(
                &remote.builtin,
                &remote.registries,
            )),
            None => self.registry_sync().map(|sync| sync.creative_holders()),
        }
    }

    /// The screen uses the completed cache, with the same holder definitions for search text.
    pub(in crate::game) fn refresh_creative_tabs(&mut self) {
        let Ok(lists) = self.build_creative(self.creative_permissions()) else {
            return;
        };
        if let Some(published) = &self.creative_contents.published
            && Arc::ptr_eq(&lists, &published.lists)
        {
            self.creative_screen.publish(
                lists,
                published.holders.clone(),
                published.items.clone(),
                &self.pack.names,
            );
            self.refresh_creative_search_names();
        }
    }

    /// The name components that replace the creative search's prepared name lines.
    pub(in crate::game) fn refresh_creative_search_names(&mut self) {
        // Only when the search tab's stacks were made again, not every creative tick.
        let generation = self.creative_screen.search_generation();
        if self.creative_search_named == Some(generation) {
            return;
        }
        self.creative_search_named = Some(generation);
        let names = self
            .creative_screen
            .search_items()
            .iter()
            .map(|stack| {
                if sunrise_client::stations::names_itself(stack) {
                    Some(self.stack_hover_name_text(stack))
                } else {
                    self.mod_item_name(stack)
                }
            })
            .collect();
        self.creative_screen.set_search_names(names);
    }

    pub(in crate::game) fn creative_tabs(
        &mut self,
        permissions: CreativeTabPermissions,
    ) -> Result<Arc<CreativeModeTabs>, CreativeTabReadError> {
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        let lists = self.build_creative(permissions)?;
        let catalog = self.creative_screen.catalog();
        if self.creative_contents.active.is_some() {
            return Ok(snapshot(catalog, permissions, &lists));
        }
        let published = self
            .creative_contents
            .published
            .as_ref()
            .expect("complete creative build");
        Ok(published
            .api
            .get_or_init(|| snapshot(catalog, permissions, &lists))
            .clone())
    }

    /// The last completed build's catalog, for a reader that cannot build: unavailable outside
    /// a live world's phase, and `None` before the side has built once.
    pub(in crate::game) fn built_creative_tabs(
        &self,
    ) -> Result<Option<Arc<CreativeModeTabs>>, CreativeTabReadError> {
        if self.native_lifecycle.epoch().is_none() {
            return Err(CreativeTabReadError::Unavailable);
        }
        Ok(self.creative_contents.published.as_ref().map(|published| {
            published
                .api
                .get_or_init(|| {
                    snapshot(
                        self.creative_screen.catalog(),
                        published.key.permissions,
                        &published.lists,
                    )
                })
                .clone()
        }))
    }

    /// The search tab's canonical display list (not the client-only UI additions).
    pub(in crate::game) fn creative_search_items(&mut self) -> Vec<ItemStack> {
        if self.native_lifecycle.epoch().is_none() {
            return Vec::new();
        }
        let Ok(lists) = self.build_creative(self.creative_permissions()) else {
            return Vec::new();
        };
        self.creative_screen
            .catalog()
            .search_display(&lists)
            .to_vec()
    }

    /// The built-in sorter is only a reader. A cold sort previews stock order
    /// without publishing anything or giving mods a creative callback.
    pub(in crate::game) fn stock_sort_order(&self) -> Vec<ItemStack> {
        let preview;
        let lists = if let Some(published) = &self.creative_contents.published {
            &published.lists
        } else {
            preview = self.creative_screen.catalog().build(
                Parameters {
                    features: self.enabled_feature_ids(),
                    operator: false,
                    holders: &self.creative_holders().unwrap_or_default(),
                    items: self.items(),
                },
                None,
                |_, _, _| {},
            );
            &preview
        };
        self.creative_screen.search_order_from(lists)
    }

    pub(in crate::game) fn creative_stack_key(&self, stack: &ItemStack) -> Option<ItemStack> {
        self.native_lifecycle.epoch()?;
        // Use the very same prototypes as the generator during a reentrant read.
        let items = self
            .creative_contents
            .active
            .as_ref()
            .map_or_else(|| self.items(), |active| &active.items);
        comparison_key(stack.clone(), items).ok()
    }

    pub(in crate::game) fn creative_tab_entries(
        &self,
        edit: CreativeTabEdit,
        list: CreativeTabList,
    ) -> Result<Vec<ItemStack>, CreativeTabEditError> {
        self.edit_creative(edit, |contents, _| Ok(list_items(contents, list).clone()))
    }

    pub(in crate::game) fn creative_tab_change(
        &self,
        edit: CreativeTabEdit,
        change: CreativeTabChange,
    ) -> Result<(), CreativeTabEditError> {
        match change {
            CreativeTabChange::Insert {
                placement,
                stack,
                visibility,
            } => self.creative_tab_insert(edit, placement, stack, visibility),
            CreativeTabChange::Remove { stack, visibility } => {
                self.creative_tab_remove(edit, &stack, visibility)
            }
        }
    }

    fn creative_tab_insert(
        &self,
        edit: CreativeTabEdit,
        placement: CreativeTabPlacement,
        stack: mod_api::ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        self.edit_creative(edit, |contents, items| {
            if stack.count() != 1 {
                return Err(CreativeTabEditError::Count);
            }
            let mut stack = stack.try_into_engine(items)?;
            normalize_components(&mut stack, items);
            let (anchor, after) = match &placement {
                CreativeTabPlacement::Before(anchor) => {
                    (Some(comparison_key(stack_value(anchor)?, items)?), false)
                }
                CreativeTabPlacement::After(anchor) => {
                    (Some(comparison_key(stack_value(anchor)?, items)?), true)
                }
                _ => (None, false),
            };
            for list in target_lists(visibility) {
                let entries = list_items(contents, *list);
                if entries
                    .iter()
                    .any(|entry| entry.same_item_same_components(&stack))
                {
                    return Err(CreativeTabEditError::Duplicate(*list));
                }
                let index = match &anchor {
                    Some(anchor) => {
                        entries
                            .iter()
                            .position(|entry| entry.same_item_same_components(anchor))
                            .ok_or(CreativeTabEditError::MissingAnchor(*list))?
                            + usize::from(after)
                    }
                    None if matches!(placement, CreativeTabPlacement::First) => 0,
                    None => entries.len(),
                };
                entries.insert(index, stack.clone());
            }
            Ok(())
        })
    }

    fn creative_tab_remove(
        &self,
        edit: CreativeTabEdit,
        stack: &mod_api::ItemStack,
        visibility: CreativeTabVisibility,
    ) -> Result<(), CreativeTabEditError> {
        self.edit_creative(edit, |contents, items| {
            let stack = comparison_key(stack_value(stack)?, items)?;
            for list in target_lists(visibility) {
                list_items(contents, *list)
                    .retain(|entry| !entry.same_item_same_components(&stack));
            }
            Ok(())
        })
    }

    fn edit_creative<T>(
        &self,
        edit: CreativeTabEdit,
        apply: impl FnOnce(&mut TabContents, &Items) -> Result<T, CreativeTabEditError>,
    ) -> Result<T, CreativeTabEditError> {
        let Some(active) = &self.creative_contents.active else {
            return Err(CreativeTabEditError::Stale);
        };
        let mut session = active
            .edit
            .try_borrow_mut()
            .map_err(|_| CreativeTabEditError::Stale)?;
        let session = session
            .as_mut()
            .filter(|s| s.token == edit)
            .ok_or(CreativeTabEditError::Stale)?;
        apply(&mut session.contents, &active.items)
    }
}

fn api_stacks(stacks: &[ItemStack]) -> Vec<mod_api::ItemStack> {
    stacks
        .iter()
        .filter_map(|item| item.try_into_api().ok())
        .collect()
}

fn snapshot(
    catalog: &Catalog,
    permissions: CreativeTabPermissions,
    lists: &[Arc<TabContents>],
) -> Arc<CreativeModeTabs> {
    let tabs = catalog
        .tabs()
        .iter()
        .zip(lists)
        .map(|(tab, stacks)| {
            CreativeModeTabInfo::new(
                tab.id.clone(),
                tab.kind == Kind::Category,
                display_name(&tab.id, &tab.title),
                api_stacks(&stacks.display),
                api_stacks(&stacks.search),
            )
        })
        .collect();
    Arc::new(CreativeModeTabs::new(permissions, tabs))
}

/// `CreativeModeTab.getDisplayName`: the tab's `itemGroup.*` key with its title as fallback.
fn display_name(id: &str, title: &str) -> mod_api::Text {
    let key = sunrise_client::creative::item_group_key(id).map_or_else(
        || sunrise_client::creative::mod_item_group_key(id),
        str::to_owned,
    );
    mod_api::Text::new(mod_api::text::TextContent::Translatable {
        key,
        fallback: Some(title.to_owned()),
        arguments: Vec::new(),
    })
}

// Comparison ignores count and resident maxima, but damage must fit the component codec.
fn comparison_key(
    mut stack: ItemStack,
    items: &Items,
) -> Result<ItemStack, mod_api::StackWriteError> {
    if i32::try_from(stack.damage).is_err() {
        return Err(mod_api::StackWriteError::InvalidComponents {
            reason: mod_api::StackError::InvalidDamage,
        });
    }
    normalize_components(&mut stack, items);
    Ok(stack)
}

fn target_lists(visibility: CreativeTabVisibility) -> &'static [CreativeTabList] {
    use CreativeTabList::{Parent, Search};
    match visibility {
        CreativeTabVisibility::Parent => &[Parent],
        CreativeTabVisibility::Search => &[Search],
        CreativeTabVisibility::Both => &[Parent, Search],
    }
}

fn list_items(contents: &mut TabContents, list: CreativeTabList) -> &mut Vec<ItemStack> {
    match list {
        CreativeTabList::Parent => &mut contents.display,
        CreativeTabList::Search => &mut contents.search,
    }
}

#[cfg(test)]
#[path = "build_creative_mode_tab_contents/tests.rs"]
mod tests;
