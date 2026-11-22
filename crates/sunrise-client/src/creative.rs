//! Creative inventory (CreativeModeInventoryScreen): vanilla's tabs and their
//! contents as built by the game's own display generators
//! (`creative-inventory.json`), a 9x5 scrolling grid and the search tab.
use crate::{
    Result, item_tooltip,
    items::{ItemStack, Items},
    lang,
    ui::EditBox,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use sunrise_items::{
    creative_catalog::{Catalog, Kind, TabContents},
    item_data::component_snapshot,
};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<ItemStack>,
    pub top: bool,
    pub column: u32,
    pub search: bool,
    pub inventory: bool,
    /// `CreativeModeTab.Type.HOTBAR`: the saved toolbars, filled when the
    /// tab is selected (`set_saved_hotbars`).
    pub hotbar: bool,
    /// The page of tabs it is on: vanilla's are 0, mods' after, as NeoForge
    /// pages them.
    pub page: u32,
    /// A Polytone creative tab modifier's `name`: its text (a translation key when
    /// the flag is set) and colour, over the tab's own.
    pub name: Option<(String, Option<u32>, bool)>,
}
impl Tab {
    /// The tab without its stacks.
    fn header(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            icon: self.icon.clone(),
            items: Vec::new(),
            top: self.top,
            column: self.column,
            search: self.search,
            inventory: self.inventory,
            hotbar: self.hotbar,
            page: self.page,
            name: self.name.clone(),
        }
    }
}
/// What the client's creative search trees (SessionSearchTrees) index for one
/// stack of the search tab: its creative tooltip lines and its item tags.
#[derive(Clone, Debug, Default)]
struct SearchEntry {
    /// The item id's namespace and path, lower-cased.
    namespace: String,
    path: String,
    /// Prepared English tooltip lines, never used on a holder-dependent stack.
    lines: Vec<String>,
    live_tooltip: bool,
    /// Item tags as (namespace, path), lower-cased.
    tags: Vec<(String, String)>,
}
pub struct Creative {
    /// The tabs shown (`CreativeModeTabs.tabs()`), with the stacks the current
    /// permissions put in them.
    pub tabs: Vec<Tab>,
    selected_id: String,
    /// The page of tabs shown.
    pub page: u32,
    /// Scroll position 0..1 (scrollOffs).
    pub scroll: f32,
    /// The scrollbar is held (scrolling): dragging moves it.
    pub scrolling: bool,
    /// CreativeModeInventoryScreen's search box: an unbordered `EditBox` of 50.
    pub search: EditBox,
    /// The grid's rows: vanilla's five, or Bedrock Inventory's six, its hotbar being beside them.
    pub shown_rows: usize,
    /// The search tab's stacks, in its display order.
    search_items: Vec<ItemStack>,
    /// The name a mod item's part gives each search stack (`Item.getName`), by index into
    /// `search_items`; empty while no part names any. Set by the game (`set_search_names`).
    search_names: Vec<Option<crate::text::Text>>,
    /// Counts `project`s: a new value means new search stacks to name.
    search_generation: u64,
    search_index: Vec<SearchEntry>,
    /// Definitions from the same catalog snapshot, rendered in the active language.
    holders: Arc<sunrise_items::creative_catalog::Holders>,
    items: Arc<Items>,
    catalog: Arc<Catalog>,
    headers: Vec<Tab>,
    /// Prepared search presentation only, keyed by item and exact components.
    metadata: HashMap<(Arc<str>, Option<sunrise_core::nbt::Tag>), SearchEntry>,
    /// Non-tabbed content items still live at the search UI's tail.
    ungrouped: Vec<ItemStack>,
    /// World item tags, inverted for search and Polytone presentation.
    item_tags: Option<HashMap<String, Vec<(String, String)>>>,
    contents: Option<Arc<[Arc<TabContents>]>>,
    /// ItemPickerMenu.items: what the grid shows, filled when the tab or the
    /// search changes (selectTab, refreshSearchResults), not on every frame.
    shown: Mutex<Option<Shown>>,
    /// SessionSearchTrees.creativeByNameSearch: the lower-cased tooltip lines
    /// of every search item, built on the first search and kept until the
    /// translations, item list or enchantment registry change.
    lines_cache: Mutex<Option<(u64, Arc<[Vec<String>]>)>>,
    /// The packs' Polytone creative tab modifiers, applied by `rebuild`.
    pack: Vec<crate::polytone_creative_tabs::TabModifier>,
}
/// The same registries a vanilla client finishes configuring, in network ID order.
/// Known-pack entries have already been filled from local resources by configuration.
pub fn live_holders(
    builtin: &sunrise_protocol::registry::Builtin,
    dynamic: &sunrise_protocol::registry::Dynamic,
) -> sunrise_items::creative_catalog::Holders {
    use std::num::NonZeroU8;
    use sunrise_core::nbt::Tag;
    use sunrise_items::creative_catalog::{Enchantment, Holders, Painting};
    let size = |value: &Tag| {
        let n = match *value {
            Tag::Byte(n) => i64::from(n),
            Tag::Short(n) => i64::from(n),
            Tag::Int(n) => i64::from(n),
            Tag::Long(n) => n,
            _ => return None,
        };
        NonZeroU8::new(u8::try_from(n).ok()?)
    };
    let entries = |kind: &str| dynamic.registries.get(kind).into_iter().flatten();
    let text = |data: &Tag, key: &str| data.get(key).map(crate::scoreboard::tag_to_json);
    let enchantments = entries("minecraft:enchantment")
        .filter_map(|(id, data)| {
            let data = data.as_ref()?;
            Some(Enchantment {
                id: id.clone(),
                max_level: size(data.get("max_level")?)?,
                description: text(data, "description").unwrap_or_default(),
            })
        })
        .collect();
    let placeable = dynamic
        .tag("minecraft:painting_variant", "minecraft:placeable")
        .unwrap_or_default();
    let paintings = entries("minecraft:painting_variant")
        .enumerate()
        .filter_map(|(i, (id, data))| {
            let data = data.as_ref()?;
            Some(Painting {
                id: id.clone(),
                width: size(data.get("width")?)?,
                height: size(data.get("height")?)?,
                placeable: placeable.contains(&(i as i32)),
                title: text(data, "title"),
                author: text(data, "author"),
            })
        })
        .collect();
    let horns = dynamic
        .tag("minecraft:instrument", "minecraft:goat_horns")
        .unwrap_or_default()
        .iter()
        .filter_map(|&id| dynamic.name("minecraft:instrument", id).map(str::to_owned))
        .collect();
    let instrument_descriptions = entries("minecraft:instrument")
        .filter_map(|(id, data)| Some((id.clone(), text(data.as_ref()?, "description")?)))
        .collect();
    let banner_patterns = entries("minecraft:banner_pattern")
        .map(|(id, _)| id.clone())
        .collect();
    let potions = builtin
        .get("minecraft:potion")
        .map_or_else(Vec::new, |registry| registry.names.clone());
    Holders {
        potions,
        enchantments,
        paintings,
        horns,
        instrument_descriptions,
        banner_patterns,
    }
}

fn append_unique(items: &mut Vec<ItemStack>, added: &[ItemStack]) {
    for stack in added {
        if stack.count > 0
            && stack.item.as_ref() != "minecraft:air"
            && !items
                .iter()
                .any(|item| item.same_item_same_components(stack))
        {
            items.push(stack.clone());
        }
    }
}

/// A text component's text, its colour (a name, or `#rrggbb`) and whether the
/// text is a `translate` key, to be resolved when drawn.
fn component_text(json: &Value) -> (String, Option<u32>, bool) {
    match json {
        Value::String(s) => (s.clone(), None, false),
        Value::Object(o) => {
            let translate = o.get("translate").and_then(Value::as_str);
            let text = o
                .get("text")
                .and_then(Value::as_str)
                .or(translate)
                .unwrap_or("")
                .to_owned();
            let translate = o.get("text").is_none() && translate.is_some();
            let color = o.get("color").and_then(Value::as_str).and_then(|c| {
                c.strip_prefix('#')
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| crate::text::named_color(c))
            });
            (text, color, translate)
        }
        _ => (String::new(), None, false),
    }
}
/// The grid's stacks for one tab, search text and language.
struct Shown {
    tab: usize,
    search: String,
    language_generation: u64,
    items: Arc<[ItemStack]>,
}
/// Display names from the pinned en_us language file.
#[derive(Default)]
pub struct Names {
    map: HashMap<String, String>,
    item_names: HashMap<String, serde_json::Value>,
    /// The block registry, which says whether an item is a block's (`Item.Properties.useBlockDescriptionPrefix`).
    blocks: Option<&'static sunrise_world::registry::BlockRegistry>,
}
impl Names {
    pub fn load(root: &sunrise_assets::resource_source::ResourceSource) -> Self {
        let read = |namespace: &str| {
            root.read(&format!("assets/{namespace}/lang/en_us.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
                .unwrap_or_default()
        };
        let mut map = read("minecraft");
        // Content mods' names (docs/modpacks.md); none in a vanilla root.
        for namespace in root
            .namespaces("assets")
            .into_iter()
            .filter(|ns| ns != "minecraft")
        {
            map.extend(read(&namespace));
        }
        Self {
            map,
            item_names: HashMap::new(),
            blocks: None,
        }
    }
    /// Keep each item's default `minecraft:item_name` component from the items' prototypes:
    /// `Item.getDescriptionId` is its translation key, so the key is the item's own, never
    /// guessed from the language.
    pub fn with_item_names(
        mut self,
        prototypes: &std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<String, serde_json::Value>,
        >,
        blocks: &'static sunrise_world::registry::BlockRegistry,
    ) -> Self {
        self.blocks = Some(blocks);
        self.item_names = prototypes
            .iter()
            .filter_map(|(id, components)| {
                Some((id.clone(), components.get("minecraft:item_name")?.clone()))
            })
            .collect();
        self
    }
    /// Whether `id` is the item of a block of the same id, which takes its block's `block.*`
    /// description id (a mod block item, whose prototype carries no `item_name`).
    pub fn is_block_item(&self, id: &str) -> bool {
        self.blocks
            .is_some_and(|blocks| blocks.default_state(id).is_some())
    }
    /// An item's default `minecraft:item_name` component, as its prototype has it.
    pub fn item_name(&self, id: &str) -> Option<&serde_json::Value> {
        self.item_names.get(id)
    }
    pub fn item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        self.map
            .get(&format!("item.{namespace}.{path}"))
            .or_else(|| self.map.get(&format!("block.{namespace}.{path}")))
            .cloned()
            .unwrap_or_else(|| path.replace('_', " "))
    }
    /// An item's display name in the selected language: `item.*` then
    /// `block.*`, then the pinned English map (`Names::item`).
    pub fn display_item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        if let Some(lang) = crate::lang::current() {
            for prefix in ["item", "block"] {
                if let Some(name) = lang.get(&format!("{prefix}.{namespace}.{path}")) {
                    return name.to_owned();
                }
            }
        }
        self.item(id)
    }
    /// A translation key's text in the selected language, else the pinned
    /// English map (`HoverName` and tooltip keys such as potion names).
    pub fn display_get(&self, key: &str) -> Option<String> {
        crate::lang::current()
            .and_then(|lang| lang.get(key).map(str::to_owned))
            .or_else(|| self.get(key).map(str::to_owned))
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}

/// `CreativeModeTabs`' key for an exported tab id (`itemGroup.*`); the
/// export's `title` is the English fallback.
pub fn item_group_key(id: &str) -> Option<&'static str> {
    Some(match id.trim_start_matches("minecraft:") {
        "building_blocks" => "itemGroup.buildingBlocks",
        "colored_blocks" => "itemGroup.coloredBlocks",
        "natural_blocks" => "itemGroup.natural",
        "functional_blocks" => "itemGroup.functional",
        "redstone_blocks" => "itemGroup.redstone",
        "hotbar" => "itemGroup.hotbar",
        "search" => "itemGroup.search",
        "tools_and_utilities" => "itemGroup.tools",
        "combat" => "itemGroup.combat",
        "food_and_drinks" => "itemGroup.foodAndDrink",
        "ingredients" => "itemGroup.ingredients",
        "spawn_eggs" => "itemGroup.spawnEggs",
        "op_blocks" => "itemGroup.op",
        "inventory" => "itemGroup.inventory",
        _ => return None,
    })
}
/// A mod tab's own `itemGroup.<mod>.<name>` key, for its id `<mod>:<name>`.
pub fn mod_item_group_key(id: &str) -> String {
    format!("itemGroup.{}", id.replace(':', "."))
}
impl Creative {
    pub const WIDTH: f32 = 195.0;
    pub const HEIGHT: f32 = 136.0;
    /// How far a tab row reaches outside the window, above or below it.
    pub const TAB_ROW_DEPTH: f32 = 28.0;
    pub const TRASH_POSITION: (f32, f32) = (173.0, 112.0);
    /// How far past an item tab's grid of items a stack put down still goes back to the tab.
    pub const GRID_MARGIN: f32 = 4.0;

    /// Whether `mouse`, relative to the screen origin, is on an item tab's grid of `rows` rows of
    /// items or within [`Creative::GRID_MARGIN`] of it. The display slots' hover boxes leave no
    /// gap between them (`isHovering` grows each by a pixel); the margin is Starling's.
    pub fn on_item_grid(mouse: (f32, f32), rows: usize) -> bool {
        let m = Self::GRID_MARGIN;
        let (left, top) = (8.0 - m, 17.0 - m);
        let (right, bottom) = (8.0 + 9.0 * 18.0 + m, 17.0 + rows as f32 * 18.0 + m);
        mouse.0 >= left && mouse.0 < right && mouse.1 >= top && mouse.1 < bottom
    }

    /// Whether `mouse`, relative to the screen origin, is within `reach` of the middle of the
    /// Survival Inventory tab's Destroy Item slot, as a finger means a slot a little past it.
    pub fn near_trash(mouse: (f32, f32), reach: f32) -> bool {
        let (x, y) = Self::TRASH_POSITION;
        (mouse.0 - x - 8.0).abs() <= reach && (mouse.1 - y - 8.0).abs() <= reach
    }

    /// Visible player slots in the creative inventory tab (game slot ids).
    pub fn inventory_slots() -> impl Iterator<Item = usize> {
        (0..36).chain(500..505)
    }

    /// Bedrock Inventory: the crafting grid and its result in the room at the player's right.
    pub fn bedrock_craft_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            100 => Some((129.0, 5.0)),
            101 => Some((147.0, 5.0)),
            102 => Some((129.0, 23.0)),
            103 => Some((147.0, 23.0)),
            200 => Some((173.0, 14.0)),
            _ => None,
        }
    }

    /// CreativeModeInventoryScreen.selectTab positions its InventoryMenu wrappers.
    pub fn inventory_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            9..36 => Some((
                9.0 + ((slot - 9) % 9) as f32 * 18.0,
                54.0 + ((slot - 9) / 9) as f32 * 18.0,
            )),
            500..504 => {
                let position = slot - 500;
                Some((
                    54.0 + (position / 2) as f32 * 54.0,
                    6.0 + (position % 2) as f32 * 27.0,
                ))
            }
            504 => Some((35.0, 20.0)),
            _ => None,
        }
    }
    /// Game slot id of an item tab's first display slot (ItemPickerMenu's
    /// `CustomCreativeSlot`s, 9 by 5); its hotbar row keeps ids 0..9.
    pub const DISPLAY: usize = 900;

    /// Visible slots of an item tab (game slot ids).
    pub fn tab_slots() -> impl Iterator<Item = usize> {
        (Self::DISPLAY..Self::DISPLAY + 45).chain(0..9)
    }

    /// Bedrock Inventory's item window: six rows of items, the last right under the fifth.
    pub fn six_row_slots() -> impl Iterator<Item = usize> {
        Self::DISPLAY..Self::DISPLAY + 54
    }

    /// ItemPickerMenu's slot positions (a sixth row of items goes on after the fifth).
    pub fn tab_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            _ if (Self::DISPLAY..Self::DISPLAY + 54).contains(&slot) => {
                let i = slot - Self::DISPLAY;
                Some((9.0 + (i % 9) as f32 * 18.0, 18.0 + (i / 9) as f32 * 18.0))
            }
            _ => None,
        }
    }
    /// The scrollbar track (insideScrollbar), relative to the screen origin.
    pub const SCROLL_X: f32 = 175.0;
    pub const SCROLL_Y: f32 = 18.0;
    pub const SCROLL_WIDTH: f32 = 14.0;
    pub const SCROLL_HEIGHT: f32 = 112.0;
    /// Load the prepared recipes and the UI-only search metadata.
    pub fn load(root: &Path) -> Result<Self> {
        Self::from_json(&serde_json::from_slice(&fs::read(
            root.join("creative-inventory.json"),
        )?)?)
    }

    pub fn from_json(raw: &Value) -> Result<Self> {
        let catalog = Arc::new(Catalog::parse(raw)?);
        let mut headers = Vec::new();
        let mut metadata = HashMap::new();
        let mut ungrouped = Vec::new();
        for (tab, recipe) in raw["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(catalog.tabs())
        {
            if recipe.kind == Kind::Search {
                for row in tab["search_metadata"].as_array().into_iter().flatten() {
                    let snbt = row["stack"]
                        .as_str()
                        .ok_or("missing search metadata stack")?;
                    let stack = crate::item_data::parse_stack(snbt)?
                        .ok_or("empty search metadata stack")?;
                    let strings = |key: &str| {
                        row[key]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(java_lower)
                    };
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    metadata.insert(
                        (stack.item.clone(), component_snapshot(&stack)),
                        SearchEntry {
                            namespace,
                            path,
                            lines: strings("lines").collect(),
                            live_tooltip: false,
                            tags: strings("tags").map(|tag| split_id(&tag)).collect(),
                        },
                    );
                    if row["ungrouped"] == true {
                        ungrouped.push(stack);
                    }
                }
            }
            let icon = tab["icon"]
                .as_str()
                .and_then(|s| crate::item_data::parse_stack(s).ok().flatten());
            headers.push(Tab {
                id: recipe.id.clone(),
                title: recipe.title.clone(),
                icon: icon.map_or_else(|| "minecraft:stone".to_owned(), |s| s.item.to_string()),
                items: Vec::new(),
                top: tab["row"] == "TOP",
                column: tab["column"].as_u64().unwrap_or(0) as u32,
                search: recipe.kind == Kind::Search,
                inventory: recipe.kind == Kind::Inventory,
                hotbar: recipe.kind == Kind::Hotbar,
                page: tab["page"].as_u64().unwrap_or(0) as u32,
                name: None,
            });
        }
        let mut creative = Self {
            tabs: Vec::new(),
            selected_id: headers
                .first()
                .map_or_else(String::new, |tab| tab.id.clone()),
            page: 0,
            scroll: 0.0,
            scrolling: false,
            search: {
                let mut search = EditBox::new(0.0, 0.0, 80.0, 9.0);
                search.bordered = false;
                search.max_length = 50;
                search.text_color = u32::MAX;
                search
            },
            shown_rows: 5,
            search_items: Vec::new(),
            search_names: Vec::new(),
            search_generation: 0,
            search_index: Vec::new(),
            holders: Default::default(),
            items: Default::default(),
            catalog,
            headers,
            metadata,
            ungrouped,
            item_tags: None,
            contents: None,
            shown: Mutex::new(None),
            lines_cache: Mutex::new(None),
            pack: Vec::new(),
        };
        creative.project();
        Ok(creative)
    }

    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Project the complete canonical lists onto the UI, retaining client-only presentation.
    fn project(&mut self) {
        let mut search = Vec::new();
        let mut tabs = Vec::new();
        for (i, recipe) in self.catalog.tabs().iter().enumerate() {
            let mut tab = self.headers[i].header();
            if let Some(contents) = &self.contents {
                tab.items.clone_from(&contents[i].display);
            }
            if tab.search {
                search = self.with_ungrouped(&tab.items);
            }
            self.apply_pack(&mut tab);
            if self.contents.is_none() || recipe.kind != Kind::Category || !tab.items.is_empty() {
                tabs.push(tab);
            }
        }
        self.tabs = tabs;
        self.search_items = search;
        self.search_names.clear();
        self.search_generation += 1;
        self.index_search();
    }

    /// The built-in sorter keeps the creative UI's ranking for ungrouped items, too.
    pub fn search_order_from(&self, contents: &[Arc<TabContents>]) -> Vec<ItemStack> {
        self.with_ungrouped(self.catalog.search_display(contents))
    }

    fn with_ungrouped(&self, search: &[ItemStack]) -> Vec<ItemStack> {
        let mut order = search.to_vec();
        append_unique(&mut order, &self.ungrouped);
        order
    }

    pub fn built_contents(&self) -> Option<&[Arc<TabContents>]> {
        self.contents.as_deref()
    }

    pub fn has_contents(&self) -> bool {
        self.contents.is_some()
    }

    /// Drop the previous world's projection when its holder provider changes.
    pub fn reset_contents(&mut self) {
        self.contents = None;
        self.holders = Arc::default();
        self.items = Arc::default();
        self.project();
    }

    fn index_search(&mut self) {
        *self
            .lines_cache
            .get_mut()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.search_index = self
            .search_items
            .iter()
            .map(|stack| {
                let key = (stack.item.clone(), component_snapshot(stack));
                let mut entry = self.metadata.get(&key).cloned().unwrap_or_else(|| {
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    SearchEntry {
                        namespace,
                        path,
                        ..Default::default()
                    }
                });
                entry.live_tooltip = matches!(
                    stack.item.as_ref(),
                    "minecraft:enchanted_book"
                        | "minecraft:painting"
                        | "minecraft:goat_horn"
                        | "minecraft:potion"
                        | "minecraft:splash_potion"
                        | "minecraft:lingering_potion"
                        | "minecraft:tipped_arrow"
                ) || !stack.stored_enchantments().is_empty()
                    || !stack.enchantments().is_empty()
                    || stack.data().is_some_and(|data| {
                        data.ominous_banner || data.other("minecraft:instrument").is_some()
                    });
                if let Some(tags) = &self.item_tags {
                    entry.tags = tags.get(stack.item.as_ref()).cloned().unwrap_or_default();
                }
                entry
            })
            .collect();
        self.forget_items();
    }

    /// Tag reload changes the search UI, not the cached catalog membership.
    pub fn set_item_tags(&mut self, tags: Option<&HashMap<String, Vec<String>>>) {
        self.item_tags = tags.map(|tags| {
            let mut by_item = HashMap::<String, Vec<_>>::new();
            let mut tags: Vec<_> = tags.iter().collect();
            tags.sort_unstable_by_key(|(tag, _)| *tag);
            for (tag, items) in tags {
                let tag = split_id(&java_lower(tag));
                for item in items {
                    by_item.entry(item.clone()).or_default().push(tag.clone());
                }
            }
            by_item
        });
        self.index_search();
    }

    /// Apply Polytone's stack, icon and name overlays without rebuilding the catalog.
    pub fn set_pack_modifiers(&mut self, pack: Vec<crate::polytone_creative_tabs::TabModifier>) {
        self.pack = pack;
        self.project();
        self.select_default_if_missing();
    }
    /// A tab after the modifiers aimed at it: its items, icon and name.
    fn apply_pack(&self, tab: &mut Tab) {
        let aimed = self
            .pack
            .iter()
            .filter(|m| m.targets.matches(&tab.id, |_| false));
        for m in aimed {
            let in_tag = |item: &str, tag: &str| {
                let (namespace, path) = split_id(tag);
                let in_tag = |tags: &[(String, String)]| {
                    tags.iter().any(|t| t.0 == namespace && t.1 == path)
                };
                match &self.item_tags {
                    Some(tags) => tags.get(item).is_some_and(|tags| in_tag(tags)),
                    None => self
                        .metadata
                        .iter()
                        .any(|((id, _), entry)| id.as_ref() == item && in_tag(&entry.tags)),
                }
            };
            let mut ids: Vec<String> = tab.items.iter().map(|s| s.item.to_string()).collect();
            m.apply(&mut ids, &in_tag);
            let mut old = std::mem::take(&mut tab.items);
            tab.items = ids
                .into_iter()
                .map(|id| match old.iter().position(|s| *s.item == *id) {
                    Some(i) => old.remove(i),
                    None => ItemStack::new(&id, 1),
                })
                .collect();
            if let Some(icon) = &m.icon {
                tab.icon = icon.clone();
            }
            if let Some(name) = &m.name {
                tab.name = Some(component_text(name));
            }
        }
    }
    /// Publish a complete build. Keep the selected tab's first row if the tab still exists;
    /// otherwise select vanilla's default tab (`refreshCurrentTabContents`).
    pub fn publish(
        &mut self,
        contents: Arc<[Arc<TabContents>]>,
        holders: Arc<sunrise_items::creative_catalog::Holders>,
        items: Arc<Items>,
        names: &Names,
    ) {
        if self
            .contents
            .as_ref()
            .is_some_and(|prior| Arc::ptr_eq(prior, &contents))
        {
            return;
        }
        let row = self.first_row(names);
        self.contents = Some(contents);
        self.holders = holders;
        self.items = items;
        self.project();
        if !self.select_default_if_missing() {
            // ItemPickerMenu.getScrollForRowIndex.
            let hidden = self.hidden_rows(names);
            self.scroll = if hidden > 0 {
                (row as f32 / hidden as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }

    fn select_default_if_missing(&mut self) -> bool {
        if self.tabs.iter().any(|t| t.id == self.selected_id) {
            return false;
        }
        // CreativeModeTabs.getDefaultTab.
        let default = self
            .tabs
            .iter()
            .position(|t| t.id == "minecraft:building_blocks");
        self.select(default.unwrap_or(0));
        true
    }
    /// Where a stack comes in the creative tabs (the search tab lists them
    /// all, in tab order): the same stack's place, else its item's first.
    /// None for items the tabs don't show.
    pub fn order_of(items: &[ItemStack], stack: &ItemStack) -> Option<usize> {
        items
            .iter()
            .position(|s| s.same_item_same_components(stack))
            .or_else(|| items.iter().position(|s| s.item == stack.item))
    }
    /// The tabs without their stacks: what the screen draws of them each frame.
    pub fn tab_headers(&self) -> Vec<Tab> {
        self.tabs.iter().map(Tab::header).collect()
    }
    pub fn tab(&self) -> &Tab {
        if self.tabs.is_empty() {
            // An empty export (`{"tabs":[]}`) or a rebuild that filtered
            // every category tab away leaves no tabs. `tabs.len() - 1`
            // would underflow; return a shared empty tab instead, so
            // `set_permissions`/`select`/`items` keep working.
            static EMPTY_TAB: OnceLock<Tab> = OnceLock::new();
            return EMPTY_TAB.get_or_init(|| Tab {
                id: String::new(),
                title: String::new(),
                icon: "minecraft:stone".to_owned(),
                items: Vec::new(),
                top: true,
                column: 0,
                search: false,
                inventory: false,
                hotbar: false,
                page: 0,
                name: None,
            });
        }
        &self.tabs[self.selected_index()]
    }
    /// The selected tab's position in the current projection, for input and rendering.
    pub fn selected_index(&self) -> usize {
        self.tabs
            .iter()
            .position(|tab| tab.id == self.selected_id)
            .unwrap_or(0)
    }
    /// Starling: the tab the screen works with, without `select`'s search box and scroll: Bedrock
    /// Inventory shows two tabs' windows and goes from one to the other inside a frame.
    pub fn show(&mut self, tab: usize) {
        if let Some(t) = self.tabs.get(tab) {
            self.selected_id.clone_from(&t.id);
        }
    }
    /// The search tab's index (`CreativeModeTabs.searchTab`), the tab the chat
    /// key opens while another one is shown.
    pub fn search_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.search)
    }
    /// selectTab: a different tab empties the search box; the scrollbar goes to the top.
    /// A tab on another page shows that page.
    pub fn select(&mut self, tab: usize) {
        let Some(t) = self.tabs.get(tab) else {
            return;
        };
        if t.id != self.selected_id {
            self.search.set_value("");
            self.selected_id.clone_from(&t.id);
        }
        if !Self::on_every_page(t) {
            self.page = t.page;
        }
        self.scroll = 0.0;
        // The search box is shown, focused and impossible to unfocus only on
        // its own tab; another tab hides it and takes the focus away.
        let search = self.tab().search;
        if search {
            self.search.reset_blink();
        }
        self.search.focused = search;
    }
    /// How many pages of tabs there are (1 without mods' tabs).
    pub fn pages(&self) -> u32 {
        self.tabs.iter().map(|t| t.page + 1).max().unwrap_or(1)
    }
    /// The tabs every page shows (NeoForge's): the right-hand ones, saved
    /// hotbars, search, operator blocks and the inventory.
    fn on_every_page(t: &Tab) -> bool {
        t.column >= 5
    }
    /// Whether tab `t` shows on the page shown.
    pub fn on_page(&self, t: &Tab) -> bool {
        t.page == self.page || Self::on_every_page(t)
    }
    /// Shows page `page` (NeoForge's page buttons) and selects its first tab,
    /// unless the selected tab is one every page shows.
    pub fn set_page(&mut self, page: u32) {
        let page = page.min(self.pages() - 1);
        let kept = self
            .tabs
            .get(self.selected_index())
            .is_some_and(Self::on_every_page);
        if !kept
            && let Some(first) = self
                .tabs
                .iter()
                .position(|t| t.page == page && !Self::on_every_page(t))
        {
            self.select(first);
        }
        self.page = page;
    }
    /// The search tab's stacks.
    pub fn search_items(&self) -> &[ItemStack] {
        &self.search_items
    }
    /// Which projection of the search stacks this is: it changes whenever they are made again.
    pub fn search_generation(&self) -> u64 {
        self.search_generation
    }
    /// The search stacks' resolved name overrides, by index into
    /// [`Self::search_items`]: the search matches them as the stacks' name lines. Nothing but
    /// `None`s keeps the prepared index.
    pub fn set_search_names(&mut self, names: Vec<Option<crate::text::Text>>) {
        self.search_names = if names.iter().any(Option::is_some) {
            names
        } else {
            Vec::new()
        };
        *self.lines_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    /// Opening the screen: vanilla rebuilds CreativeModeInventoryScreen, whose
    /// init() makes a fresh search box and a zeroed scrollbar; only the tab
    /// persists (its static selectedTab). selectTab on the reopened screen
    /// takes the oldTab != tab branch, so it empties and refocuses the box.
    pub fn reopen(&mut self) {
        self.search.set_value("");
        self.release();
        self.scroll = 0.0;
        self.search.focused = self.tab().search;
        if self.search.focused {
            self.search.reset_blink();
        }
    }
    /// `CreativeModeInventoryScreen.refreshSearchResults`: indices into `search_items`,
    /// matching tag text after `#` or tooltip text and ids in the name search.
    pub fn search_results(&self, term: &str, names: &Names) -> Vec<usize> {
        let english = lang::code() == lang::DEFAULT
            && names.display_get("itemGroup.search").as_deref() == names.get("itemGroup.search");
        let local = if term.is_empty() || term.starts_with('#') {
            Arc::from([])
        } else if english
            && self.search_names.is_empty()
            && self
                .search_index
                .iter()
                .all(|e| !e.live_tooltip && !e.lines.is_empty())
        {
            // The exported English lines are the whole index.
            Arc::from([])
        } else {
            let generation = lang::generation();
            let mut cache = self.lines_cache.lock().unwrap_or_else(|e| e.into_inner());
            match cache.as_ref().filter(|(built, _)| *built == generation) {
                Some((_, lines)) => lines.clone(),
                None => {
                    let lines = self.localized_lines(names);
                    *cache = Some((generation, lines.clone()));
                    lines
                }
            }
        };
        self.search_results_in(term, names, &local, english)
    }
    /// `SessionSearchTrees.getTooltipLines`: holder-dependent entries use only
    /// the active definitions and the selected language, never their prepared names.
    fn localized_lines(&self, names: &Names) -> Arc<[Vec<String>]> {
        self.search_items
            .iter()
            .enumerate()
            .map(|(i, stack)| {
                let mut lines = item_tooltip::lines(
                    stack,
                    self.items.get(&stack.item),
                    &item_tooltip::NoAttributeListener,
                    item_tooltip::TooltipHolders::Live(&self.holders),
                    true,
                    &|key| names.display_get(key),
                    // The search cache has no attribute catalog.
                    &|_| None,
                );
                if let Some(first) = lines.first_mut() {
                    let name = match self.search_names.get(i).and_then(Option::as_ref) {
                        Some(name) => crate::scoreboard::comp_text(name.component_view().as_ref()),
                        None => crate::stations::hover_name_text(names, stack),
                    };
                    *first = crate::clipboard::strip_formatting(&name.plain());
                }
                lines
                    .iter()
                    .map(|line| java_trim(line))
                    .filter(|line| !line.is_empty())
                    .map(java_lower)
                    .collect()
            })
            .collect()
    }
    fn search_results_in(
        &self,
        term: &str,
        names: &Names,
        local: &[Vec<String>],
        english: bool,
    ) -> Vec<usize> {
        let all = 0..self.search_index.len();
        if term.is_empty() {
            return all.collect();
        }
        if let Some(tag) = term.strip_prefix('#') {
            // IdSearchTree over the stacks' tags.
            let text = java_lower(tag);
            return all
                .filter(|&i| {
                    let tags = &self.search_index[i].tags;
                    match text.split_once(':') {
                        None => any(tags.iter().map(|t| t.1.as_str()), &text),
                        Some((ns, path)) => {
                            let (ns, path) = (java_trim(ns), java_trim(path));
                            any(tags.iter().map(|t| t.0.as_str()), ns)
                                && any(tags.iter().map(|t| t.1.as_str()), path)
                        }
                    }
                })
                .collect();
        }
        // FullTextSearchTree: tooltip lines in the selected language, and
        // the item id after a colon. Only non-holder stacks may use the
        // prepared English tooltip or the plain item-name fallback.
        let text = java_lower(term);
        all.filter(|&i| {
            let e = &self.search_index[i];
            let local = local.get(i).map_or(&[][..], Vec::as_slice);
            let named = self.search_names.get(i).is_some_and(Option::is_some);
            let hit = |term: &str| {
                if !named && !e.live_tooltip && english && !e.lines.is_empty() {
                    any(e.lines.iter().map(String::as_str), term)
                } else if !named && local.is_empty() && !e.live_tooltip {
                    java_lower(&names.item(&self.search_items[i].item)).contains(term)
                } else {
                    any(local.iter().map(String::as_str), term)
                }
            };
            match text.split_once(':') {
                None => hit(&text),
                Some((ns, path)) => {
                    let (ns, path) = (java_trim(ns), java_trim(path));
                    e.namespace.contains(ns) && (e.path.contains(path) || hit(path))
                }
            }
        })
        .collect()
    }
    /// Items shown by the selected tab (the search tab filters by `search`),
    /// worked out again only when the tab, the search or the language changed.
    pub fn items(&self, names: &Names) -> Arc<[ItemStack]> {
        if self.tabs.is_empty() {
            return Arc::from([]);
        }
        let tab = self.selected_index();
        let search = if self.tab().search {
            self.search.value.as_str()
        } else {
            ""
        };
        let language_generation = lang::generation();
        let mut shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = shown.as_ref()
            && s.tab == tab
            && s.search == search
            && s.language_generation == language_generation
        {
            return s.items.clone();
        }
        let items: Arc<[ItemStack]> = if self.tab().search {
            self.search_results(search, names)
                .into_iter()
                .map(|i| self.search_items[i].clone())
                .collect()
        } else {
            self.tab().items.as_slice().into()
        };
        *shown = Some(Shown {
            tab,
            search: search.to_owned(),
            language_generation,
            items: items.clone(),
        });
        items
    }
    /// The stack in grid cell `index` (row-major from the tab's top), None
    /// for an empty cell: a saved toolbar's empty slot, or past the end.
    pub fn shown(&self, names: &Names, index: usize) -> Option<ItemStack> {
        self.items(names)
            .get(index)
            .filter(|s| s.count > 0)
            .cloned()
    }
    /// `CreativeModeInventoryScreen.selectTab` for the HOTBAR tab: the nine
    /// saved rows of `HotbarManager` as the tab's items, an empty stack (count
    /// 0) for an empty slot.
    pub fn set_saved_hotbars(&mut self, items: Vec<ItemStack>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.hotbar) {
            tab.items = items;
        }
        self.forget_items();
    }
    /// Drop the shown items, so the next `items` works them out again: a
    /// reload can rename items without changing the Search label.
    pub fn forget_items(&self) {
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub fn rows(&self, names: &Names) -> usize {
        self.items(names).len().div_ceil(9)
    }
    /// ItemPickerMenu.calculateRowCount: rows beyond the ones shown.
    fn hidden_rows(&self, names: &Names) -> i32 {
        self.rows(names) as i32 - self.shown_rows as i32
    }
    /// First visible grid row for the scroll position (getRowIndexForScroll).
    pub fn first_row(&self, names: &Names) -> usize {
        ((self.scroll * self.hidden_rows(names) as f32 + 0.5) as i32).max(0) as usize
    }
    /// CreativeModeInventoryScreen.canScroll: more rows than shown, on a tab with a scrollbar.
    pub fn can_scroll(&self, names: &Names) -> bool {
        !self.tab().inventory && self.items(names).len() > self.shown_rows * 9
    }
    /// insideScrollbar, for a point relative to the screen origin.
    pub fn inside_scrollbar(&self, x: f32, y: f32) -> bool {
        x >= Self::SCROLL_X
            && y >= Self::SCROLL_Y
            && x < Self::SCROLL_X + Self::SCROLL_WIDTH
            && y < Self::SCROLL_Y + Self::SCROLL_HEIGHT
            && !self.tab().inventory
    }
    /// mouseClicked on the scrollbar: it is held only if the tab can scroll
    /// (the thumb doesn't move until the mouse does).
    pub fn press_scrollbar(&mut self, x: f32, y: f32, names: &Names) -> bool {
        if !self.inside_scrollbar(x, y) {
            return false;
        }
        self.scrolling = self.can_scroll(names);
        true
    }
    /// mouseDragged while the scrollbar is held, at a cursor height relative to
    /// the screen origin, wherever the cursor is (even off the track or the window).
    pub fn drag_scrollbar(&mut self, y: f32) -> bool {
        if !self.scrolling {
            return false;
        }
        let (top, bottom) = (Self::SCROLL_Y, Self::SCROLL_Y + Self::SCROLL_HEIGHT);
        self.scroll = ((y - top - 7.5) / (bottom - top - 15.0)).clamp(0.0, 1.0);
        true
    }
    /// mouseReleased.
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.scrolling)
    }
    /// mouseScrolled: ItemPickerMenu.subtractInputFromScroll.
    pub fn wheel(&mut self, amount: f64, names: &Names) {
        if !self.can_scroll(names) {
            return;
        }
        let rows = self.hidden_rows(names) as f64;
        self.scroll = (self.scroll - (amount / rows) as f32).clamp(0.0, 1.0);
    }
    /// The scroller sprite's offset from the top of the track.
    pub fn scroller_offset(&self) -> f32 {
        ((Self::SCROLL_HEIGHT - 17.0) * self.scroll) as i32 as f32
    }
    /// Visible categories whose canonical search list contains `stack` by item
    /// and exact components, in registry order. UI-only additions never decide membership.
    pub fn tabs_containing(&self, stack: &ItemStack) -> Vec<usize> {
        let Some(contents) = &self.contents else {
            return Vec::new();
        };
        self.catalog
            .tabs()
            .iter()
            .zip(contents.iter())
            .filter(|(recipe, _)| recipe.kind == Kind::Category)
            .filter(|(_, contents)| {
                contents
                    .search
                    .iter()
                    .any(|shown| same_components(shown, stack))
            })
            .filter_map(|(recipe, _)| self.tabs.iter().position(|tab| tab.id == recipe.id))
            .collect()
    }
    /// The `#namespace:path` lines `getTooltipFromContainerItem` adds for a
    /// search tab stack while the search is a `#` query: the item tags
    /// `updateVisibleTags` keeps for the text after `#` (the path contains it;
    /// with a colon, the namespace and the path each contain their part) that
    /// the stack has.
    pub fn search_tag_lines(&self, stack: &ItemStack) -> Vec<String> {
        let Some(text) = self.search.value.strip_prefix('#') else {
            return Vec::new();
        };
        let Some(entry) = self
            .search_items
            .iter()
            .position(|s| same_components(s, stack))
            .and_then(|i| self.search_index.get(i))
        else {
            return Vec::new();
        };
        let visible = |(ns, path): &(String, String)| match text.split_once(':') {
            None => path.contains(text),
            Some((n, p)) => ns.contains(java_trim(n)) && path.contains(java_trim(p)),
        };
        entry
            .tags
            .iter()
            .filter(|t| visible(t))
            .map(|(ns, path)| format!("#{ns}:{path}"))
            .collect()
    }
    /// Tab button position relative to the screen origin (CreativeModeInventoryScreen.getTabX/Y).
    pub fn tab_position(tab: &Tab) -> (f32, f32) {
        let mut x = tab.column as f32 * 27.0;
        if tab.column >= 5 {
            // Right-aligned tabs (search, operator, inventory).
            x = Self::WIDTH - 27.0 * (7 - tab.column) as f32 + 1.0;
        }
        let y = if tab.top {
            -Self::TAB_ROW_DEPTH
        } else {
            Self::HEIGHT - 4.0
        };
        (x, y)
    }
    /// `CreativeModeInventoryScreen.checkTabHovering`: the tab whose icon
    /// rectangle, 3 pixels in from its 26x32 slot (`isHovering(x + 3, y + 3,
    /// 21, 27)`), holds the cursor, among the tabs of the page shown. The
    /// screen draws its display name.
    pub fn hovered_tab(&self, origin: (f32, f32), cursor: (f32, f32)) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            if !self.on_page(tab) {
                return false;
            }
            let (x, y) = Self::tab_position(tab);
            let (x, y) = (origin.0 + x + 3.0, origin.1 + y + 3.0);
            cursor.0 >= x && cursor.0 < x + 21.0 && cursor.1 >= y && cursor.1 < y + 27.0
        })
    }
    /// `CreativeModeInventoryScreen.extractTabButton`'s cursor: an unselected
    /// tab asks for the pointing hand when the whole-pixel mouse, relative to
    /// the screen origin, is strictly inside its 26x32 button.
    pub fn tab_wants_hand(tab: &Tab, selected: bool, mouse: (f32, f32)) -> bool {
        let (x, y) = Self::tab_position(tab);
        let (mx, my) = (mouse.0.floor(), mouse.1.floor());
        !selected && mx > x && my > y && mx < x + 26.0 && my < y + 32.0
    }
}
/// Whether a suffix array of `strings` has a suffix starting with `text`:
/// some string contains it (none for no strings, even when `text` is empty).
fn any<'a>(strings: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    strings.into_iter().any(|s| s.contains(text))
}
/// `ItemStack.isSameItemSameComponents`, the count left out, as the
/// tabs' `ItemStackLinkedSet` compares: no components and empty ones match.
fn same_components(a: &ItemStack, b: &ItemStack) -> bool {
    let none = crate::item_data::StackData::default();
    a.item == b.item
        && a.damage == b.damage
        && a.data().unwrap_or(&none) == b.data().unwrap_or(&none)
}
/// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the plain
/// tooltip only for a category tab's own grid slots (`CustomCreativeSlot`);
/// everywhere else in the screen the tab names (and, on the search tab's
/// grid, the tag lines) follow the item's name.
pub fn tooltip_lists_tabs(category_tab: bool, grid_slot: bool) -> bool {
    !(category_tab && grid_slot)
}
/// String.toLowerCase(Locale.ROOT).
fn java_lower(s: &str) -> String {
    s.to_lowercase()
}
/// String.trim: strips code points up to U+0020.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
/// Identifier.parse's namespace and path (default namespace `minecraft`).
fn split_id(id: &str) -> (String, String) {
    match id.split_once(':') {
        Some((ns, path)) => (ns.to_owned(), path.to_owned()),
        None => ("minecraft".to_owned(), id.to_owned()),
    }
}
/// The slot under the cursor, for the search tab's keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hover {
    Nothing,
    /// A display slot (`isCreativeSlot`), showing an item or past the end.
    Display {
        filled: bool,
    },
    /// Any other slot: the hotbar row, or the inventory tab's slots.
    Slot,
}

/// `CreativeModeInventoryScreen.keyPressed` on the search tab: whether a
/// key is a hotbar swap on the hovered slot instead of text for the search
/// box. `numeric` is `getNumericKeyValue().isPresent()` (the digit and keypad
/// keys); `hotbar_key` is a `keyHotbarSlots` key. Over an empty display slot
/// the key types (`doQuickSwap` is false), and `checkHotbarKeyPressed` needs
/// a hovered slot and an empty cursor.
pub fn search_key_swaps(numeric: bool, hotbar_key: bool, hover: Hover, carrying: bool) -> bool {
    let quick_swap = hover != Hover::Display { filled: false };
    quick_swap && numeric && hotbar_key && !carrying && hover != Hover::Nothing
}

/// What a click on an item tab's display slot does besides changing the
/// carried stack.
#[derive(Clone, PartialEq, Debug)]
pub enum DisplayClick {
    Nothing,
    /// SWAP: the hotbar slot gets a full stack of the item.
    Hotbar(usize, ItemStack),
    /// SWAP with button 40 (Key F): the off-hand gets a full stack of the item.
    Offhand(ItemStack),
    /// THROW: a copy of the item is dropped.
    Drop(ItemStack),
}

/// Whether a stack has `minecraft:creative_slot_lock`.
pub fn slot_locked(stack: &ItemStack) -> bool {
    stack
        .data()
        .is_some_and(|d| d.other("minecraft:creative_slot_lock").is_some())
}

/// `CreativeModeInventoryScreen.slotClicked` on a display slot
/// (`slot.container == CONTAINER`): `shown` is the slot's item (None past
/// the tab's end) and `max` gives an item's max stack size. The tab's items
/// are never changed: clicks make copies.
pub fn click_display(
    carried: &mut Option<ItemStack>,
    shown: Option<&ItemStack>,
    max: impl Fn(&ItemStack) -> u32,
    button: i32,
    input: crate::menu::Input,
) -> DisplayClick {
    use crate::menu::Input;
    // CustomCreativeSlot.mayPickup: a `creative_slot_lock` stack (the saved
    // toolbars' hint) can't be clicked at all.
    if shown.is_some_and(slot_locked) {
        return DisplayClick::Nothing;
    }
    let copy = |s: &ItemStack, count: u32| ItemStack { count, ..s.clone() };
    let quick_key = input == Input::QuickMove;
    match input {
        Input::Swap => {
            return match shown {
                Some(s) if button == 40 => DisplayClick::Offhand(copy(s, max(s))),
                Some(s) if (0..9).contains(&button) => {
                    DisplayClick::Hotbar(button as usize, copy(s, max(s)))
                }
                Some(_) | None => DisplayClick::Nothing,
            };
        }
        Input::Clone => {
            if carried.is_none()
                && let Some(s) = shown
            {
                *carried = Some(copy(s, max(s)));
            }
            return DisplayClick::Nothing;
        }
        Input::Throw => {
            return match shown {
                Some(s) => DisplayClick::Drop(copy(s, if button == 0 { 1 } else { max(s) })),
                None => DisplayClick::Nothing,
            };
        }
        _ => {}
    }
    match (carried.as_mut(), shown) {
        (Some(c), Some(s)) if c.item == s.item && c.extra == s.extra && c.damage == s.damage => {
            // isSameItemSameComponents: grow, fill or shrink what is carried.
            if button == 0 {
                if quick_key {
                    c.count = max(c);
                } else if c.count < max(c) {
                    c.count += 1;
                }
            } else {
                c.count -= 1;
            }
        }
        (None, Some(s)) => {
            *carried = Some(copy(s, if quick_key { max(s) } else { s.count }));
        }
        (Some(c), _) => {
            if button == 0 {
                *carried = None;
            } else {
                c.count -= 1;
            }
        }
        (None, None) => {}
    }
    if carried.as_ref().is_some_and(|c| c.count == 0) {
        *carried = None;
    }
    DisplayClick::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_items::creative_catalog::Parameters;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// One click on a display slot showing one stone (max 64; pearls 16).
    fn display(
        carried: Option<ItemStack>,
        button: i32,
        input: crate::menu::Input,
    ) -> (Option<ItemStack>, DisplayClick) {
        let mut carried = carried;
        let max = |s: &ItemStack| if s.item.ends_with("pearl") { 16 } else { 64 };
        let effect = click_display(&mut carried, Some(&stack("stone", 1)), max, button, input);
        (carried, effect)
    }

    fn row(stack: impl AsRef<str>) -> Value {
        serde_json::json!({"kind":"stack", "snbt":stack.as_ref(), "visibility":"PARENT_AND_SEARCH_TABS"})
    }

    fn search_only(stack: impl AsRef<str>) -> Value {
        let mut row = row(stack);
        row["visibility"] = serde_json::json!("SEARCH_TAB_ONLY");
        row
    }

    fn fixture(tabs: Value) -> Creative {
        let mut c = Creative::from_json(&serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":tabs,
        }))
        .unwrap();
        c.contents = Some(c.catalog.build(
            Parameters {
                operator: false,
                features: &[],
                holders: &Default::default(),
                items: &crate::items::Items::empty(),
            },
            None,
            |_, _, _| {},
        ));
        c.project();
        c
    }

    /// Tabs as the export lists them: two block tabs sharing the oak log,
    /// ingredients with the top-level book only, the search tab with every
    /// stack (a level 1 book only there) and its tags, the inventory.
    fn tooltip_tabs() -> Creative {
        let book = |level: u8| {
            format!(
                r#"{{components:{{"minecraft:stored_enchantments":{{"minecraft:sharpness":{level}}}}},count:1,id:"minecraft:enchanted_book"}}"#
            )
        };
        let log = r#"{count:1,id:"minecraft:oak_log"}"#;
        let dirt = r#"{count:1,id:"minecraft:dirt"}"#;
        let tab = |id: &str, kind: &str, build: Vec<Value>| serde_json::json!({"id":id,"type":kind,"title":id,"icon":log,"build":build});
        let mut search = tab("minecraft:search", "SEARCH", vec![]);
        search["search_metadata"] = serde_json::json!([
            {"stack":log,"tags":["minecraft:logs","minecraft:oak_logs","minecraft:logs_that_burn"]},
            {"stack":dirt,"tags":["minecraft:dirt"]},
        ]);
        fixture(serde_json::json!([
            tab("minecraft:building_blocks", "CATEGORY", vec![row(log)]),
            tab(
                "minecraft:natural_blocks",
                "CATEGORY",
                vec![row(log), row(dirt)]
            ),
            tab("minecraft:hotbar", "HOTBAR", vec![]),
            search,
            tab(
                "minecraft:ingredients",
                "CATEGORY",
                vec![search_only(book(1)), row(book(5)), {
                    let mut row = row(r#"{id:"minecraft:book",count:1}"#);
                    row["visibility"] = "PARENT_TAB_ONLY".into();
                    row
                }]
            ),
            tab("minecraft:inventory", "INVENTORY", vec![]),
        ]))
    }

    #[test]
    fn holder_projection_uses_registry_and_tag_order() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::{
            packets::configuration::{RegistryTags, TagEntry, UpdateTags},
            registry::{Builtin, Dynamic, Ids},
        };
        let builtin = Builtin {
            registries: HashMap::from([(
                "minecraft:potion".into(),
                Ids::from_names(vec!["minecraft:water".into(), "minecraft:awkward".into()]),
            )]),
        };
        let mut dynamic = Dynamic {
            registries: HashMap::from([
                (
                    "minecraft:enchantment".into(),
                    vec![
                        (
                            "test:z".into(),
                            Some(Tag::compound([("max_level", Tag::Int(3))])),
                        ),
                        (
                            "test:a".into(),
                            Some(Tag::compound([("max_level", Tag::Int(7))])),
                        ),
                    ],
                ),
                (
                    "minecraft:painting_variant".into(),
                    vec![(
                        "test:mural".into(),
                        Some(Tag::compound([
                            ("width", Tag::Int(3)),
                            ("height", Tag::Int(4)),
                        ])),
                    )],
                ),
                (
                    "minecraft:instrument".into(),
                    vec![("test:first".into(), None), ("test:second".into(), None)],
                ),
                (
                    "minecraft:banner_pattern".into(),
                    vec![("test:pattern".into(), None)],
                ),
            ]),
            ..Default::default()
        };
        dynamic.add_tags(&UpdateTags {
            registries: vec![
                RegistryTags {
                    registry: "minecraft:painting_variant".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:placeable".into(),
                        ids: vec![0],
                    }],
                },
                RegistryTags {
                    registry: "minecraft:instrument".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:goat_horns".into(),
                        ids: vec![1, 0],
                    }],
                },
            ],
        });
        let holders = live_holders(&builtin, &dynamic);
        assert_eq!(holders.potions, ["minecraft:water", "minecraft:awkward"]);
        let enchantments: Vec<_> = holders
            .enchantments
            .iter()
            .map(|e| (e.id.as_str(), e.max_level.get()))
            .collect();
        assert_eq!(enchantments, [("test:z", 3), ("test:a", 7)]);
        let mural = &holders.paintings[0];
        assert_eq!(
            (
                mural.id.as_str(),
                mural.width.get(),
                mural.height.get(),
                mural.placeable
            ),
            ("test:mural", 3, 4, true)
        );
        assert_eq!(holders.horns, ["test:second", "test:first"]);
        assert!(holders.banner_patterns.contains("test:pattern"));
    }

    #[test]
    fn search_indexes_the_replaced_enchantment_description_even_at_the_same_level() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::registry::{Builtin, Dynamic};
        let raw = serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":[
                {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[
                    {"kind":"enchantments","all_levels":false,"visibility":"PARENT_AND_SEARCH_TABS"}
                ]},
                {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[], "search_metadata":[
                    {"stack":r#"{components:{"minecraft:stored_enchantments":{"minecraft:sharpness":5}},count:1,id:"minecraft:enchanted_book"}"#, "lines":["Enchanted Book","Sharpness V"]}
                ]}
            ]
        });
        let mut c = Creative::from_json(&raw).unwrap();
        let items = Arc::new(Items::empty());
        let names = Names::default();
        let mut rebuild = |description: &str| {
            let dynamic = Dynamic {
                registries: HashMap::from([(
                    "minecraft:enchantment".into(),
                    vec![(
                        "minecraft:sharpness".into(),
                        Some(Tag::compound([
                            ("max_level", Tag::Int(5)),
                            ("description", Tag::String(description.into())),
                        ])),
                    )],
                )]),
                ..Default::default()
            };
            let holders = live_holders(&Builtin::default(), &dynamic);
            c.reset_contents();
            let lists = c.catalog().build(
                Parameters {
                    features: &[],
                    operator: false,
                    holders: &holders,
                    items: &items,
                },
                None,
                |_, _, _| {},
            );
            c.publish(lists, Arc::new(holders), items.clone(), &names);
            assert_eq!(
                c.search_items()[0].stored_enchantments(),
                [(Arc::from("minecraft:sharpness"), 5)]
            );
            (
                c.search_results("sharpness", &names),
                c.search_results("honed edge", &names),
            )
        };
        assert_eq!(rebuild("Sharpness"), (vec![0], vec![]));
        assert_eq!(rebuild("Honed Edge"), (vec![], vec![0]));
    }

    #[test]
    fn search_matches_visible_lore_on_a_holder_backed_stack() {
        let visible = r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:lore":["Ink under moonlight"]}}"#;
        let hidden = r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:lore":["Ink under moonlight"],"minecraft:tooltip_display":{hidden_components:["minecraft:lore"]}}}"#;
        let creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(visible), row(hidden)]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        assert_eq!(creative.search_items().len(), 2);
        assert_eq!(
            creative.search_results("under moonlight", &Names::default()),
            [0]
        );
    }

    #[test]
    fn search_matches_a_written_book_s_author_only_while_its_content_is_visible() {
        let book = |hidden: &str| {
            format!(
                r#"{{id:"minecraft:written_book",count:1,components:{{"minecraft:enchantments":{{"minecraft:sharpness":1}},"minecraft:written_book_content":{{title:{{raw:"Notes"}},author:"Ada Lovelace",pages:[{{raw:"one"}}]}}{hidden}}}}}"#
            )
        };
        let visible = book("");
        let hidden = book(
            r#","minecraft:tooltip_display":{hidden_components:["minecraft:written_book_content"]}"#,
        );
        let creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(&visible), row(&hidden)]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        assert_eq!(creative.search_items().len(), 2);
        let language = lang::Language::of("en_us", [("book.byAuthor".into(), "by %1$s".into())]);
        lang::scoped(language, || {
            assert_eq!(
                creative.search_results("by ada lovelace", &Names::default()),
                [0]
            );
        });
    }

    #[test]
    fn search_matches_visible_attributes_alongside_live_painting_lines() {
        use sunrise_items::creative_catalog::{Holders, Painting};
        let stack = |hidden: &str| {
            format!(
                r#"{{id:"minecraft:painting",count:1,components:{{
                    "minecraft:painting/variant":"test:mural",
                    "minecraft:attribute_modifiers":[{{type:"minecraft:armor",id:"test:bonus",
                        amount:2.0d,operation:"add_value",slot:"head"}}],
                    "minecraft:tooltip_display":{{hidden_components:[{hidden}]}}
                }}}}"#
            )
        };
        let mut creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[
                row(stack("")),
                row(stack(r#""minecraft:attribute_modifiers""#)),
                row(stack(r#""minecraft:painting/variant""#))
            ]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        creative.holders = Arc::new(Holders {
            paintings: vec![Painting {
                id: "test:mural".into(),
                width: 3.try_into().unwrap(),
                height: 2.try_into().unwrap(),
                placeable: true,
                title: Some(serde_json::json!("Winter Orchard")),
                author: Some(serde_json::json!("Yuki Tanaka")),
            }],
            ..Default::default()
        });
        let names = Names {
            map: HashMap::from([
                ("attribute.name.armor".into(), "Armor".into()),
                ("attribute.modifier.plus.0".into(), "+%s %s".into()),
                ("painting.dimensions".into(), "%s by %s".into()),
            ]),
            ..Default::default()
        };
        let lang = lang::Language::of("en_us", names.map.clone());
        let (local, _) = lang::scoped(lang, || creative.localized_lines(&names));
        let search = |term| creative.search_results_in(term, &names, &local, false);
        assert_eq!(search("+2 armor"), [0, 2]);
        assert_eq!(search("winter orchard"), [0, 1]);
        assert_eq!(search("yuki tanaka"), [0, 1]);
        assert_eq!(search("3 by 2"), [0, 1]);
    }

    #[test]
    fn search_uses_the_active_language_for_an_applied_enchantment() {
        use sunrise_items::creative_catalog::{Enchantment, Holders};
        let tool = r#"{id:"minecraft:wooden_pickaxe",count:1,components:{"minecraft:enchantments":{"test:veins":2},"minecraft:unbreakable":{}}}"#;
        let mut creative = Creative::from_json(&serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":[
                {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(tool)]},
                {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[], "search_metadata":[
                    {"stack":tool, "lines":["Wooden Pickaxe", "Stale Vein Finder II"]}
                ]}
            ]
        }))
        .unwrap();
        let holders = Holders {
            enchantments: vec![Enchantment {
                id: "test:veins".into(),
                max_level: 3.try_into().unwrap(),
                description: serde_json::json!({"translate":"test.enchantment.veins"}),
            }],
            ..Default::default()
        };
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:wooden_pickaxe":{"minecraft:attribute_modifiers":[{
                "type":"minecraft:attack_damage","id":"minecraft:base_attack_damage",
                "amount":3.0,"operation":"add_value","slot":"mainhand"
            }]}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root).unwrap();
        let items = Arc::new(
            Items::catalog(root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap(),
        );
        let lists = creative.catalog().build(
            Parameters {
                features: &[],
                operator: false,
                holders: &holders,
                items: &items,
            },
            None,
            |_, _, _| {},
        );
        let names = Names::default();
        creative.publish(lists, Arc::new(holders), items, &names);
        let language = lang::Language::of(
            "en_us",
            [
                ("test.enchantment.veins".into(), "Ore Sense".into()),
                ("enchantment.level.2".into(), "II".into()),
                (
                    "attribute.name.attack_damage".into(),
                    "Attack Damage".into(),
                ),
                ("attribute.modifier.plus.0".into(), "+%s %s".into()),
                ("item.unbreakable".into(), "Unbreakable".into()),
            ],
        );
        lang::scoped(language, || {
            assert_eq!(creative.search_results("ore sense ii", &names), [0]);
            assert_eq!(creative.search_results("+3 attack damage", &names), [0]);
            assert_eq!(creative.search_results("unbreakable", &names), [0]);
            assert!(
                creative
                    .search_results("stale vein finder", &names)
                    .is_empty()
            );
            assert!(creative.search_results("veins", &names).is_empty());
        });
    }

    #[test]
    fn tag_reload_updates_search_without_asking_for_new_tab_contents() {
        let mut c = tooltip_tabs();
        let names = Names::default();
        let previous = c.contents.clone().unwrap();
        assert_eq!(c.search_results("#oak_logs", &names), [0]);
        c.set_item_tags(Some(&HashMap::from([(
            "test:replacement".into(),
            vec!["minecraft:oak_log".into()],
        )])));
        assert_eq!(c.search_results("#replacement", &names), [0]);
        assert!(c.search_results("#oak_logs", &names).is_empty());
        assert!(Arc::ptr_eq(&previous, c.contents.as_ref().unwrap()));
    }

    #[test]
    fn tooltip_lists_the_tabs_that_contain_the_stack() {
        let c = tooltip_tabs();
        let ids = |s: &ItemStack| -> Vec<&str> {
            c.tabs_containing(s)
                .into_iter()
                .map(|i| c.tabs[i].id.as_str())
                .collect()
        };
        // In tab order, the search and inventory tabs left out; the count
        // doesn't matter.
        assert_eq!(
            ids(&stack("oak_log", 37)),
            ["minecraft:building_blocks", "minecraft:natural_blocks"]
        );
        assert_eq!(ids(&stack("dirt", 1)), ["minecraft:natural_blocks"]);
        // A stack no tab holds: other components, or no creative item.
        let mut damaged = stack("oak_log", 1);
        damaged.damage = 3;
        assert!(ids(&damaged).is_empty());
        assert!(ids(&stack("barrier", 1)).is_empty());
        // A level 1 book is a search-tab-only stack of the ingredients tab.
        let books: Vec<&ItemStack> = c
            .search_items()
            .iter()
            .filter(|s| &*s.item == "minecraft:enchanted_book")
            .collect();
        assert_eq!(ids(books[0]), ["minecraft:ingredients"]);
        assert_eq!(ids(books[1]), ["minecraft:ingredients"]);
        let ingredients = c
            .tabs
            .iter()
            .find(|tab| tab.id == "minecraft:ingredients")
            .unwrap();
        assert!(
            ingredients
                .items
                .iter()
                .any(|s| s.item.as_ref() == "minecraft:book")
        );
        assert!(
            ids(&stack("book", 1)).is_empty(),
            "the display-only stack is not a member"
        );
    }

    #[test]
    fn tooltip_tab_names_skip_only_a_category_tab_grid() {
        assert!(!tooltip_lists_tabs(true, true));
        // The hotbar row of a category tab, the search tab's grid and hotbar,
        // the inventory tab.
        assert!(tooltip_lists_tabs(true, false));
        assert!(tooltip_lists_tabs(false, true));
        assert!(tooltip_lists_tabs(false, false));
    }

    #[test]
    fn tag_search_lists_the_matching_tags_of_the_stack() {
        let mut c = tooltip_tabs();
        let log = stack("oak_log", 1);
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("logs");
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("#logs");
        assert_eq!(
            c.search_tag_lines(&log),
            [
                "#minecraft:logs",
                "#minecraft:oak_logs",
                "#minecraft:logs_that_burn"
            ]
        );
        c.search.set_value("#oak");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:oak_logs"]);
        c.search.set_value("#mine : burn");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:logs_that_burn"]);
        assert!(c.search_tag_lines(&stack("dirt", 1)).is_empty());
    }

    #[test]
    fn search_tab_number_keys_swap_over_a_slot_before_typing() {
        let filled = Hover::Display { filled: true };
        let empty = Hover::Display { filled: false };
        // Over an item, the hotbar row or another slot: the hotbar swap.
        assert!(search_key_swaps(true, true, filled, false));
        assert!(search_key_swaps(true, true, Hover::Slot, false));
        // Over an empty display slot, nothing or with a carried stack: text.
        assert!(!search_key_swaps(true, true, empty, false));
        assert!(!search_key_swaps(true, true, Hover::Nothing, false));
        assert!(!search_key_swaps(true, true, filled, true));
        // Letters always type; so do the digits no hotbar key is bound to.
        assert!(!search_key_swaps(false, false, filled, false));
        assert!(!search_key_swaps(true, false, filled, false));
    }

    #[test]
    fn display_slot_clicks_follow_slot_clicked() {
        use crate::menu::Input::*;
        // An empty hand takes the shown stack, or a full one with Shift.
        assert_eq!(display(None, 0, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 1, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 0, QuickMove).0, Some(stack("stone", 64)));
        // The same item grows by one (left), fills (Shift) or shrinks (right).
        assert_eq!(
            display(Some(stack("stone", 5)), 0, Pickup).0,
            Some(stack("stone", 6))
        );
        assert_eq!(
            display(Some(stack("stone", 64)), 0, Pickup).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 0, QuickMove).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 1, Pickup).0,
            Some(stack("stone", 4))
        );
        assert_eq!(display(Some(stack("stone", 1)), 1, QuickMove).0, None);
        // Another item is deleted (left) or loses one (right).
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, Pickup).0, None);
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, QuickMove).0, None);
        assert_eq!(
            display(Some(stack("ender_pearl", 9)), 1, Pickup).0,
            Some(stack("ender_pearl", 8))
        );
        // Number keys fill a hotbar slot, F fills off-hand, Q drops copies, pick block clones.
        assert_eq!(
            display(None, 3, Swap).1,
            DisplayClick::Hotbar(3, stack("stone", 64))
        );
        assert_eq!(
            display(None, 40, Swap).1,
            DisplayClick::Offhand(stack("stone", 64))
        );
        assert_eq!(
            display(None, 0, Throw).1,
            DisplayClick::Drop(stack("stone", 1))
        );
        assert_eq!(
            display(None, 1, Throw).1,
            DisplayClick::Drop(stack("stone", 64))
        );
        assert_eq!(display(None, 2, Clone).0, Some(stack("stone", 64)));
        assert_eq!(
            display(Some(stack("dirt", 2)), 2, Clone).0,
            Some(stack("dirt", 2))
        );
    }

    #[test]
    fn an_empty_display_slot_deletes_or_shrinks_the_carried_stack() {
        use crate::menu::Input::Pickup;
        let mut carried = Some(stack("dirt", 3));
        click_display(&mut carried, None, |_| 64, 1, Pickup);
        assert_eq!(carried, Some(stack("dirt", 2)));
        click_display(&mut carried, None, |_| 64, 0, Pickup);
        assert_eq!(carried, None);
    }

    fn creative(items: usize) -> Creative {
        let mut stacks: Vec<String> = ["anvil", "crafting_table", "wooden_hoe"]
            .iter()
            .map(|i| format!("{{count:1,id:\"minecraft:{i}\"}}"))
            .collect();
        stacks.extend((0..items).map(|i| format!("{{count:1,id:\"minecraft:item_{i}\"}}")));
        let mut lines = vec![
            vec!["Anvil"],
            vec!["Crafting Table"],
            vec!["Wooden Hoe", "When in Main Hand:", "-3 Attack Speed"],
        ];
        lines.extend(vec![vec!["Thing"]; items]);
        let mut tags = vec![
            vec!["minecraft:anvil"],
            vec![],
            vec!["minecraft:hoes", "minecraft:enchantable/mining"],
        ];
        tags.extend(vec![vec![]; items]);
        let build: Vec<_> = stacks
            .iter()
            .enumerate()
            .map(|(i, stack)| {
                if i < 2 {
                    row(stack)
                } else {
                    search_only(stack)
                }
            })
            .collect();
        let metadata: Vec<_> = stacks.iter().zip(lines).zip(tags).map(|((stack,lines),tags)| {
            serde_json::json!({"stack":stack,"lines":lines,"tags":tags})
        }).collect();
        fixture(serde_json::json!([
            {"id": "minecraft:functional_blocks", "type": "CATEGORY", "row": "TOP", "column": 3, "title": "Functional Blocks",
             "icon": "{count:1,id:\"minecraft:oak_sign\"}", "build":build},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": "{count:1,id:\"minecraft:command_block\"}", "build":[]},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": "{count:1,id:\"minecraft:compass\"}", "build":[], "search_metadata":metadata},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "build":[]}
        ]))
    }

    #[test]
    fn six_rows_of_items_have_no_gap() {
        for (slot, expected) in [(0, (9.0, 18.0)), (44, (153.0, 90.0)), (45, (9.0, 108.0))] {
            assert_eq!(
                Creative::tab_slot_position(Creative::DISPLAY + slot),
                Some(expected)
            );
        }
    }

    #[test]
    fn the_item_grid_takes_its_cells_the_pixels_between_them_and_a_margin() {
        // A cell's middle, the pixels between two items, and the grid's corners past the cells.
        for mouse in [(17.0, 26.0), (25.5, 35.5), (4.0, 13.0), (173.9, 110.9)] {
            assert!(Creative::on_item_grid(mouse, 5), "{mouse:?}");
        }
        // Past the margin, the scrollbar, and the hotbar under five rows.
        for mouse in [(3.9, 26.0), (17.0, 12.9), (175.0, 40.0), (17.0, 112.0)] {
            assert!(!Creative::on_item_grid(mouse, 5), "{mouse:?}");
        }
        // Bedrock Inventory's sixth row, which has no hotbar under it.
        assert!(Creative::on_item_grid((17.0, 116.0), 6));
        assert!(!Creative::on_item_grid((17.0, 129.0), 6));
    }

    #[test]
    fn a_finger_means_the_destroy_item_slot_a_little_past_it() {
        // Its middle, and 11 pixels off it (half a cell and 2 beyond).
        assert!(Creative::near_trash((181.0, 120.0), 11.0));
        assert!(Creative::near_trash((170.0, 131.0), 11.0));
        // The last hotbar slot's middle, beside it.
        assert!(!Creative::near_trash((161.0, 120.0), 11.0));
        assert!(!Creative::near_trash((181.0, 132.0), 11.0));
    }

    #[test]
    fn inventory_tab_slot_layout_matches_vanilla() {
        assert_eq!(Creative::inventory_slots().count(), 41);
        for (slot, expected) in [
            (0, (9.0, 112.0)),
            (8, (153.0, 112.0)),
            (9, (9.0, 54.0)),
            (35, (153.0, 90.0)),
            (500, (54.0, 6.0)),
            (501, (54.0, 33.0)),
            (502, (108.0, 6.0)),
            (503, (108.0, 33.0)),
            (504, (35.0, 20.0)),
        ] {
            assert_eq!(Creative::inventory_slot_position(slot), Some(expected));
        }
        assert_eq!(Creative::inventory_slot_position(100), None);
        assert_eq!(Creative::TRASH_POSITION, (173.0, 112.0));
        let mut c = creative(90);
        c.select(2);
        assert!(c.tab().inventory);
        assert!(!c.can_scroll(&Names {
            map: HashMap::new(),
            ..Default::default()
        }));
    }

    #[test]
    fn inventory_tab_can_return_to_category_and_search() {
        let mut c = creative(90);
        for (selected, inventory, search) in [
            (2, true, false),
            (0, false, false),
            (1, false, true),
            (2, true, false),
        ] {
            c.search.set_value("anvil");
            c.scroll = 0.5;
            c.select(selected);
            assert_eq!(c.selected_index(), selected);
            assert_eq!(c.tab().inventory, inventory);
            assert_eq!(c.tab().search, search);
            assert!(c.search.value.is_empty());
            assert_eq!(c.scroll, 0.0);
        }
    }

    #[test]
    fn hidden_tabs_are_skipped() {
        let c = creative(0);
        let ids: Vec<&str> = c.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minecraft:functional_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.tabs[0].items[0].item.as_ref(), "minecraft:anvil");
        let mut c = c;
        c.select(1);
        c.search.set_value("anvil");
        c.scroll = 0.5;
        c.select(1);
        assert_eq!((c.search.value.as_str(), c.scroll), ("anvil", 0.0));
        c.select(0);
        assert_eq!(c.search.value, "");
    }

    #[test]
    fn stepping_through_tabs_turns_their_pages() {
        let stack = |id: &str| format!("{{count:1,id:\"minecraft:{id}\"}}");
        let tab = |id: &str, column: u32, page: u32| {
            serde_json::json!({"id": format!("minecraft:{id}"), "type": "CATEGORY", "row": "TOP",
                "column": column, "page": page, "title": id, "icon": stack("stone"),
                "build": [row(stack("stone"))]})
        };
        let mut c = fixture(serde_json::json!([
            tab("first", 0, 0),
            tab("second", 1, 0),
            tab("modded", 0, 1),
            tab("more", 1, 2),
            tab("everywhere", 6, 0)
        ]));
        assert_eq!(c.pages(), 3);
        let count = c.tabs.len();
        let mut pages = Vec::new();
        for step in 1..=count {
            let tab = (c.selected_index() + 1) % count;
            c.select(tab);
            assert_eq!(c.selected_index(), tab, "step {step}");
            assert!(
                c.on_page(&c.tabs[tab]),
                "tab {tab} selected on another page"
            );
            pages.push(c.page);
        }
        // A tab every page shows keeps the page it was reached from; then it wraps.
        assert_eq!(pages, [0, 1, 2, 2, 0]);
        // Backwards off the first tab, across pages, the same way.
        c.select((c.selected_index() + count - 1) % count);
        assert_eq!(c.page, 0);
        c.select((c.selected_index() + count - 1) % count);
        assert_eq!(c.page, 2);
    }

    /// An export built with permissions: the operator tab's stacks, and their
    /// place at the end of the search tab, only with them.
    fn with_operator_tab() -> Creative {
        let stack = |id: &str| format!("{{count:1,id:\"minecraft:{id}\"}}");
        fixture(serde_json::json!([
            {"id": "minecraft:building_blocks", "type": "CATEGORY", "row": "TOP", "column": 0, "title": "Building Blocks",
             "icon": stack("bricks"), "build": [row(stack("stone"))]},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": stack("command_block"), "build": [], "operator_build": [row(stack("command_block")),row(stack("barrier"))]},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": stack("compass"), "build": [], "search_metadata": [
                 {"stack":stack("stone"), "lines":["Stone"]},
                 {"stack":stack("command_block"), "lines":["Command Block"]},
                 {"stack":stack("barrier"), "lines":["Barrier"]}
             ]},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": stack("chest"), "build": []}
        ]))
    }

    fn tab_ids(c: &Creative) -> Vec<&str> {
        c.tabs.iter().map(|t| t.id.as_str()).collect()
    }

    fn publish_with(
        c: &mut Creative,
        permissions: bool,
        names: &Names,
        mods: impl Fn(&str, bool) -> Vec<ItemStack>,
    ) {
        let lists = c.catalog.build(
            Parameters {
                operator: permissions,
                features: &[],
                holders: &Default::default(),
                items: &crate::items::Items::empty(),
            },
            c.built_contents(),
            |tab, _, current| {
                let stacks = mods(tab.id.as_str(), permissions);
                append_unique(&mut current.display, &stacks);
                append_unique(&mut current.search, &stacks);
            },
        );
        c.publish(lists, Arc::default(), Arc::default(), names);
    }

    fn publish(c: &mut Creative, permissions: bool, names: &Names) {
        publish_with(c, permissions, names, |_, _| Vec::new());
    }

    fn ids(items: &[ItemStack]) -> Vec<&str> {
        items.iter().map(|s| s.item.as_ref()).collect()
    }

    #[test]
    fn operator_stacks_stay_out_without_permissions() {
        let c = with_operator_tab();
        let names = Names::default();
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.search_items().len(), 1);
        assert!(c.search_results("command", &names).is_empty());
        assert_eq!(c.search_results("", &names), [0]);
    }

    #[test]
    fn operator_tab_comes_and_goes_with_permissions() {
        let mut c = with_operator_tab();
        let names = Names::default();
        publish(&mut c, true, &names);
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:op_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        let op: Vec<&str> = c.tabs[1].items.iter().map(|s| s.item.as_ref()).collect();
        assert_eq!(op, ["minecraft:command_block", "minecraft:barrier"]);
        assert_eq!(c.search_results("command", &names), [1]);
        assert_eq!(c.search_results("", &names), [0, 1, 2]);
        // The search tab keeps its place across the rebuild.
        c.select(2);
        c.search.set_value("block");
        publish(&mut c, false, &names);
        assert!(c.tab().search);
        assert_eq!(c.search.value, "block");
        assert!(c.items(&names).is_empty());
        c.reset_contents(); // the operator header now precedes Search again
        publish(&mut c, false, &names);
        assert!(c.tab().search);
        assert_eq!(c.search.value, "block");
        // The selected operator tab is left empty: the default tab instead.
        publish(&mut c, true, &names);
        c.select(1);
        assert_eq!(c.items(&names).len(), 2);
        publish(&mut c, false, &names);
        assert_eq!(c.tab().id, "minecraft:building_blocks");
        assert_eq!(c.items(&names)[0].item.as_ref(), "minecraft:stone");
    }

    #[test]
    fn mods_stacks_follow_the_tabs_own_and_join_the_search_tab() {
        let mut c = with_operator_tab();
        let names = Names::default();
        let mods = |tab: &str, _: bool| match tab {
            "minecraft:building_blocks" => vec![
                ItemStack::new("ae2:sky_stone_block", 1),
                // Already in the tab and the search tab: not listed twice.
                ItemStack::new("minecraft:stone", 1),
                // Empty stacks are left out.
                ItemStack::new("ae2:quartz_block", 0),
                ItemStack::new("minecraft:air", 1),
                // Listed once, though accepted twice.
                ItemStack::new("ae2:sky_stone_block", 1),
            ],
            _ => Vec::new(),
        };
        publish_with(&mut c, false, &names, mods);
        assert_eq!(
            ids(&c.tabs[0].items),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
        assert_eq!(
            ids(c.search_items()),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
        // Searched by its name, which it has no exported line for.
        assert_eq!(c.search_results("sky", &names), [1]);
        assert_eq!(c.search_results("ae2:", &names), [1]);
        c.select(0);
        assert_eq!(
            ids(&c.items(&names)),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
    }

    #[test]
    fn a_rebuild_asks_the_mods_again() {
        let mut c = with_operator_tab();
        let names = Names::default();
        // An operator-only stack a mod lists with permissions alone.
        let mods = |tab: &str, permissions: bool| {
            if tab == "minecraft:op_blocks" && permissions {
                vec![ItemStack::new("ae2:debug_card", 1)]
            } else {
                Vec::new()
            }
        };
        publish_with(&mut c, false, &names, mods);
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        publish_with(&mut c, true, &names, mods);
        assert_eq!(
            ids(&c.tabs[1].items),
            [
                "minecraft:command_block",
                "minecraft:barrier",
                "ae2:debug_card"
            ]
        );
        assert_eq!(
            ids(c.search_items()),
            [
                "minecraft:stone",
                "minecraft:command_block",
                "minecraft:barrier",
                "ae2:debug_card"
            ]
        );
        // Without permissions the mod's stack goes with the tab.
        publish_with(&mut c, false, &names, mods);
        assert_eq!(ids(c.search_items()), ["minecraft:stone"]);
    }

    #[test]
    fn a_mods_stack_brings_back_an_empty_category_tab() {
        let mut c = with_operator_tab();
        let names = Names::default();
        // `shouldDisplay`: a category tab with a stack is shown, whoever added it.
        publish_with(&mut c, false, &names, |tab, _| {
            if tab == "minecraft:op_blocks" {
                vec![ItemStack::new("ae2:creative_energy_cell", 1)]
            } else {
                Vec::new()
            }
        });
        assert_eq!(tab_ids(&c)[1], "minecraft:op_blocks");
        assert_eq!(ids(&c.tabs[1].items), ["ae2:creative_energy_cell"]);
    }

    #[test]
    fn saved_hotbars_tab_shows_its_rows_and_locks_the_hint() {
        let mut c = fixture(serde_json::json!([
            {"id": "minecraft:hotbar", "type": "HOTBAR", "row": "TOP", "column": 5, "title": "Saved Hotbars",
             "icon": "{count:1,id:\"minecraft:bookshelf\"}", "build": []},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "build": []}
        ]));
        assert!(c.tab().hotbar);
        assert!(!c.tab_headers()[1].hotbar);
        let mut hint = stack("paper", 1);
        hint.data_mut().set_other(
            "minecraft:creative_slot_lock",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        // A saved sword in slot 0, an empty slot, then the hint.
        c.set_saved_hotbars(vec![stack("diamond_sword", 1), stack("air", 0), hint]);
        let names = Names {
            map: HashMap::new(),
            ..Default::default()
        };
        assert_eq!(c.shown(&names, 0), Some(stack("diamond_sword", 1)));
        assert_eq!(c.shown(&names, 1), None);
        assert_eq!(c.shown(&names, 3), None);
        let hint = c.shown(&names, 2).unwrap();
        assert!(slot_locked(&hint));
        assert!(!slot_locked(&stack("diamond_sword", 1)));
        // CustomCreativeSlot.mayPickup: the hint is never taken, and a carried
        // stack stays as it is.
        use crate::menu::Input::*;
        for (carried, button, input) in [
            (None, 0, Pickup),
            (None, 0, QuickMove),
            (None, 2, Clone),
            (None, 3, Swap),
            (None, 0, Throw),
            (Some(stack("dirt", 2)), 0, Pickup),
            (Some(stack("dirt", 2)), 1, Pickup),
        ] {
            let mut carried = carried;
            let before = carried.clone();
            let effect = click_display(&mut carried, Some(&hint), |_| 64, button, input);
            assert_eq!(effect, DisplayClick::Nothing);
            assert_eq!(carried, before);
        }
    }

    /// `CreativeModeInventoryScreen.keyPressed`: from a tab that is not the
    /// search, the chat key selects `CreativeModeTabs.searchTab`, which focuses
    /// the search box and empties the query the other tab left.
    #[test]
    fn the_chat_key_opens_the_search_tab() {
        let mut c = creative(0);
        c.search.set_value("anvil");
        let tab = c.search_tab().expect("the fixture has a search tab");
        assert!(c.tabs[tab].search);
        assert!(!c.tab().search);
        c.select(tab);
        assert!(c.tab().search);
        assert!(c.search.focused);
        assert_eq!(c.search.value, "", "another tab empties the query");
    }

    #[test]
    fn search_matches_vanilla_trees() {
        let c = creative(0);
        let names = Names {
            map: HashMap::new(),
            ..Default::default()
        };
        let q = |s: &str| c.search_results(s, &names);
        assert_eq!(q(""), [0, 1, 2]);
        assert_eq!(q("ANVIL"), [0]);
        assert_eq!(q("table"), [1]);
        assert_eq!(q("a"), [0, 1, 2]);
        // Tooltip lines, not just names; no trimming without a colon.
        assert_eq!(q("attack speed"), [2]);
        assert_eq!(q(" anvil"), Vec::<usize>::new());
        // A colon searches the id: namespace, then the path or a tooltip line.
        assert_eq!(q("minecraft:crafting_t"), [1]);
        assert_eq!(q(" minecraft : hoe "), [2]);
        assert_eq!(q("mine:anvil"), [0]);
        assert_eq!(q("other:anvil"), Vec::<usize>::new());
        assert_eq!(q(":"), [0, 1, 2]);
        // Tags after "#": only stacks with tags.
        assert_eq!(q("#"), [0, 2]);
        assert_eq!(q("#hoe"), [2]);
        assert_eq!(q("#minecraft:enchantable/"), [2]);
        assert_eq!(q("#craft"), Vec::<usize>::new());
    }

    #[test]
    fn search_matches_the_display_name_in_the_selected_language() {
        let c = creative(0);
        let names = Names {
            map: HashMap::from([
                ("item.minecraft.anvil".to_owned(), "铁砧".to_owned()),
                ("item.minecraft.wooden_hoe".to_owned(), "木锄".to_owned()),
            ]),
            ..Default::default()
        };
        let lang = lang::Language::of("zh_cn", names.map.clone());
        let (local, _) = lang::scoped(lang, || c.localized_lines(&names));
        let q = |s: &str| c.search_results_in(s, &names, &local, false);
        assert_eq!(q("铁砧"), [0]);
        assert_eq!(q("木锄"), [2]);
        // Only the selected language's lines count outside English.
        assert!(q("anvil").is_empty());
        assert_eq!(c.search_results_in("anvil", &names, &local, true), [0]);
        // A colon's path searches the display name too.
        assert_eq!(q("minecraft:铁砧"), [0]);
    }

    #[test]
    fn search_matches_every_localized_tooltip_line() {
        // Vanilla's FullTextSearchTree indexes the selected language's
        // tooltip lines; the export's English lines do not apply.
        let c = creative(0);
        let names = Names::default();
        let lang = lang::Language::of(
            "ru_ru",
            HashMap::from([("item.minecraft.anvil".into(), "Наковальня".into())]),
        );
        let (local, _) = lang::scoped(lang, || c.localized_lines(&names));
        let q = |s: &str, english| c.search_results_in(s, &names, &local, english);
        assert_eq!(q("наков", false), [0]);
        assert!(q("anvil", false).is_empty());
        assert_eq!(q("anvil", true), [0]);
    }

    #[test]
    fn creative_search_rerenders_retained_names_when_language_changes() {
        let mut creative = creative(0);
        let names = Names::default();
        let key = "item.test.anvil_name";
        let english = lang::Language::of("en_us", [(key.into(), "Metalworking".into())]);
        lang::scoped(english, || {
            creative.set_search_names(vec![Some(crate::text::Text::translate(key, &[]))]);
            assert_eq!(creative.search_results("metalworking", &names), [0]);
        });

        let german = lang::Language::of("de_de", [(key.into(), "Schmieden".into())]);
        lang::scoped(german, || {
            assert_eq!(creative.search_results("schmieden", &names), [0]);
            assert!(creative.search_results("metalworking", &names).is_empty());
        });
    }

    #[test]
    fn search_strips_legacy_formatting_across_name_components() {
        let mut creative = creative(0);
        let name = crate::text::Text::literal("Ruby§").append(crate::text::Text::literal("aGem"));
        creative.set_search_names(vec![Some(name)]);
        assert_eq!(creative.search_results("rubygem", &Names::default()), [0]);
    }

    #[test]
    fn scrollbar_drag_follows_the_mouse_while_held() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
            ..Default::default()
        };
        c.select(1);
        // Pressing the track holds the scrollbar without moving it.
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(c.scrolling);
        assert_eq!(c.scroll, 0.0);
        // Dragging maps the cursor height onto the track (minus half the thumb).
        assert!(c.drag_scrollbar(18.0 + 7.5 + 48.5));
        assert!((c.scroll - 0.5).abs() < 1e-6);
        // The cursor may leave the track, the panel or the window: it keeps scrolling.
        c.drag_scrollbar(400.0);
        assert_eq!(c.scroll, 1.0);
        c.drag_scrollbar(-50.0);
        assert_eq!(c.scroll, 0.0);
        c.drag_scrollbar(18.0 + 7.5 + 97.0 * 0.25);
        assert!((c.scroll - 0.25).abs() < 1e-6);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        assert_eq!(c.first_row(&names), (0.25 * rows + 0.5) as usize);
        // Releasing stops it.
        c.release();
        assert!(!c.drag_scrollbar(400.0));
        assert!((c.scroll - 0.25).abs() < 1e-6);
        // Off the track, or with too few items, it isn't held.
        assert!(!c.press_scrollbar(170.0, 60.0, &names));
        assert!(!c.press_scrollbar(180.0, 130.0, &names));
        c.select(0);
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(!c.scrolling);
    }

    /// `CreativeModeInventoryScreen.selectTab`: the search box is focused on
    /// its own tab and loses the focus when another tab is shown (and, hidden,
    /// is no longer the field a key types into).
    #[test]
    fn the_search_box_loses_the_focus_with_its_tab() {
        let mut c = creative(0);
        let search = c
            .tabs
            .iter()
            .position(|t| t.search)
            .expect("the fixture has a search tab");
        c.select(search);
        assert!(c.search.focused);
        c.select(0);
        assert!(!c.search.focused, "another tab takes the focus away");
        assert!(!c.tab().search);
        c.select(search);
        assert!(c.search.focused, "and the search tab takes it back");
    }

    #[test]
    fn creative_tab_titles_have_vanilla_keys() {
        assert_eq!(
            item_group_key("minecraft:building_blocks"),
            Some("itemGroup.buildingBlocks")
        );
        assert_eq!(
            item_group_key("minecraft:food_and_drinks"),
            Some("itemGroup.foodAndDrink")
        );
        assert_eq!(item_group_key("minecraft:hotbar"), Some("itemGroup.hotbar"));
        assert_eq!(item_group_key("minecraft:nonsense"), None);
    }

    #[test]
    fn wheel_moves_one_row_per_notch() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
            ..Default::default()
        };
        c.select(1);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        c.wheel(-1.0, &names);
        assert!((c.scroll - 1.0 / rows).abs() < 1e-6);
        assert_eq!(c.first_row(&names), 1);
        c.wheel(5.0, &names);
        assert_eq!(c.scroll, 0.0);
        assert_eq!(c.scroller_offset(), 0.0);
        c.scroll = 1.0;
        assert_eq!(c.scroller_offset(), 95.0);
    }

    #[test]
    fn search_edits_at_its_cursor() {
        use winit::keyboard::KeyCode;
        let mut c = creative(0);
        c.select(1);
        c.search.insert_text("hello world");
        for _ in 0..5 {
            assert!(c.search.key(KeyCode::ArrowLeft, false, false));
        }
        c.search.insert_text("X");
        assert_eq!(c.search.value, "hello Xworld");
        c.search.key(KeyCode::Home, false, true);
        c.search.insert_text("Y");
        assert_eq!(c.search.value, "Yworld");
        // Ctrl+Left jumps a word; one step right and Backspace deletes before the cursor.
        c.search.key(KeyCode::ArrowLeft, true, false);
        c.search.key(KeyCode::ArrowRight, false, false);
        c.search.key(KeyCode::Backspace, false, false);
        assert_eq!(c.search.value, "world");
    }

    #[test]
    fn unselected_tabs_want_the_hand_strictly_inside() {
        let tab = |top, column| Tab {
            id: String::new(),
            title: String::new(),
            icon: String::new(),
            items: Vec::new(),
            top,
            column,
            search: false,
            inventory: false,
            hotbar: false,
            page: 0,
            name: None,
        };
        // Top row, second column: the button spans x 27..53, y -28..4.
        let t = tab(true, 1);
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, true, (40.0, -10.0)));
        // Its edges ask for nothing; the mouse counts in whole pixels.
        assert!(!Creative::tab_wants_hand(&t, false, (27.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (27.9, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (28.0, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (52.9, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (53.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, -28.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -27.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, 3.9)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, 4.0)));
        // Bottom row, right-aligned inventory column: x 169..195, y 132..164.
        let b = tab(false, 6);
        assert!(Creative::tab_wants_hand(&b, false, (180.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (169.0, 140.0)));
        assert!(Creative::tab_wants_hand(&b, false, (170.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (180.0, 132.0)));
    }

    /// An empty export (`{"tabs":[]}`) leaves no tabs. `tab()` and
    /// `items()` used `tabs.len() - 1`, which underflows and panics;
    /// both now handle the empty case.
    #[test]
    fn empty_creative_tabs_do_not_panic_on_tab_or_items() {
        let c = fixture(serde_json::json!([]));
        assert!(c.tabs.is_empty());
        // Fallback tab: no panic, empty id, not search.
        assert_eq!(c.tab().id, "");
        assert!(!c.tab().search);
        let names = Names {
            map: HashMap::new(),
            ..Default::default()
        };
        assert!(c.items(&names).is_empty());
        assert!(c.tab_headers().is_empty());
        assert_eq!(c.search_tab(), None);
    }

    /// `CreativeModeInventoryScreen.checkTabHovering`: the 3-pixel inset of a
    /// tab's 26x32 slot (21x27) is the hover area whose name the screen shows.
    #[test]
    fn only_a_tabs_inset_icon_is_hovered() {
        let screen = tooltip_tabs();
        let tab = &screen.tabs[0];
        let (x, y) = Creative::tab_position(tab);
        let at = |dx: f32, dy: f32| screen.hovered_tab((0.0, 0.0), (x + dx, y + dy));
        assert_eq!(at(4.0, 4.0), Some(0));
        assert_eq!(at(23.5, 4.0), Some(0));
        assert_eq!(at(2.0, 4.0), None);
        assert_eq!(at(24.0, 4.0), None);
        assert_eq!(at(4.0, 2.0), None);
        assert_eq!(at(4.0, 30.0), None);
    }

    /// NeoForge's pages: a tab on another page is not hovered, even where it
    /// shares a slot with one on the page shown (#2646).
    #[test]
    fn only_the_shown_pages_tabs_are_hovered() {
        let tab = |id: &str, column: u32, page: u32| {
            serde_json::json!({"id": format!("minecraft:{id}"), "type": "CATEGORY", "row": "TOP",
                "column": column, "page": page, "title": id, "build": []})
        };
        let raw = serde_json::json!({"schema": 4, "item_features": {}, "potion_features": {}, "tabs": [
            tab("first", 0, 0), tab("second", 1, 0), tab("modded", 0, 1), tab("everywhere", 6, 0)
        ]});
        let mut c = Creative::from_json(&raw).unwrap();
        c.set_page(1);
        let at = |c: &Creative, i: usize| {
            let (x, y) = Creative::tab_position(&c.tabs[i]);
            c.hovered_tab((0.0, 0.0), (x + 4.0, y + 4.0))
        };
        assert_eq!(at(&c, 0), Some(2));
        assert_eq!(at(&c, 1), None);
        assert_eq!(at(&c, 3), Some(3));
    }
}

#[cfg(test)]
mod source_name_tests {
    #[test]
    fn item_names_read_selected_namespace_resources_without_files() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "assets/reader/lang/en_us.json".into(),
            Resource::bytes(br#"{"item.reader.widget":"Selected widget"}"#.to_vec()),
        )]));
        let names = super::Names::load(&source);
        assert_eq!(names.item("reader:widget"), "Selected widget");
    }
}
