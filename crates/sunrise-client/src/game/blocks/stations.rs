//! Workstation menus in the game: enchanting table, anvil, grindstone,
//! smithing table, stonecutter and the brewing stand screen. Layouts, sprites,
//! colours and sounds follow the vanilla screens (`EnchantmentScreen`,
//! `AnvilScreen`, `GrindstoneScreen`, `SmithingScreen`, `StonecutterScreen`,
//! `BrewingStandScreen`); item rules live in `sunrise_client::stations`.

#[path = "stations/anvil.rs"]
mod anvil;
#[path = "stations/draw.rs"]
mod draw;
#[path = "stations/tooltip.rs"]
mod tooltip;
use super::smithing_icons::smithing_slot_icons;
struct EngineTooltipLines<'a>(&'a mut Vec<(sunrise_client::text::Text, u32)>);

impl mod_api::TooltipLinesAccess for EngineTooltipLines<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn color(&self, index: usize) -> Option<u32> {
        self.0.get(index).map(|&(_, color)| color)
    }

    fn get(&self, index: usize) -> Option<Result<(mod_api::Text, u32), mod_api::TextFormError>> {
        self.0.get(index).map(TryIntoApi::try_into_api)
    }

    fn push(&mut self, text: mod_api::Text, color: u32) -> Result<(), mod_api::TextFormError> {
        self.0.push((text.try_into_engine(&())?, color));
        Ok(())
    }

    fn insert(
        &mut self,
        index: usize,
        text: mod_api::Text,
        color: u32,
    ) -> Result<bool, mod_api::TextFormError> {
        if index > self.0.len() {
            return Ok(false);
        }
        self.0.insert(index, (text.try_into_engine(&())?, color));
        Ok(true)
    }

    fn set(
        &mut self,
        index: usize,
        text: mod_api::Text,
        color: u32,
    ) -> Result<bool, mod_api::TextFormError> {
        if index >= self.0.len() {
            return Ok(false);
        }
        let converted = text.try_into_engine(&())?;
        self.0[index] = (converted, color);
        Ok(true)
    }

    fn set_color(&mut self, index: usize, color: u32) -> bool {
        let Some((_, slot)) = self.0.get_mut(index) else {
            return false;
        };
        *slot = color;
        true
    }

    fn remove(
        &mut self,
        index: usize,
    ) -> Option<Result<(mod_api::Text, u32), mod_api::TextFormError>> {
        if index >= self.0.len() {
            return None;
        }
        let removed = self.0.remove(index);
        Some(removed.try_into_api())
    }

    fn pop(&mut self) -> Option<Result<(mod_api::Text, u32), mod_api::TextFormError>> {
        self.0.pop().map(|removed| removed.try_into_api())
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.0.len() || b >= self.0.len() {
            return false;
        }
        self.0.swap(a, b);
        true
    }
}

/// Invalid measurements must not reach tooltip positioning or reserve empty space.
fn tooltip_component_size(size: Option<[f32; 2]>) -> Option<[f32; 2]> {
    size.filter(|size| size.iter().all(|v| v.is_finite() && *v > 0.0))
}

fn tooltip_position(
    (x, y): (f32, f32),
    (width, height): (f32, f32),
    (w, h): (f32, f32),
) -> (f32, f32) {
    let mut x = (x + 12.0).round();
    let mut y = (y - 12.0).round();
    if x + w > width - 4.0 {
        x = (x - 28.0 - w).max(4.0);
    }
    if x + w > width - 4.0 {
        x = (width - w - 4.0).max(4.0);
    }
    if y + h + 3.0 > height {
        y = height - h - 3.0;
    }
    y = y.max(4.0);
    (x.round(), y.round())
}

use super::api_values::{TryIntoApi, TryIntoEngine};
use super::{Game, Screen};
use mod_api::{TooltipElement, TooltipImage};
use std::cell::Cell;
use sunrise_client::{
    block_entity_item, brewing,
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    items::ItemStack,
    random_source,
    stations::{self, AnvilResult, EnchantOffers},
    text::Text,
};
use sunrise_core::random::{LegacyRandom, RandomSource};

thread_local! {
    static IN_TOOLTIP_BUILD: Cell<bool> = const { Cell::new(false) };
}

struct TooltipBuildGuard;

impl Drop for TooltipBuildGuard {
    fn drop(&mut self) {
        IN_TOOLTIP_BUILD.set(false);
    }
}

/// The book above the enchanting table's offers: `EnchantmentScreen`'s
/// `time`, `flip`, `oFlip`, `flipT`, `flipA`, `open`, `oOpen` and `last`.
pub(super) struct EnchantBook {
    time: i32,
    flip: f32,
    o_flip: f32,
    flip_t: f32,
    flip_a: f32,
    open: f32,
    o_open: f32,
    last: Option<ItemStack>,
    random: LegacyRandom,
}
impl EnchantBook {
    fn new(random: LegacyRandom) -> Self {
        Self {
            time: 0,
            flip: 0.0,
            o_flip: 0.0,
            flip_t: 0.0,
            flip_a: 0.0,
            open: 0.0,
            o_open: 0.0,
            last: None,
            random,
        }
    }
    /// `EnchantmentScreen.tickBook`: a new item in the enchanting slot picks a
    /// new page to turn to; the book opens while any offer has a cost.
    pub(super) fn tick(&mut self, input: Option<&ItemStack>, costs: [i32; 3]) {
        if input != self.last.as_ref() {
            self.last = input.cloned();
            loop {
                let a = self.random.next_int_bound(4).unwrap_or(0);
                let b = self.random.next_int_bound(4).unwrap_or(0);
                self.flip_t += (a - b) as f32;
                if !(self.flip <= self.flip_t + 1.0 && self.flip >= self.flip_t - 1.0) {
                    break;
                }
            }
        }
        self.time += 1;
        self.o_flip = self.flip;
        self.o_open = self.open;
        let open = costs.iter().any(|&c| c != 0);
        self.open = (self.open + if open { 0.2 } else { -0.2 }).clamp(0.0, 1.0);
        let diff = ((self.flip_t - self.flip) * 0.4).clamp(-0.2, 0.2);
        self.flip_a += (diff - self.flip_a) * 0.9;
        self.flip += self.flip_a;
    }
    /// `EnchantmentScreen.extractBook`'s `open` and `flip` at the partial tick.
    pub(super) fn at(&self, partial: f32) -> (f32, f32) {
        (
            self.o_open + (self.open - self.o_open) * partial,
            self.o_flip + (self.flip - self.o_flip) * partial,
        )
    }
}

/// Slot ids of the open station: inputs from 600, the result at 650.
pub(super) const INPUT: usize = 600;
pub(super) const RESULT: usize = 650;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Enchanting,
    Anvil,
    Grindstone,
    Smithing,
    Stonecutter,
    /// Villager trading (game/items/trading.rs).
    Merchant,
    /// The beacon menu (game/blocks/beacon.rs).
    Beacon,
    /// The cartography table (game/blocks/cartography.rs).
    Cartography,
    /// The loom (game/blocks/loom.rs).
    Loom,
}
impl Kind {
    pub(super) fn from_block(name: &str) -> Option<Self> {
        Some(match name {
            "minecraft:enchanting_table" => Self::Enchanting,
            "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
                Self::Anvil
            }
            "minecraft:grindstone" => Self::Grindstone,
            "minecraft:smithing_table" => Self::Smithing,
            "minecraft:stonecutter" => Self::Stonecutter,
            "minecraft:beacon" => Self::Beacon,
            "minecraft:cartography_table" => Self::Cartography,
            "minecraft:loom" => Self::Loom,
            _ => return None,
        })
    }
    pub(super) fn inputs(self) -> usize {
        match self {
            Self::Smithing | Self::Loom => 3,
            Self::Stonecutter | Self::Beacon => 1,
            _ => 2,
        }
    }
    fn texture(self) -> &'static str {
        match self {
            Self::Enchanting => "gui/container/enchanting_table",
            Self::Anvil => "gui/container/anvil",
            Self::Grindstone => "gui/container/grindstone",
            Self::Smithing => "gui/container/smithing",
            Self::Stonecutter => "gui/container/stonecutter",
            Self::Merchant => "gui/container/villager",
            Self::Beacon => "gui/container/beacon",
            Self::Cartography => "gui/container/cartography_table",
            Self::Loom => "gui/container/loom",
        }
    }
    pub(super) fn title_key(self) -> &'static str {
        match self {
            Self::Enchanting => "container.enchant",
            Self::Anvil => "container.repair",
            Self::Grindstone => "container.grindstone_title",
            Self::Smithing => "container.upgrade",
            Self::Stonecutter => "container.stonecutter",
            Self::Merchant => "merchant.trades",
            Self::Beacon => "container.beacon",
            Self::Cartography => "container.cartography_table",
            Self::Loom => "container.loom",
        }
    }
    /// (titleLabelX, titleLabelY).
    fn title_pos(self) -> (f32, f32) {
        match self {
            Self::Anvil => (60.0, 6.0),
            Self::Smithing => (44.0, 15.0),
            Self::Stonecutter => (8.0, 5.0),
            Self::Cartography | Self::Loom => (8.0, 4.0),
            _ => (8.0, 6.0),
        }
    }
    fn input_pos(self, i: usize) -> (f32, f32) {
        let table: &[(f32, f32)] = match self {
            Self::Enchanting => &[(15.0, 47.0), (35.0, 47.0)],
            Self::Anvil => &[(27.0, 47.0), (76.0, 47.0)],
            Self::Grindstone => &[(49.0, 19.0), (49.0, 40.0)],
            Self::Smithing => &[(8.0, 48.0), (26.0, 48.0), (44.0, 48.0)],
            Self::Stonecutter => &[(20.0, 33.0)],
            Self::Merchant => &[(86.0, 37.0), (112.0, 37.0)],
            Self::Beacon => &[(109.0, 110.0)],
            Self::Cartography => &[(15.0, 15.0), (15.0, 52.0)],
            Self::Loom => &[(13.0, 26.0), (33.0, 26.0), (23.0, 45.0)],
        };
        table[i.min(table.len() - 1)]
    }
    /// Whether the menu has a result slot after its inputs.
    pub(super) fn has_result(self) -> bool {
        self.result_pos().is_some()
    }
    fn result_pos(self) -> Option<(f32, f32)> {
        match self {
            Self::Enchanting => None,
            Self::Anvil => Some((134.0, 47.0)),
            Self::Grindstone => Some((129.0, 34.0)),
            Self::Smithing => Some((98.0, 48.0)),
            Self::Stonecutter => Some((143.0, 33.0)),
            Self::Merchant => Some((170.0, 37.0)),
            Self::Beacon => None,
            Self::Cartography => Some((145.0, 39.0)),
            Self::Loom => Some((143.0, 57.0)),
        }
    }
}

/// `CyclingSlotBackground`: the icons an empty slot cycles through, one every
/// 30 ticks, the next fading in over 4 (`CyclingSlotBackground.tick` in
/// `advance`, `CyclingSlotBackground.extractRenderState` in `shown`).
#[derive(Default)]
struct CyclingSlot {
    icons: Vec<&'static str>,
    tick: u32,
    index: usize,
    /// The client tick it was last advanced to.
    at: Option<u64>,
}
impl CyclingSlot {
    const CHANGE_TICKS: u32 = 30;
    const FADE_TICKS: f32 = 4.0;
    /// `CyclingSlotBackground.tick(newIcons)` for each client tick since the last call.
    fn advance(&mut self, now: u64, icons: &[&'static str]) {
        let ticks = self.at.map_or(0, |at| now.saturating_sub(at).min(600));
        self.at = Some(now);
        if self.icons != icons {
            self.icons = icons.to_vec();
            self.index = 0;
        }
        for _ in 0..ticks {
            if !self.icons.is_empty() {
                self.tick += 1;
                if self.tick.is_multiple_of(Self::CHANGE_TICKS) {
                    self.index = (self.index + 1) % self.icons.len();
                }
            }
        }
    }
    /// What `CyclingSlotBackground.extractRenderState` draws for an empty slot: the icon before
    /// this one fading out while this one fades in, with their alphas.
    fn shown(&self, partial: f32) -> Vec<(&'static str, f32)> {
        if self.icons.is_empty() {
            return Vec::new();
        }
        let fading = self.icons.len() > 1 && self.tick >= Self::CHANGE_TICKS;
        let alpha = if fading {
            (((self.tick % Self::CHANGE_TICKS) as f32 + partial).min(Self::FADE_TICKS))
                / Self::FADE_TICKS
        } else {
            1.0
        };
        let mut shown = Vec::new();
        if alpha < 1.0 {
            let previous = (self.index + self.icons.len() - 1) % self.icons.len();
            shown.push((self.icons[previous], 1.0 - alpha));
        }
        shown.push((self.icons[self.index], alpha));
        shown
    }
}
/// `SmithingScreen`'s `EMPTY_SLOT_SMITHING_TEMPLATES`.
const EMPTY_TEMPLATE_ICONS: [&str; 2] = [
    "smithing_template_armor_trim",
    "smithing_template_netherite_upgrade",
];

pub(super) struct Station {
    pub(super) kind: Kind,
    pub(super) pos: [i32; 3],
    pub(super) inputs: Vec<Option<ItemStack>>,
    pub(super) result: Option<ItemStack>,
    pub(super) offers: EnchantOffers,
    bookcases: i32,
    pub(super) anvil: AnvilResult,
    /// Anvil rename box text (`AnvilMenu.itemName`).
    pub(super) name: String,
    /// The box itself (`AnvilScreen`'s `EditBox` of 50, unbordered).
    pub(super) rename: sunrise_client::ui::EditBox,
    /// Stonecutter: the menu's recipe list (sunrise_client::menu), its
    /// `selectedRecipeIndex` and the screen's first shown index.
    pub(super) stone: sunrise_client::menu::Stonecutter,
    pub(super) selected: Option<usize>,
    start: usize,
    /// Stonecutter: the screen's `scrollOffs` (0 to 1) and `scrolling`, the
    /// scroller held by the mouse.
    scroll: f32,
    scrolling: bool,
    /// Anvil: the input the rename box was last filled from.
    anvil_last: Option<ItemStack>,
    /// Anvil on a server: the name last sent (the client menu's `itemName`).
    pub(super) anvil_sent: Option<String>,
    pub(super) last_sound: u64,
    /// Villager trading state.
    pub(super) merchant: Option<Box<super::trading::MerchantState>>,
    /// Beacon menu: the selected primary and secondary power (EFFECTS index).
    pub(super) beacon: [Option<usize>; 2],
    /// A server's beacon menu: the pyramid's levels (its data slot 0).
    pub(super) beacon_levels: Option<i32>,
    /// The loom's patterns, selection and scroller (game/blocks/loom.rs).
    pub(super) loom: super::loom::LoomState,
    /// The smithing table's three `CyclingSlotBackground`s: the template, base and additions slots.
    cycling: [CyclingSlot; 3],
    /// The enchanting table's book (`EnchantmentScreen`'s flip and open state).
    pub(super) book: EnchantBook,
}
impl Station {
    pub(in crate::game) fn new(kind: Kind, pos: [i32; 3]) -> Self {
        Self {
            kind,
            pos,
            inputs: vec![None; kind.inputs()],
            result: None,
            offers: EnchantOffers::default(),
            bookcases: 0,
            anvil: AnvilResult::default(),
            name: String::new(),
            rename: anvil::rename_box(),
            stone: Default::default(),
            selected: None,
            start: 0,
            scroll: 0.0,
            scrolling: false,
            anvil_last: None,
            anvil_sent: None,
            last_sound: 0,
            merchant: None,
            beacon: [None; 2],
            beacon_levels: None,
            loom: Default::default(),
            cycling: Default::default(),
            book: EnchantBook::new(random_source::create()),
        }
    }
    pub(super) fn new_kind(kind: Kind, pos: [i32; 3]) -> Self {
        Self::new(kind, pos)
    }
    /// `StonecutterScreen.containerChanged`: a changed input puts the list back at the top.
    pub(super) fn stonecutter_changed(&mut self) {
        self.start = 0;
        self.scroll = 0.0;
    }
    /// Clear state tied to the previous `UpdateRecipes` table.
    pub(in crate::game) fn reset_stonecutter_generation(&mut self) {
        self.stone = Default::default();
        self.selected = None;
        self.result = None;
        self.scrolling = false;
        self.stonecutter_changed();
    }
    /// `StonecutterScreen.isScrollBarActive`.
    fn stonecutter_scrolls(&self) -> bool {
        self.inputs[0].is_some() && self.stone.recipes.len() > 12
    }
    /// Moves the stonecutter's scroller to `offs`, and its first shown
    /// recipe (`startIndex`) with it.
    fn stonecutter_scroll_to(&mut self, offs: f32) {
        self.scroll = offs.clamp(0.0, 1.0);
        self.start = stonecutter_start(self.stone.recipes.len(), self.scroll);
    }
}

/// `at` is on one of a list's `rows`, with no row further that way.
fn off_visible_rows(rows: &[[f32; 4]], at: (f32, f32), up: bool) -> bool {
    let inside =
        |r: &[f32; 4]| at.0 >= r[0] && at.0 < r[0] + r[2] && at.1 >= r[1] && at.1 < r[1] + r[3];
    let Some(on) = rows.iter().find(|r| inside(r)) else {
        return false;
    };
    !rows
        .iter()
        .any(|r| r[0] == on[0] && if up { r[1] < on[1] } else { r[1] > on[1] })
}

/// `StonecutterScreen.getOffscreenRows`: the recipe rows below the 4x3 grid.
fn stonecutter_offscreen_rows(recipes: usize) -> i32 {
    recipes.div_ceil(4) as i32 - 3
}
/// `startIndex` for a scroller position: the first recipe of the nearest row.
fn stonecutter_start(recipes: usize, offs: f32) -> usize {
    ((offs * stonecutter_offscreen_rows(recipes) as f32 + 0.5) as i32).max(0) as usize * 4
}
/// `StonecutterScreen.mouseDragged`: the scroller position under the cursor,
/// `y` relative to the GUI origin (the 54-px track starts at 14).
fn stonecutter_drag_offs(y: f32) -> f32 {
    ((y - 14.0 - 7.5) / (54.0 - 15.0)).clamp(0.0, 1.0)
}

/// The 108x19 enchantment button `i` (relative to the GUI origin).
fn enchant_button(i: usize) -> (f32, f32, f32, f32) {
    (60.0, 14.0 + 19.0 * i as f32, 108.0, 19.0)
}
fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
/// `EnchantmentNames` word list.
const WORDS: &[&str] = &[
    "the",
    "elder",
    "scrolls",
    "klaatu",
    "berata",
    "niktu",
    "xyzzy",
    "bless",
    "curse",
    "light",
    "darkness",
    "fire",
    "air",
    "earth",
    "water",
    "hot",
    "dry",
    "cold",
    "wet",
    "ignite",
    "snuff",
    "embiggen",
    "twist",
    "shorten",
    "stretch",
    "fiddle",
    "destroy",
    "imbue",
    "galvanize",
    "enchant",
    "free",
    "limited",
    "range",
    "of",
    "towards",
    "inside",
    "sphere",
    "cube",
    "self",
    "other",
    "ball",
    "mental",
    "physical",
    "grow",
    "shrink",
    "demon",
    "elemental",
    "spirit",
    "animal",
    "creature",
    "beast",
    "humanoid",
    "undead",
    "fresh",
    "stale",
    "phnglui",
    "mglwnafh",
    "cthulhu",
    "rlyeh",
    "wgahnagl",
    "fhtagn",
    "baguette",
];
const GREY: u32 = 0xFF40_4040;
fn argb(c: i32) -> u32 {
    let c = c as u32;
    rgba(
        (c >> 16 & 255) as u8,
        (c >> 8 & 255) as u8,
        (c & 255) as u8,
        (c >> 24 & 255) as u8,
    )
}

/// A text style's `0xRRGGBB` colour from a HUD colour (`rgba`).
fn rgb_of(rgba: u32) -> u32 {
    (rgba & 0xFF) << 16 | (rgba >> 8 & 0xFF) << 8 | (rgba >> 16 & 0xFF)
}
impl Game<'_> {
    /// `EnchantmentScreen.containerTick`'s `tickBook` for an open enchanting table.
    pub(super) fn tick_enchanting_book(&mut self) {
        if let Some(s) = self.station.as_mut().filter(|s| s.kind == Kind::Enchanting) {
            let costs = s.offers.costs;
            s.book.tick(s.inputs[0].as_ref(), costs);
        }
    }
    /// `Component.translatable`: the selected language (which `lang` layers
    /// over `en_us`), the caller's English text last.
    pub(super) fn translate_with_fallback(&self, key: &str, fallback: &str) -> String {
        sunrise_client::lang::get_or(key, fallback)
    }
    fn brewing_open(&self) -> bool {
        matches!(self.container_kind(), Some(BlockEntity::Brewing(_)))
    }
    /// Opens a workstation menu for the block; false when it is not one.
    pub(super) fn open_station(&mut self, pos: [i32; 3], name: &str) -> bool {
        let Some(kind) = Kind::from_block(name) else {
            return false;
        };
        if self.level.xp_seed == 0 {
            self.level.xp_seed = self.world.random.next_int();
        }
        self.station = Some(Station::new(kind, pos));
        self.station_changed();
        self.beacon_opened();
        // The block's interaction statistic.
        let stat = match name {
            "minecraft:crafting_table" => "interact_with_crafting_table",
            "minecraft:furnace" => "interact_with_furnace",
            "minecraft:blast_furnace" => "interact_with_blast_furnace",
            "minecraft:smoker" => "interact_with_smoker",
            "minecraft:brewing_stand" => "interact_with_brewingstand",
            "minecraft:beacon" => "interact_with_beacon",
            "minecraft:grindstone" => "interact_with_grindstone",
            "minecraft:smithing_table" => "interact_with_smithing_table",
            "minecraft:stonecutter" => "interact_with_stonecutter",
            "minecraft:loom" => "interact_with_loom",
            "minecraft:cartography_table" => "interact_with_cartography_table",
            n if n.ends_with("anvil") => "interact_with_anvil",
            _ => "",
        };
        if !stat.is_empty() {
            self.award_custom_stat(stat, 1);
        }
        true
    }
    /// `AbstractContainerMenu.clearContainer`: inputs go back to the player.
    pub(super) fn close_station(&mut self) {
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Merchant)
        {
            // MerchantMenu.removed: the carried stack and the payment go back.
            self.menu_session(|menu, env, _| {
                menu.merchant_close(env);
                Some(true)
            });
        }
        let Some(mut station) = self.station.take() else {
            return;
        };
        if self.remote.is_some() && station.kind != Kind::Merchant {
            // A server's workstation: its removed() gives the inputs back.
            return;
        }
        if let Some(m) = station.merchant.as_mut() {
            self.merchant_closed(m);
        }
        for stack in station.inputs.into_iter().flatten() {
            self.give_or_drop(stack);
        }
    }
    pub(super) fn station_slots(&self) -> Option<Vec<usize>> {
        let mut v: Vec<usize> = (0..36).collect();
        if self.brewing_open() {
            v.extend(300..305);
            return Some(v);
        }
        let s = self.station.as_ref()?;
        v.extend((0..s.inputs.len()).map(|i| INPUT + i));
        if s.kind.result_pos().is_some() {
            v.push(RESULT);
        }
        Some(v)
    }
    /// How many slots `station_slots` lists, counted without building it
    /// (`Game::slot_count` asks every frame): same branches, same order.
    pub(super) fn station_slot_count(&self) -> Option<usize> {
        if self.brewing_open() {
            return Some(36 + 5);
        }
        let s = self.station.as_ref()?;
        Some(36 + s.inputs.len() + usize::from(s.kind.result_pos().is_some()))
    }
    pub(super) fn station_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if self.brewing_open() && (300..305).contains(&i) {
            return Some(
                [
                    (56.0, 51.0),
                    (79.0, 58.0),
                    (102.0, 51.0),
                    (79.0, 17.0),
                    (17.0, 17.0),
                ][i - 300],
            );
        }
        let s = self.station.as_ref()?;
        if s.kind == Kind::Merchant && i < 36 {
            return Some(super::trading::inventory_slot(i));
        }
        if s.kind == Kind::Beacon {
            return super::beacon::slot_position(i);
        }
        match i {
            RESULT => s.kind.result_pos(),
            _ if i >= INPUT && i < INPUT + s.inputs.len() => Some(s.kind.input_pos(i - INPUT)),
            _ => None,
        }
    }
    pub(super) fn station_slot(&self, i: usize) -> &Option<ItemStack> {
        match self.station.as_ref() {
            Some(s) if i == RESULT => &s.result,
            Some(s) if i >= INPUT && i < INPUT + s.inputs.len() => &s.inputs[i - INPUT],
            _ => &self.scratch_slot,
        }
    }
    pub(super) fn station_slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        match self.station.as_mut() {
            Some(s) if i == RESULT => &mut s.result,
            Some(s) if i >= INPUT && i < INPUT + s.inputs.len() => &mut s.inputs[i - INPUT],
            _ => &mut self.scratch_slot,
        }
    }
    /// The anvil name the menu works with (`AnvilScreen.onNameChanged`).
    pub(super) fn anvil_item_name(&self, s: &Station) -> String {
        match &s.inputs[0] {
            Some(input)
                if input.custom_name().is_none()
                    && s.name
                        == sunrise_client::lang::returned_to_server(|| {
                            self.stack_hover_name_text(input).plain()
                        }) =>
            {
                String::new()
            }
            Some(_) => s.name.clone(),
            None => String::new(),
        }
    }
    /// Apply the client anvil hook after vanilla `createResult`.
    fn update_anvil_answer(&mut self, s: &mut Station, name: &str) {
        if !mod_api::__private::listened(mod_api::HookId::AnvilUpdate) {
            return;
        }
        let Some(left) = s.inputs.first().and_then(Option::as_ref) else {
            return;
        };
        let inputs = crate::menu::AnvilUpdateInputs {
            left,
            right: s.inputs.get(1).and_then(Option::as_ref),
            name: Some(name),
        };
        let (player, entity_id) = (self.own_uuid(), self.own_entity_id());
        self.api_anvil_update(player, entity_id, s.pos, inputs, &mut s.anvil);
    }
    /// `slotsChanged` / `createResult` for the open station.
    pub(super) fn station_changed(&mut self) {
        let Some(mut s) = self.station.take() else {
            return;
        };
        let creative = self.creative;
        let (enchantments, data) = (
            std::sync::Arc::clone(self.item_enchantments()),
            self.stations(),
        );
        let items = self.items();
        match s.kind {
            // A server's table: its data slots carry the offers, and its
            // client menu only clears them without an enchantable item.
            Kind::Enchanting if self.remote.is_some() => {
                let enchantable = s.inputs[0]
                    .as_ref()
                    .is_some_and(|i| stations::is_enchantable(&enchantments, i));
                if !enchantable {
                    s.offers = Default::default();
                }
            }
            Kind::Enchanting => {
                s.bookcases = self.count_bookshelves(s.pos);
                s.offers = stations::enchant_offers(
                    &enchantments,
                    items,
                    self.level.xp_seed,
                    s.bookcases,
                    s.inputs[0].as_ref(),
                );
            }
            Kind::Anvil => {
                let name = self.anvil_item_name(&s);
                // createResult's getHoverName().getString(), as vanilla resolves it.
                let hover = s.inputs[0]
                    .as_ref()
                    .map(|i| {
                        sunrise_client::lang::returned_to_server(|| {
                            self.stack_hover_name_text(i).plain()
                        })
                    })
                    .unwrap_or_default();
                s.anvil = stations::anvil_result(
                    &enchantments,
                    items,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                    Some(&name),
                    &hover,
                    creative,
                );
                // AnvilUpdateEvent on a server's client menu, as on the server,
                // whenever createResult ran with a left input.
                if s.inputs.first().is_some_and(Option::is_some) && self.remote.is_some() {
                    self.update_anvil_answer(&mut s, &name);
                }
                s.result = s.anvil.result.clone();
            }
            Kind::Grindstone => {
                s.result = stations::grindstone_result(
                    &self.loop_context,
                    &enchantments,
                    items,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                );
            }
            Kind::Smithing => {
                s.result = data.recipes.smithing_result(
                    &self.loop_context,
                    &data.enchantments,
                    s.inputs[0].as_ref(),
                    s.inputs[1].as_ref(),
                    s.inputs[2].as_ref(),
                );
            }
            // The menu engine keeps the merchant's result (Game::menu_session).
            Kind::Merchant => {}
            Kind::Beacon => {}
            // The menu engine keeps the cartography table's, the loom's and
            // the stonecutter's state (Game::menu_session).
            Kind::Cartography | Kind::Loom | Kind::Stonecutter => {}
        }
        self.station = Some(s);
    }
    pub(super) fn max_stack(&self, stack: &ItemStack) -> u32 {
        self.res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
    }
    /// `Slot.mayPlace` and `getMaxStackSize` for station slots.
    fn slot_rules(&self, slot: usize, stack: &ItemStack) -> (bool, u32) {
        if (super::mount_menu::SADDLE..crate::game::inventory_slots::MOD_MENU_SLOT_BASE)
            .contains(&slot)
        {
            return self.mount_slot_rules(slot, stack);
        }
        if let Some(BlockEntity::Crafter(c)) = self.container_kind()
            && (300..309).contains(&slot)
        {
            // CrafterSlot.mayPlace: not a disabled slot.
            return (!c.disabled[slot - 300], 64);
        }
        if (300..400).contains(&slot) && !self.brewing_open() && self.refuses(slot, stack) {
            // ShulkerBoxSlot.mayPlace.
            return (false, 64);
        }
        let Some(s) = self.station.as_ref() else {
            return (true, 64);
        };
        if slot < 36 {
            return (true, 64);
        }
        if slot == RESULT {
            return (false, 64);
        }
        (s.kind != Kind::Loom, 64)
    }
    /// `AbstractContainerMenu.doClick` PICKUP on a filtered slot.
    pub(super) fn click_filtered(&mut self, slot: usize, right: bool) {
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let (new_slot, new_carried) = match (carried, current) {
            (None, None) => (None, None),
            (None, Some(mut s)) => {
                let take = if right { s.count.div_ceil(2) } else { s.count };
                s.count -= take;
                let taken = ItemStack {
                    count: take,
                    ..s.clone()
                };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (Some(mut c), None) => {
                let (ok, cap) = self.slot_rules(slot, &c);
                if ok {
                    let n = if right { 1 } else { c.count }.min(cap.min(self.max_stack(&c)));
                    let placed = ItemStack {
                        count: n,
                        ..c.clone()
                    };
                    c.count -= n;
                    (Some(placed), (c.count > 0).then_some(c))
                } else {
                    (None, Some(c))
                }
            }
            (Some(mut c), Some(mut s)) => {
                let (ok, cap) = self.slot_rules(slot, &c);
                if ok && c.same_item_same_components(&s) {
                    let room = cap.min(self.max_stack(&s)).saturating_sub(s.count);
                    let n = if right { 1 } else { c.count }.min(room);
                    s.count += n;
                    c.count -= n;
                    (Some(s), (c.count > 0).then_some(c))
                } else if ok && c.count <= cap.min(self.max_stack(&c)) {
                    (Some(c), Some(s))
                } else if c.same_item_same_components(&s) {
                    let n = s.count.min(self.max_stack(&c).saturating_sub(c.count));
                    s.count -= n;
                    c.count += n;
                    ((s.count > 0).then_some(s), Some(c))
                } else {
                    (Some(s), Some(c))
                }
            }
        };
        *self.slot_mut(slot) = new_slot;
        self.carried = new_carried;
    }
    /// `moveItemStackTo` over `targets` (menu order), merging first, then empty slots.
    pub(super) fn move_to(
        &mut self,
        stack: &mut ItemStack,
        targets: &[usize],
        reverse: bool,
    ) -> bool {
        let order: Vec<usize> = if reverse {
            targets.iter().rev().copied().collect()
        } else {
            targets.to_vec()
        };
        let start = stack.count;
        if self.max_stack(stack) > 1 {
            for &t in &order {
                if stack.count == 0 {
                    break;
                }
                let (_, cap) = self.slot_rules(t, stack);
                let max = cap.min(self.max_stack(stack));
                if let Some(s) = self.slot_mut(t).as_mut()
                    && s.same_item_same_components(stack)
                {
                    let n = max.saturating_sub(s.count).min(stack.count);
                    s.count += n;
                    stack.count -= n;
                }
            }
        }
        for &t in &order {
            if stack.count == 0 {
                break;
            }
            let (ok, cap) = self.slot_rules(t, stack);
            if self.slot_mut(t).is_none() && (t < 36 || ok) {
                let n = cap.min(self.max_stack(stack)).min(stack.count);
                *self.slot_mut(t) = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                stack.count -= n;
            }
        }
        stack.count != start
    }
    /// After a live client anvil session (`anvil_menu_session`): copy the final anvil state
    /// from `menu` into `self.station` and sync the rename box without re-running `createResult`
    /// or `anvil_update` a second time.
    pub(super) fn anvil_session_end(&mut self, menu: &sunrise_client::menu::Menu) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.kind != Kind::Anvil {
            return;
        }
        s.anvil.cost = menu.data[0];
        s.anvil.repair_item_count_cost = menu.anvil.repair_count;
        s.anvil.only_renaming = menu.anvil.only_renaming;
        s.anvil.result = s.result.clone();
        if s.inputs[0] != s.anvil_last {
            s.anvil_last = s.inputs[0].clone();
            let name = s.inputs[0]
                .as_ref()
                .map(|i| {
                    sunrise_client::lang::returned_to_server(|| {
                        stations::hover_name(&self.pack.names, i)
                    })
                })
                .unwrap_or_default();
            anvil::input_changed(s, name);
        }
    }
    /// After an anvil or smithing click on the menu engine: the anvil's cost
    /// and state back into the station and the rename box following the
    /// input (`AnvilScreen.slotChanged`), then the rest of a taken result's
    /// `onTake` (the levels, the anvil's wear, the use sound) unless a
    /// server does it.
    pub(super) fn combiner_session_end(
        &mut self,
        menu: &sunrise_client::menu::Menu,
        anvil_created: bool,
        predicting: bool,
    ) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let (pos, kind) = (s.pos, s.kind);
        if kind == Kind::Anvil {
            if anvil_created {
                s.anvil.cost = menu.data[0];
                s.anvil.repair_item_count_cost = menu.anvil.repair_count;
                s.anvil.only_renaming = menu.anvil.only_renaming;
                s.anvil.result = s.result.clone();
                self.station_after_input();
            }
        } else {
            self.station_after_input();
        }
        if !menu.combined || predicting {
            return;
        }
        if kind == Kind::Smithing {
            self.level_event_at(1044, pos, 0);
            return;
        }
        self.give_levels(menu.level_change);
        if self.anvil_used(pos, self.creative) {
            self.close_station();
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
    }
    /// `AnvilMenu.onTake`'s level part: out of creative a 12% chance
    /// (`AnvilBlock.damage`) to wear the anvil a stage, or break a damaged one
    /// (level event 1029); otherwise level event 1030. True when it broke.
    pub(super) fn anvil_used(&mut self, pos: [i32; 3], creative: bool) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = self.res.facts().get(state).name.clone();
        let anvil = matches!(
            &*name,
            "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil"
        );
        if !creative && anvil && self.world.random.next_float() < 0.12 {
            match &*name {
                "minecraft:anvil" => {
                    let _ = self.set_state_named(pos, "minecraft:chipped_anvil", &["facing"]);
                    self.native_anvil_damaged(self.dim.id(), pos, &name, "minecraft:chipped_anvil");
                }
                "minecraft:chipped_anvil" => {
                    let _ = self.set_state_named(pos, "minecraft:damaged_anvil", &["facing"]);
                    self.native_anvil_damaged(self.dim.id(), pos, &name, "minecraft:damaged_anvil");
                }
                _ => {
                    let _ = self.world.set(pos[0], pos[1], pos[2], self.world.air);
                    self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
                    self.native_anvil_damaged(self.dim.id(), pos, &name, "minecraft:air");
                    self.level_event_at(1029, pos, 0);
                    return true;
                }
            }
        }
        self.level_event_at(1030, pos, 0);
        false
    }
    /// The grindstone result's `onTake` with a level (`amount`: the menu's
    /// `getExperienceAmount` sum): the orbs at the block's centre
    /// (`ExperienceOrb.award`), then level event 1042, the grindstone's sound
    /// for everyone around.
    pub(super) fn grindstone_taken(&mut self, pos: [i32; 3], amount: i32) {
        let xp = stations::grindstone_xp_roll(amount, &mut self.world.random);
        self.grindstone_award(pos, xp);
    }
    /// The rest of `onTake` with the experience `xp` decided: the orbs and
    /// level event 1042.
    pub(super) fn grindstone_award(&mut self, pos: [i32; 3], xp: u32) {
        if xp > 0 {
            self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), xp);
        }
        self.after_commit(super::transactions::After::GrindstoneEvent(pos));
    }
    fn play_at(&mut self, sound: &str, pos: [i32; 3], volume: f32, pitch: f32) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio.play(sound, Some(at), volume, pitch);
    }
    pub(super) fn give_or_drop(&mut self, stack: ItemStack) {
        if let Some(rest) = self.add_stack_to_inventory(stack)
            && let Some(eye) = self.eye_position()
        {
            // Inventory.placeItemBackInInventory: `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false, false);
        }
    }
    /// `Player.giveExperienceLevels`, and the level change reported to the mods.
    pub(super) fn give_levels(&mut self, levels: i32) {
        let old_level = self.experience.level;
        self.add_experience_levels(levels);
        self.native_player_level_changed(old_level);
    }
    /// `ServerPlayer.giveExperienceLevels`: the mods of a server player may change
    /// or cancel the levels, and every fifth level gained plays the level-up sound
    /// (at most every 100 ticks).
    pub(super) fn add_experience_levels(&mut self, mut levels: i32) {
        // ServerPlayer.giveExperienceLevels
        if levels == 0 {
            return;
        }
        if mod_api::__private::listened(mod_api::HookId::PlayerXpLevelChange)
            && let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            let mut cancel = false;
            self.native_player_xp_level_change(player, &name, &mut levels, &mut cancel);
            if cancel {
                return;
            }
        }
        self.experience.add_levels(levels);
        let level = self.experience.level;
        self.play_level_up(levels, level);
    }
    /// `Player.onEnchantmentPerformed`: the cost leaves the level directly, not
    /// through `giveExperienceLevels`, and the mods hear of the level change.
    pub(super) fn enchantment_performed(&mut self, cost: i32) {
        let old_level = self.experience.level;
        self.experience.add_levels(-cost);
        self.native_player_level_changed(old_level);
    }
    /// `Player.giveExperienceLevels`: all players near its feet hear the sound.
    pub(super) fn play_level_up(&mut self, amount: i32, level: i32) {
        if let Some(volume) = self.level_up_sound.give_levels(amount, level) {
            let at = self
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]);
            self.audio
                .play("minecraft:entity.player.levelup", at, volume, 1.0);
        }
    }
    /// Whether the result slot may be taken (`mayPickup`).
    fn may_take_result(&self) -> bool {
        let Some(s) = self.station.as_ref() else {
            return false;
        };
        match s.kind {
            Kind::Anvil => {
                (self.creative || self.experience.level >= s.anvil.cost) && s.anvil.cost > 0
            }
            _ => true,
        }
    }
    /// `EnchantmentMenu.clickMenuButton`.
    fn click_enchant(&mut self, button: usize) {
        let Some(s) = self.station.as_ref() else {
            return;
        };
        let cost = s.offers.costs[button];
        let lapis_needed = button as u32 + 1;
        let Some(item) = s.inputs[0].clone() else {
            return;
        };
        let lapis = s.inputs[1].as_ref().map_or(0, |l| l.count);
        if lapis < lapis_needed && !self.creative {
            return;
        }
        if cost <= 0
            || (self.experience.level < lapis_needed as i32 || self.experience.level < cost)
                && !self.creative
        {
            return;
        }
        if self.remote.is_some() {
            // EnchantmentScreen.mouseClicked: the client menu took it, the server enchants.
            self.remote_station_button(button as i32);
            return;
        }
        let Some(enchanted) = self.table_enchant(self.level.xp_seed, button, cost, &item) else {
            return;
        };
        let pos = s.pos;
        let enchanted_item = enchanted.clone();
        self.enchantment_performed(lapis_needed as i32);
        self.level.xp_seed = self.world.random.next_int();
        if let Some(s) = self.station.as_mut() {
            s.inputs[0] = Some(enchanted);
            if !self.creative
                && let Some(l) = s.inputs[1].as_mut()
            {
                l.count -= lapis_needed;
                if l.count == 0 {
                    s.inputs[1] = None;
                }
            }
        }
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.award_custom_stat("enchant_item", 1);
        self.trigger_enchanted_item(&enchanted_item, lapis_needed as i32);
        self.play_at("minecraft:block.enchantment_table.use", pos, 1.0, pitch);
        self.station_changed();
    }
    /// Mouse click while a station or the brewing stand is open. True when handled.
    /// A station's own buttons a controller can focus besides its slots
    /// (GUI origin relative): offers, patterns, effects, enchantments and
    /// stonecutter recipes.
    pub(super) fn station_targets(&self) -> Vec<[f32; 4]> {
        let Some(s) = self.station.as_ref() else {
            return Vec::new();
        };
        match s.kind {
            Kind::Merchant => self.merchant_targets(),
            Kind::Loom => self.loom_targets(),
            Kind::Beacon => self.beacon_targets(),
            Kind::Enchanting => (0..3)
                .map(|i| {
                    let (x, y, w, h) = enchant_button(i);
                    [x, y, w, h]
                })
                .collect(),
            Kind::Stonecutter if s.inputs[0].is_some() => (s.start
                ..(s.start + 12).min(s.stone.recipes.len()))
                .map(|index| {
                    let p = index - s.start;
                    [
                        52.0 + (p % 4) as f32 * 16.0,
                        14.0 + (p / 4) as f32 * 18.0,
                        16.0,
                        18.0,
                    ]
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    pub(super) fn station_click(&mut self, right: bool) -> bool {
        // A server's brewing stand: its slots are the server's.
        if self.station.is_none() && (!self.brewing_open() || self.remote.is_some()) {
            return false;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return true;
        };
        let batch = self.interface_batch(size);
        let (ox, oy) = self.inventory_origin(&batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        let shift = self.has_shift_down();
        if let Some(kind) = self.station.as_ref().map(|s| s.kind) {
            if kind == Kind::Merchant && !right && self.merchant_click(local) {
                return true;
            }
            if kind == Kind::Merchant && self.remote.is_some() {
                // A server's trading window: its slots are the server's.
                return false;
            }
            if kind == Kind::Loom && !right && self.loom_click(local) {
                return true;
            }
            if kind == Kind::Beacon && !right && self.beacon_click(local) {
                return true;
            }
            if kind == Kind::Enchanting && !right {
                for i in 0..3 {
                    let (x, y, w, h) = enchant_button(i);
                    if inside(local, x, y, w, h) {
                        self.click_enchant(i);
                        return true;
                    }
                }
            }
            if kind == Kind::Stonecutter && self.stonecutter_click(local) {
                return true;
            }
            if kind == Kind::Anvil && !right && self.anvil_name_click(ox, oy) {
                return true;
            }
        }
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return true;
        };
        // Every station's clicks (and the brewing stand's) run on the
        // verified menu engine.
        let input = if shift {
            sunrise_client::menu::Input::QuickMove
        } else {
            sunrise_client::menu::Input::Pickup
        };
        self.click_open_inventory(Some(slot), i32::from(right), input);
        true
    }
    /// Recompute after slot changes; the anvil box follows its input item.
    fn station_after_input(&mut self) {
        let Some(s) = self.station.as_ref() else {
            return;
        };
        if s.kind == Kind::Anvil && s.inputs[0] != s.anvil_last {
            // AnvilScreen.slotChanged: the box follows its input item, named as vanilla's
            // client names it (the box's text goes back to a server).
            let input = s.inputs[0].clone();
            let name = input
                .as_ref()
                .map(|i| {
                    sunrise_client::lang::returned_to_server(|| {
                        self.stack_hover_name_text(i).plain()
                    })
                })
                .unwrap_or_default();
            if let Some(s) = self.station.as_mut() {
                s.anvil_last = input;
                anvil::input_changed(s, name);
            }
        }
        self.station_changed();
    }
    /// `StonecutterScreen.mouseClicked` on its recipe grid: `clickMenuButton`
    /// with the cell's index (an empty cell's too), and the select sound when
    /// the menu took it.
    /// Beside the grid, a press on the scroller's track holds it (`scrolling`).
    fn stonecutter_click(&mut self, local: (f32, f32)) -> bool {
        let Some(s) = self.station.as_mut() else {
            return false;
        };
        s.scrolling = false;
        if s.inputs[0].is_some() && !s.stone.recipes.is_empty() {
            let start = s.start;
            if self.stonecutter_recipe_click(local, start) {
                return true;
            }
        }
        if inside(local, 119.0, 9.0, 12.0, 54.0)
            && let Some(s) = self.station.as_mut()
        {
            s.scrolling = true;
            return true;
        }
        false
    }
    fn stonecutter_recipe_click(&mut self, local: (f32, f32), start: usize) -> bool {
        for index in start..start + 12 {
            let p = index - start;
            let (x, y) = (52.0 + (p % 4) as f32 * 16.0, 14.0 + (p / 4) as f32 * 18.0);
            if !inside(local, x, y, 16.0, 18.0) {
                continue;
            }
            let mut taken = false;
            let flag = &mut taken;
            self.menu_session(|menu, env, _| {
                *flag = menu.button(env, index as i32);
                Some(*flag)
            });
            if taken {
                self.audio
                    .play("minecraft:ui.stonecutter.select_recipe", None, 1.0, 1.0);
                self.remote_station_button(index as i32);
                return true;
            }
        }
        false
    }
    /// A D-pad step off the visible rows of a station's list, from the row
    /// under the cursor: the list scrolls a row, if it can, and the cursor
    /// stays. False leaves the step to go on to whatever is that way.
    pub(super) fn station_list_step(&mut self, up: bool) -> bool {
        let Some(batch) = self.gui_batch() else {
            return false;
        };
        let (ox, oy) = self.inventory_origin(&batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        if !off_visible_rows(&self.station_targets(), local, up) {
            return false;
        }
        let shown = |g: &Self| {
            g.station.as_ref().map(|s| {
                let merchant = s.merchant.as_ref().map(|m| m.scroll);
                (s.start, s.loom.start_row, merchant)
            })
        };
        let before = shown(self);
        self.station_scroll(if up { 1.0 } else { -1.0 });
        shown(self) != before
    }
    /// `StonecutterScreen.mouseScrolled`.
    pub(super) fn station_scroll(&mut self, d: f32) {
        self.merchant_scroll(d);
        self.loom_scroll(d);
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.kind != Kind::Stonecutter || !s.stonecutter_scrolls() {
            return;
        }
        let rows = stonecutter_offscreen_rows(s.stone.recipes.len());
        s.stonecutter_scroll_to(s.scroll - d / rows as f32);
    }
    /// The mouse moved with a button held on a station: the loom's,
    /// stonecutter's and merchant's scrollers follow it.
    pub(super) fn station_drag(&mut self) {
        self.loom_drag();
        self.merchant_drag();
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let (_, oy) = self.inventory_origin(&self.interface_batch(size));
        let y = self.cursor.1 - oy;
        // `StonecutterScreen.mouseDragged`.
        if let Some(s) = self.station.as_mut()
            && s.kind == Kind::Stonecutter
            && s.scrolling
            && s.stonecutter_scrolls()
        {
            s.stonecutter_scroll_to(stonecutter_drag_offs(y));
        }
    }
    /// A mouse button let go: no scroller is held any more.
    pub(super) fn station_release(&mut self) -> bool {
        let mut handled = self.loom_release();
        if let Some(s) = self.station.as_mut() {
            handled |= std::mem::take(&mut s.scrolling);
            if let Some(m) = s.merchant.as_mut() {
                handled |= std::mem::take(&mut m.dragging);
            }
        }
        handled
    }
    /// Whether keyboard text goes to the anvil rename box.
    pub(super) fn station_typing(&self) -> bool {
        self.screen == Screen::Inventory
            && self
                .station
                .as_ref()
                .is_some_and(|s| s.kind == Kind::Anvil && s.inputs[0].is_some())
    }
    pub(super) fn station_text(&mut self, text: Option<&str>, backspace: bool) {
        // The recipe book's search box takes scripted typing too.
        if self.recipe_search_focused() {
            self.recipe_text(text, backspace);
            return;
        }
        let before = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil)
            .map(|s| self.anvil_item_name(s));
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if backspace {
            s.rename.delete_chars(-1);
        } else if let Some(t) = text {
            s.rename.insert_text(t);
        }
        s.name.clone_from(&s.rename.value);
        let after = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil)
            .map(|s| self.anvil_item_name(s));
        if before.is_none() || after != before {
            self.station_changed();
        }
        self.remote_anvil_rename();
    }
    /// `AnvilScreen.keyPressed`: the rename box handles the key.
    pub(super) fn station_key(&mut self, key: winit::keyboard::KeyCode) -> bool {
        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
        let before = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil)
            .map(|s| self.anvil_item_name(s));
        let Some(s) = self.station.as_mut() else {
            return false;
        };
        if !s.rename.key(key, ctrl, shift) {
            return false;
        }
        s.name.clone_from(&s.rename.value);
        let after = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil)
            .map(|s| self.anvil_item_name(s));
        if before.is_none() || after != before {
            self.station_changed();
        }
        self.remote_anvil_rename();
        true
    }
    /// Ctrl+A/C/V/X in the anvil's rename box (an `EditBox` of 50) or the
    /// recipe book's search box.
    pub(super) fn station_shortcut(&mut self, shortcut: sunrise_client::clipboard::EditShortcut) {
        if self.recipe_search_focused() {
            self.recipe_shortcut(shortcut);
            return;
        }
        let before = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil)
            .map(|s| self.anvil_item_name(s));
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.rename.shortcut(shortcut, &mut self.clipboard) {
            s.name.clone_from(&s.rename.value);
            let after = self
                .station
                .as_ref()
                .filter(|s| s.kind == Kind::Anvil)
                .map(|s| self.anvil_item_name(s));
            if before.is_none() || after != before {
                self.station_changed();
            }
            self.remote_anvil_rename();
        }
    }
    /// A click in the anvil's name box (`EditBox.onClick`).
    fn anvil_name_click(&mut self, ox: f32, oy: f32) -> bool {
        let open = self.station.as_ref().is_some_and(|s| s.kind == Kind::Anvil);
        if !open {
            return false;
        }
        let (x, y, w, h) = (ox + 62.0, oy + 24.0, 103.0, 12.0);
        let (mx, my) = self.cursor;
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }
        let shift = self.has_shift_down();
        let Some(s) = self.station.as_mut() else {
            return false;
        };
        s.rename.x = x;
        s.rename.y = y;
        s.rename.width = w;
        s.rename.click(&self.gui.atlas, mx, shift, false);
        true
    }
    /// AnvilScreen's name box, drawn after the station picture.
    fn draw_anvil_name(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let edit = &mut s.rename;
        edit.x = ox + 62.0;
        edit.y = oy + 24.0;
        edit.width = 103.0;
        edit.height = 12.0;
        edit.bordered = false;
        edit.focused = true;
        edit.text_color = u32::MAX;
        edit.max_length = anvil::NAME_CHARS;
        edit.draw(batch, &self.gui.atlas, None);
    }

    // ---------------------------------------------------------------- drawing

    fn part(
        batch: &mut HudBatch,
        atlas: &sunrise_client::hud::HudAtlas,
        name: &str,
        src: (f32, f32),
        size: (f32, f32),
        dst: (f32, f32),
    ) {
        if let Some(mut r) = atlas.gui_rect(name).copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u0 += src.0 * du;
            r.v0 += src.1 * dv;
            r.u1 = r.u0 + size.0 * du;
            r.v1 = r.v0 + size.1 * dv;
            batch.rect_uv(dst.0, dst.1, size.0, size.1, r, u32::MAX);
        }
    }
    /// Text in the Standard Galactic alphabet (`minecraft:alt`), cut to `max` pixels.
    fn galactic(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32, max: f32, color: u32) {
        let atlas = &self.gui.atlas;
        let Some(sheet) = atlas.sprites.get("font/ascii_sga").copied() else {
            batch.text(atlas, text, x, y, color, false);
            return;
        };
        let cell = sheet.w / 16;
        let (px, py) = (
            (sheet.u0 * atlas.width as f32) as u32,
            (sheet.v0 * atlas.height as f32) as u32,
        );
        let mut cursor = x;
        for ch in text.chars() {
            let advance = if ch == ' ' {
                4.0
            } else {
                let code = ch as u32;
                if !(0x41..=0x5A).contains(&code) && !(0x61..=0x7A).contains(&code) {
                    continue;
                }
                let (col, row) = (code % 16, code / 16);
                let (cx, cy) = (px + col * cell, py + row * cell);
                let used = u32::from(atlas.galactic_widths[code as usize]);
                let advance = (used as f32 * 8.0 / cell as f32 + 0.5).floor() + 1.0;
                if cursor + advance - x > max {
                    break;
                }
                let r = sunrise_client::hud::Rect {
                    u0: cx as f32 / atlas.width as f32,
                    v0: cy as f32 / atlas.height as f32,
                    u1: (cx + cell) as f32 / atlas.width as f32,
                    v1: (cy + cell) as f32 / atlas.height as f32,
                    w: cell,
                    h: cell,
                };
                batch.rect_uv(cursor, y, 8.0, 8.0, r, color);
                advance
            };
            if cursor + advance - x > max {
                break;
            }
            cursor += advance;
        }
    }
    fn galactic_deferred(
        &self,
        batch: &mut HudBatch,
        text: &str,
        x: f32,
        y: f32,
        max: f32,
        color: u32,
    ) {
        self.galactic(batch, text, x, y, max, color);
    }
    fn slot_position_any(&self, i: usize) -> Option<(f32, f32)> {
        self.station_slot_position(i)
            .or_else(|| (i < 36).then(|| self.slot_position(i)))
    }

    // ---------------------------------------------------------------- tooltips

    /// `ItemStack.getTooltipLines` subset: name in rarity colour, lore,
    /// potion effects, enchantments (gray, curses red), stored enchantments, trims.
    pub(super) fn stack_tooltip(
        &self,
        stack: &ItemStack,
    ) -> Vec<(sunrise_client::text::Text, u32)> {
        self.stack_tooltip_lines(stack, false, false)
    }
    /// `ItemStack.getRarity().color()` in text's RGB format.
    pub(in crate::game) fn stack_rarity_color(&self, stack: &ItemStack) -> u32 {
        use sunrise_client::text;
        let props = self.stations().enchantments.props(&stack.item);
        match sunrise_client::item_tooltip::rarity(
            stack,
            props.map_or("common", |p| p.rarity.as_str()),
        ) {
            "uncommon" => text::YELLOW,
            "rare" => text::AQUA,
            "epic" => text::LIGHT_PURPLE,
            _ => text::WHITE,
        }
    }
    /// The same rarity colour in the GUI batch's packed RGBA format.
    fn rarity_color(&self, stack: &ItemStack) -> u32 {
        let c = self.stack_rarity_color(stack);
        rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
    }
    /// The resolved `ItemStack.getHoverName` rendered as plain text.
    pub(in crate::game) fn stack_hover_name(&self, stack: &ItemStack) -> String {
        self.stack_hover_name_text(stack).plain()
    }
    /// `ItemStack.getTooltipLines`. A `nested` stack is a crossbow's
    /// projectile: `ChargedProjectiles.addProjectileTooltip` takes its details
    /// without the name, the player and the advanced flag. `creative` is
    /// `TooltipFlag.isCreative`: a creative tab's own slot.
    fn stack_tooltip_lines(
        &self,
        stack: &ItemStack,
        nested: bool,
        creative: bool,
    ) -> Vec<(sunrise_client::text::Text, u32)> {
        self.tooltip_lines(stack, nested, creative, self.options_advanced_tooltips())
    }

    #[allow(clippy::needless_question_mark)] // TODO(#2273)
    pub(in crate::game) fn item_resource_tooltip(
        &self,
        stack: &ItemStack,
        advanced: bool,
        creative: bool,
    ) -> Option<Vec<mod_api::TooltipElement>> {
        self.items().get(&stack.item)?;
        Some(
            self.tooltip_lines(stack, false, creative, advanced)
                .into_iter()
                .map(|(text, color)| {
                    text.try_into_api()
                        .map(|text| mod_api::TooltipElement::Text(text, color))
                })
                .collect::<Result<Vec<_>, _>>()
                .ok()?,
        )
    }

    fn tooltip_lines(
        &self,
        stack: &ItemStack,
        nested: bool,
        creative: bool,
        advanced: bool,
    ) -> Vec<(sunrise_client::text::Text, u32)> {
        // A listener, part or appender may ask for another tooltip on the same client thread.
        let guard = if IN_TOOLTIP_BUILD.replace(true) {
            None
        } else {
            Some(TooltipBuildGuard)
        };
        self.build_stack_tooltip_lines(stack, nested, creative, advanced, guard.is_some())
    }

    fn build_stack_tooltip_lines(
        &self,
        stack: &ItemStack,
        nested: bool,
        creative: bool,
        advanced: bool,
        mod_callbacks: bool,
    ) -> Vec<(sunrise_client::text::Text, u32)> {
        use sunrise_client::item_tooltip::{
            self, EnchantmentComponent, PaintingTooltip, TooltipHolders,
        };
        let holders = TooltipHolders::Definitions(self.active_enchantment_definitions());
        let color = self.rarity_color(stack);
        let advanced = !nested && advanced;
        let display = sunrise_client::item_tooltip::tooltip_display(stack);
        let mut lines = Vec::new();
        // `getTooltipLines`' own branch: a tooltip the stack doesn't hide.
        let shown = !nested && display.name_visible(creative);
        if shown {
            let name = self.styled_stack_name(stack);
            lines.push((name, color));
        }
        let translate = |k: &str| self.pack.names.display_get(k);
        let push = |lines: &mut Vec<(sunrise_client::text::Text, u32)>, s: String, c: u32| {
            lines.push((sunrise_client::text::Text::legacy(&s), c));
        };
        let gray = rgba(170, 170, 170, 255);
        let item_info = self.items().get(&stack.item);
        // SmithingTemplateItem.appendHoverText: `Item.appendHoverText` comes
        // before the components.
        for (text, c) in sunrise_client::item_tooltip::smithing_template_lines(stack, &translate) {
            push(&mut lines, text, c);
        }
        // DiscFragmentItem.appendHoverText.
        if let Some(text) = sunrise_client::item_tooltip::disc_fragment_line(stack, &translate) {
            push(&mut lines, text, gray);
        }
        lines.extend(
            item_tooltip::painting_lines(
                stack,
                &display,
                creative,
                |id| {
                    let v = self.painting_variants.get(id)?;
                    Some(PaintingTooltip {
                        title: v.title.as_ref(),
                        author: v.author.as_ref(),
                        size: [v.width, v.height],
                    })
                },
                &sunrise_client::text::Text::translate,
            )
            .into_iter()
            .map(|text| (text, u32::MAX)),
        );
        let context = mod_api::TooltipAppendContext {
            player: self.own_uuid(),
            player_entity_id: self.own_entity_id(),
            tick_rate: if self.native_client_level().is_some() {
                self.clock.rate()
            } else {
                20.0
            },
            advanced,
            creative,
            nested,
        };
        // A mod item's own `Item.appendHoverText`, its parts' lines, before the components; a
        // tooltip the stack hides, and a crossbow's projectile, stay without them.
        if mod_callbacks && !nested && !lines.is_empty() {
            let mut own = Vec::new();
            self.native_tooltip(stack, &context, &mut own);
            lines.extend(own);
        }
        // The component details, marked for the placements of the registered appenders.
        let appenders = if mod_callbacks {
            mod_api::__private::tooltip_appenders()
        } else {
            &[]
        };
        let mut details = sunrise_client::item_tooltip::DetailMarks::new(!appenders.is_empty());
        details.start("minecraft:tropical_fish/pattern", lines.len());
        // TropicalFish.Pattern.addToTooltip.
        for text in sunrise_client::item_tooltip::tropical_fish_lines(stack, &display) {
            lines.push((text, gray));
        }
        details.start("minecraft:instrument", lines.len());
        // InstrumentComponent.addToTooltip.
        if let Some(text) = item_tooltip::instrument_line(
            stack,
            &display,
            Some(|id: &str| {
                let key = format!("instrument.{}", id.replacen(':', ".", 1));
                Some(translate(&key).unwrap_or(key))
            }),
        ) {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:map_id", lines.len());
        // MapId.addToTooltip: the saved data's lock and scale, when it is known.
        let map = stack
            .data()
            .and_then(|d| d.map_id)
            .and_then(|id| self.maps.data.get(&id))
            .map(|m| (m.locked, m.scale));
        for text in
            sunrise_client::item_tooltip::map_id_lines(stack, &display, advanced, map, &translate)
        {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:bees", lines.len());
        // Bees.addToTooltip.
        if let Some(text) = sunrise_client::item_tooltip::bees_line(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:container", lines.len());
        // ItemContainerContents.addToTooltip.
        if display.shows("minecraft:container") {
            for text in sunrise_client::item_tooltip::container_lines(
                stack,
                &|s| self.stack_hover_name(s),
                &translate,
            ) {
                push(&mut lines, text, u32::MAX);
            }
        }
        details.start("minecraft:banner_patterns", lines.len());
        // BannerPatternLayers.addToTooltip: up to six layer names.
        if display.shows("minecraft:banner_patterns") {
            for layer in sunrise_client::banner::tooltip_layers(stack).iter().take(6) {
                let key = self.stations().banners.layer_key(layer);
                push(
                    &mut lines,
                    self.translate_with_fallback(&key, &key),
                    rgba(170, 170, 170, 255),
                );
            }
        }
        details.start("minecraft:written_book_content", lines.len());
        // WrittenBookContent.addToTooltip.
        for text in sunrise_client::item_tooltip::written_book_lines(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:charged_projectiles", lines.len());
        // ChargedProjectiles.addToTooltip: each run of equal projectiles, then
        // the projectile's own details indented under it.
        if display.shows("minecraft:charged_projectiles") {
            let charged = stack.data().map_or(&[][..], |d| &d.charged[..]);
            for run in charged.chunk_by(|a, b| a.same_item_same_components(b)) {
                let projectile = &run[0];
                let line = if run.len() == 1 {
                    sunrise_client::text::format_translation(
                        &self.translate_with_fallback(
                            "item.minecraft.crossbow.projectile.single",
                            "Projectile: %s",
                        ),
                        &[self.stack_display_name(projectile)],
                    )
                } else {
                    sunrise_client::text::format_translation(
                        &self.translate_with_fallback(
                            "item.minecraft.crossbow.projectile.multiple",
                            "Projectile: %s x %s",
                        ),
                        &[
                            sunrise_client::text::Text::literal(run.len().to_string()),
                            self.stack_hover_name_text(projectile),
                        ],
                    )
                };
                lines.push((line, u32::MAX));
                for (text, c) in
                    self.build_stack_tooltip_lines(projectile, true, false, false, mod_callbacks)
                {
                    // `literal("  ").append(line).withStyle(GRAY)`: the line's
                    // own colours win over the gray.
                    let rgb = if c == u32::MAX {
                        sunrise_client::text::GRAY
                    } else {
                        rgb_of(c)
                    };
                    let indented = sunrise_client::text::Text::literal("  ")
                        .append(text)
                        .with_color(rgb);
                    lines.push((indented, u32::MAX));
                }
            }
        }
        details.start("minecraft:fireworks", lines.len());
        // Fireworks / FireworkExplosion.addToTooltip.
        for text in sunrise_client::item_tooltip::firework_lines(stack, &translate) {
            push(&mut lines, text, rgba(170, 170, 170, 255));
        }
        details.start("minecraft:potion_contents", lines.len());
        if display.shows("minecraft:potion_contents")
            && (stack.item.ends_with("potion") || &*stack.item == "minecraft:tipped_arrow")
        {
            let scale = match &*stack.item {
                "minecraft:lingering_potion" => 0.25,
                "minecraft:tipped_arrow" => 0.125,
                _ => 1.0,
            };
            // PotionContents.getPotionDescription: the effect names are
            // translatable, read in the selected language like every other line.
            for (text, cat) in brewing::tooltip(stack.potion(), scale, &translate) {
                let c = match cat {
                    brewing::Category::Beneficial => rgba(85, 85, 255, 255),
                    brewing::Category::Harmful => rgba(255, 85, 85, 255),
                    brewing::Category::Neutral => rgba(170, 170, 170, 255),
                };
                push(&mut lines, text, c);
            }
            // The attribute modifiers of the effects (`When Applied:`).
            for (text, c) in sunrise_client::item_tooltip::potion_modifier_lines(stack, &translate)
            {
                push(&mut lines, text, c);
            }
        }
        details.start("minecraft:jukebox_playable", lines.len());
        // JukeboxPlayable.addToTooltip.
        if let Some(text) =
            sunrise_client::item_tooltip::jukebox_line(stack, item_info, &display, &translate)
        {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:trim", lines.len());
        if display.shows("minecraft:trim")
            && let Some((material, pattern)) = stack
                .data()
                .and_then(|d| d.trim.as_ref())
                .and_then(sunrise_client::item_data::trim::named)
        {
            push(
                &mut lines,
                translate("item.minecraft.smithing_template.upgrade")
                    .unwrap_or_else(|| "Upgrade:".to_owned()),
                rgba(170, 170, 170, 255),
            );
            let short = |s: &str| s.trim_start_matches("minecraft:").to_owned();
            let p = translate(&format!("trim_pattern.minecraft.{}", short(pattern)))
                .unwrap_or_else(|| short(pattern));
            let m = translate(&format!("trim_material.minecraft.{}", short(material)))
                .unwrap_or_else(|| short(material));
            push(&mut lines, format!(" {p}"), rgba(170, 170, 170, 255));
            push(&mut lines, format!(" {m}"), rgba(170, 170, 170, 255));
        }
        for component in [EnchantmentComponent::Stored, EnchantmentComponent::Applied] {
            details.start(component.id(), lines.len());
            for (text, curse) in
                item_tooltip::enchantment_lines(stack, &display, component, holders, &translate)
            {
                push(
                    &mut lines,
                    text,
                    if curse { rgba(255, 85, 85, 255) } else { gray },
                );
            }
        }
        details.start("minecraft:dyed_color", lines.len());
        // DyedItemColor.addToTooltip.
        if let Some((text, italic)) =
            sunrise_client::item_tooltip::dyed_line(stack, &display, advanced, &translate)
        {
            let text = sunrise_client::text::Text::legacy(&text);
            lines.push((if italic { text.italic() } else { text }, gray));
        }
        details.start("minecraft:lore", lines.len());
        // ItemLore.addToTooltip: after the enchantments, before the attributes
        // (ItemStack.addDetailsToTooltip).
        if display.shows("minecraft:lore") {
            for text in sunrise_client::item_tooltip::lore_texts(stack) {
                lines.push((text, u32::MAX));
            }
        }
        details.start("minecraft:attribute_modifiers", lines.len());
        let attributes_shown = display.shows("minecraft:attribute_modifiers");
        let (modifier_lines, added) = self.stack_attribute_tooltip(
            stack,
            attributes_shown,
            nested,
            advanced,
            creative,
            mod_callbacks && display.name_visible(creative),
        );
        for (text, c) in modifier_lines {
            push(&mut lines, text, c);
        }
        lines.extend(added);
        details.start("minecraft:unbreakable", lines.len());
        // ItemStack.UNBREAKABLE_TOOLTIP.
        if let Some(text) =
            sunrise_client::item_tooltip::unbreakable_line(stack, &display, &translate)
        {
            push(&mut lines, text, rgba(85, 85, 255, 255));
        }
        details.start("minecraft:ominous_bottle_amplifier", lines.len());
        // OminousBottleAmplifier.addToTooltip.
        if let Some((text, c)) =
            sunrise_client::item_tooltip::ominous_bottle_line(stack, &display, &translate)
        {
            push(&mut lines, text, c);
        }
        details.start("minecraft:suspicious_stew_effects", lines.len());
        // SuspiciousStewEffects.addToTooltip: the creative flag, which vanilla
        // sets for the creative screen's own slots, as `name_visible` above.
        for (text, c) in
            sunrise_client::item_tooltip::stew_lines(stack, creative, &display, &translate)
        {
            push(&mut lines, text, c);
        }
        details.start("minecraft:block_state", lines.len());
        // BlockItemStateProperties.addToTooltip.
        if let Some(text) = sunrise_client::item_tooltip::honey_line(stack, &display, &translate) {
            push(&mut lines, text, gray);
        }
        details.start("minecraft:entity_data", lines.len());
        // TypedEntityData.addToTooltip: a type disallowed in Peaceful.
        let entity_data = block_entity_item::item_entity_data(stack, self.items());
        if let Some(text) = sunrise_client::item_tooltip::spawn_egg_peaceful_line(
            entity_data.as_ref().map(|data| data.kind.as_str()),
            self.ui.difficulty == 0,
            &display,
            &|id| {
                self.res
                    .mob_types()
                    .get(id)
                    .is_none_or(|t| t.allowed_in_peaceful())
            },
            &translate,
        ) {
            push(&mut lines, text, rgba(255, 85, 85, 255));
        }
        details.start("minecraft:block_entity_data", lines.len());
        // Spawner.appendHoverText: what a spawner or trial spawner item spawns.
        if display.shows("minecraft:block_entity_data")
            && matches!(
                &*stack.item,
                "minecraft:spawner" | "minecraft:trial_spawner"
            )
        {
            let gray = rgba(170, 170, 170, 255);
            match block_entity_item::spawner_entity(stack) {
                Some(id) => {
                    let key = format!("entity.{}", id.replacen(':', ".", 1));
                    push(&mut lines, self.translate_with_fallback(&key, &id), gray);
                }
                None => {
                    push(&mut lines, String::new(), u32::MAX);
                    let desc1 = self.translate_with_fallback(
                        "block.minecraft.spawner.desc1",
                        "Interact with Spawn Egg:",
                    );
                    push(&mut lines, desc1, gray);
                    let desc2 = self
                        .translate_with_fallback("block.minecraft.spawner.desc2", "Sets Mob Type");
                    push(&mut lines, format!(" {desc2}"), rgba(85, 85, 255, 255));
                }
            }
        }
        // AdventureModePredicate.addToTooltip under its header, for
        // can_break and then can_place_on. A block tag's blocks are listed by
        // id, as this side keeps tags unordered.
        for (text, c) in sunrise_client::item_tooltip::adventure_lines(
            stack,
            &display,
            &|tag| {
                let mut blocks: Vec<String> = self
                    .items()
                    .block_tags
                    .get(tag)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
                blocks.sort();
                blocks
            },
            &translate,
        ) {
            push(&mut lines, text, c);
        }
        details.end(lines.len());
        if display.shows("minecraft:damage")
            && let Some(info) = item_info
            && info.max_damage > 0
            && stack.damage > 0
            && advanced
        {
            // `item.durability`, in the selected language.
            lines.push((
                sunrise_client::text::Text::translate(
                    "item.durability",
                    &[
                        sunrise_client::text::Text::literal(
                            info.max_damage.saturating_sub(stack.damage).to_string(),
                        ),
                        sunrise_client::text::Text::literal(info.max_damage.to_string()),
                    ],
                ),
                u32::MAX,
            ));
        }
        // ItemStack.addDetailsToTooltipTail: with advanced tooltips the item id
        // and how many components the stack has.
        if advanced {
            let dark_gray = rgba(85, 85, 85, 255);
            push(&mut lines, stack.item.to_string(), dark_gray);
            let count = sunrise_client::item_tooltip::component_count(stack, item_info);
            if count > 0 {
                let text = self.translate_with_fallback("item.components", "%s component(s)");
                push(
                    &mut lines,
                    text.replacen("%s", &count.to_string(), 1),
                    dark_gray,
                );
            }
        }
        // ItemStack.OP_NBT_WARNING: `BlockItem` and `SpawnEggItem`
        // `.shouldPrintOpWarning` for a player with game master permission;
        // a nested projectile has no player. The first line is bold.
        if !nested
            && self.ui.registries.permission >= 2
            && (item_info.is_some_and(|i| i.block.is_some())
                && block_entity_item::op_warning(stack)
                || stack.item.ends_with("_spawn_egg")
                    && entity_data.is_some_and(|data| data.op_only()))
        {
            for (i, (key, fallback)) in [
                ("item.op_warning.line1", "Warning:"),
                (
                    "item.op_warning.line2",
                    "Use of this item might lead to command execution.",
                ),
                (
                    "item.op_warning.line3",
                    "Do not use unless you know the exact contents!",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let text = sunrise_client::text::Text::legacy(
                    &self.translate_with_fallback(key, fallback),
                );
                lines.push((
                    if i == 0 { text.bold() } else { text },
                    rgba(255, 85, 85, 255),
                ));
            }
        }
        // RegisterTooltipAppendersEvent: each registered appender's lines, where its placement puts them.
        if !appenders.is_empty() {
            sunrise_client::item_tooltip::add_appender_lines(
                &mut lines,
                &details,
                stack,
                item_info,
                |appender, stack, out| {
                    if let Ok(stack) = stack.try_into_api() {
                        let mut added = Vec::new();
                        appender.append(self, &stack, &context, &mut added);
                        out.extend(added.into_iter().map(|(text, color)| {
                            (
                                sunrise_client_lib::api_text::to_engine_inprocess(&text),
                                color,
                            )
                        }));
                    }
                },
                appenders
                    .iter()
                    .map(|p| (p.component(), p.placement(), p.enabled(), p.appender())),
            );
        }
        // ItemTooltipEvent, once the tooltip vanilla shows is complete.
        if mod_callbacks
            && shown
            && mod_api::__private::listened(mod_api::HookId::ItemTooltip)
            && let Ok(metadata) = crate::game::api_values::stack_read_metadata(stack)
        {
            let project = |metadata| crate::game::api_values::project_stack(stack, metadata).ok();
            let mut access = EngineTooltipLines(&mut lines);
            self.native_item_tooltip(mod_api::__private::ItemTooltipCall {
                player: self.own_uuid(),
                player_entity_id: self.own_entity_id(),
                stack: mod_api::ItemStackView::resident(metadata, &project),
                display: &display,
                advanced,
                creative,
                lines: mod_api::TooltipLines::new(&mut access),
            });
        }
        lines
    }
    /// Menu state for the control socket.
    pub(super) fn station_info(&self) -> serde_json::Value {
        let Some(s) = self.station.as_ref() else {
            return match self.container_kind() {
                Some(BlockEntity::Brewing(b)) => {
                    serde_json::json!({ "brew_time": b.brew_time, "fuel": b.fuel })
                }
                _ => serde_json::Value::Null,
            };
        };
        let recipes = &self.station_recipes().stonecutting;
        serde_json::json!({
            "costs": s.offers.costs,
            "clues": s.offers.clues.iter().map(|c| c.as_ref().map(|(e, l)| format!("{e} {l}"))).collect::<Vec<_>>(),
            "bookcases": s.bookcases,
            "anvil_cost": s.anvil.cost,
            "result": s.result.as_ref().map(|r| format!("{} x{}", r.item, r.count)),
            "recipes": s.stone.recipes.iter().map(|&i| &recipes[i].id).collect::<Vec<_>>(),
            "recipe_results": s.stone.recipes.iter().map(|&i| recipes[i].result.item.as_ref()).collect::<Vec<_>>(),
            "selected": s.selected,
        })
    }
    /// Options.advancedItemTooltips (F3+H).
    pub(super) fn options_advanced_tooltips(&self) -> bool {
        self.settings.file.bool("advancedItemTooltips")
    }
    /// `TooltipRenderUtil`: dark box with the purple border, first line spaced.
    pub(super) fn draw_tooltip_lines<T: Clone + Into<sunrise_client::text::Text>>(
        &self,
        batch: &mut HudBatch,
        lines: &[(T, u32)],
        x: f32,
        y: f32,
    ) {
        if lines.is_empty() {
            return;
        }
        let lines = Self::resolved(lines);
        let screen = (batch.width, batch.height);
        let Some(at) = self.tooltip_pre(batch, None, &lines, &[], (x, y), screen) else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (x, y, w, h) = Self::tooltip_geometry(atlas, &lines, at, screen, [0.0; 2]);
        self.tooltip_box(batch, None, x, y, w, h);
        Self::tooltip_text(batch, atlas, &lines, x, y);
    }
    /// A tooltip's lines with their legacy codes resolved and their colours.
    fn resolved<T: Clone + Into<sunrise_client::text::Text>>(
        lines: &[(T, u32)],
    ) -> Vec<(sunrise_client::text::Text, u32)> {
        lines
            .iter()
            .map(|(text, color)| {
                let text: sunrise_client::text::Text = text.clone().into();
                (text.resolve_legacy(), *color)
            })
            .collect()
    }
    /// The tooltip's text's top-left corner and the box's size: beside the
    /// cursor, kept on screen. `extra` is [minimum width, height after the text].
    pub(super) fn tooltip_geometry(
        atlas: &sunrise_client::hud::HudAtlas,
        lines: &[(sunrise_client::text::Text, u32)],
        (x, y): (f32, f32),
        (width, height): (f32, f32),
        extra: [f32; 2],
    ) -> (f32, f32, f32, f32) {
        let w = lines
            .iter()
            .map(|(t, _)| atlas.font.width_text(t))
            .fold(0.0, f32::max)
            .max(extra[0]);
        let text_h = match lines.len() {
            0 => 0.0,
            1 => 8.0,
            n => 8.0 + 2.0 + (n - 1) as f32 * 10.0,
        };
        let h = text_h + extra[1];
        let (w, h) = (w.ceil(), h.ceil());
        let (x, y) = tooltip_position((x, y), (width, height), (w, h));
        (x, y, w, h)
    }
    /// The box around a tooltip's contents at `x`, `y`, `w` × `h`: solid fills, or the
    /// background and frame sprites of the tooltip style a mod names.
    pub(super) fn tooltip_box(
        &self,
        batch: &mut HudBatch,
        stack: Option<&ItemStack>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        let (mut background, mut border_top, mut border_bottom) =
            (0xf0100010, 0x505000ff, 0x5028007f);
        let mut texture = None;
        self.native_render_tooltip_texture(
            stack,
            &mut texture,
            &mut background,
            &mut border_top,
            &mut border_bottom,
        );
        if let Some(style) = &texture {
            let [background, frame] =
                sunrise_client::ui::tooltip_style_sprites(style.namespace(), style.path());
            let sprites = [background.as_str(), frame.as_str()];
            if sunrise_client::ui::tooltip_sprites(batch, &self.gui.atlas, sprites, [x, y, w, h]) {
                return;
            }
        }
        let convert = |color: u32| {
            let [a, r, g, b] = color.to_be_bytes();
            rgba(r, g, b, a)
        };
        let (background, border_top, border_bottom) = (
            convert(background),
            convert(border_top),
            convert(border_bottom),
        );
        let atlas = &self.gui.atlas;
        batch.fill(atlas, x - 3.0, y - 4.0, w + 6.0, 1.0, background);
        batch.fill(atlas, x - 3.0, y + h + 3.0, w + 6.0, 1.0, background);
        batch.fill(atlas, x - 4.0, y - 3.0, w + 8.0, h + 6.0, background);
        batch.fill(atlas, x - 3.0, y - 3.0, w + 6.0, 1.0, border_top);
        batch.fill(atlas, x - 3.0, y + h + 2.0, w + 6.0, 1.0, border_bottom);
        batch.fill_gradient(
            atlas,
            (x - 3.0, y - 2.0, 1.0, h + 4.0),
            border_top,
            border_bottom,
        );
        batch.fill_gradient(
            atlas,
            (x + w + 2.0, y - 2.0, 1.0, h + 4.0),
            border_top,
            border_bottom,
        );
    }
    pub(super) fn tooltip_text(
        batch: &mut HudBatch,
        atlas: &sunrise_client::hud::HudAtlas,
        lines: &[(sunrise_client::text::Text, u32)],
        x: f32,
        y: f32,
    ) {
        let mut ty = y;
        for (i, (text, color)) in lines.iter().enumerate() {
            batch.text_styled(atlas, text, x, ty, *color, true);
            ty += if i == 0 { 12.0 } else { 10.0 };
        }
    }
    /// `ClientBundleTooltip.extractSelectedItemTooltip`: the selected item's
    /// name in a box of its own, centred over the grid (less 12) and 15 above
    /// the component, placed by `DefaultTooltipPositioner` (+12, -12, kept on
    /// screen). `x`, `y` and `w` are the component's.
    fn draw_bundle_selected_name(
        &self,
        batch: &mut HudBatch,
        layout: &sunrise_client::bundle::TooltipLayout,
        x: f32,
        y: f32,
        w: f32,
    ) {
        let Some((_, stack)) = layout.selected.and_then(|i| layout.slots.get(i)) else {
            return;
        };
        let name = self.styled_stack_name(stack);
        let lines = Self::resolved(&[(name, self.rarity_color(stack))]);
        let atlas = &self.gui.atlas;
        let tw = atlas.font.width_text(&lines[0].0);
        let center = x + (w / 2.0).floor() - 12.0;
        let mut px = center - (tw / 2.0).floor() + 12.0;
        let mut py = y - 15.0 - 12.0;
        if px + tw > batch.width {
            px = (px - 24.0 - tw).max(4.0);
        }
        if py + 8.0 + 3.0 > batch.height {
            py = batch.height - 8.0 - 3.0;
        }
        self.tooltip_box(batch, Some(stack), px, py, tw, 8.0);
        Self::tooltip_text(batch, atlas, &lines, px, py);
    }
    /// `ClientBundleTooltip.extractImage`: the slots, the count past them and
    /// the progress bar.
    fn draw_bundle_tooltip(
        &mut self,
        batch: &mut HudBatch,
        layout: &sunrise_client::bundle::TooltipLayout,
    ) {
        for (i, (at, stack)) in layout.slots.iter().enumerate() {
            let selected = layout.selected == Some(i);
            let sprite = |name: &str| format!("container/bundle/{name}");
            // Each draw takes the atlas on its own: the item's needs `&mut self`.
            sunrise_client::ui::nine_slice(
                batch,
                &self.gui.atlas,
                &sprite(if selected {
                    "slot_highlight_back"
                } else {
                    "slot_background"
                }),
                at[0],
                at[1],
                24.0,
                24.0,
                u32::MAX,
            );
            let stack = stack.clone();
            let [x, y] = *at;
            self.draw_item(batch, &stack, x + 4.0, y + 4.0);
            if selected {
                sunrise_client::ui::nine_slice(
                    batch,
                    &self.gui.atlas,
                    &sprite("slot_highlight_front"),
                    x,
                    y,
                    24.0,
                    24.0,
                    u32::MAX,
                );
            }
        }
        let atlas = &self.gui.atlas;
        if let Some((at, hidden)) = layout.overflow {
            let label = sunrise_client::text::Text::literal(format!("+{hidden}"));
            let w = atlas.font.width_text(&label);
            batch.text_styled(
                atlas,
                &label,
                at[0] + 12.0 - (w / 2.0).floor(),
                at[1] + 10.0,
                u32::MAX,
                true,
            );
        }
        if let Some((at, description)) = layout
            .empty_description
            .zip(Some("item.minecraft.bundle.empty.description"))
        {
            let text = sunrise_client::text::Text::translate(description, &[]);
            let mut ty = at[1];
            for line in atlas.font.split_lines(&text, layout.width) {
                batch.text_styled(atlas, &line.0, at[0], ty, 0xFFAA_AAAA, false);
                ty += 9.0;
            }
        }
        let [bx, by] = layout.bar;
        let fill_sprite = if layout.full {
            "container/bundle/bundle_progressbar_full"
        } else {
            "container/bundle/bundle_progressbar_fill"
        };
        sunrise_client::ui::nine_slice(
            batch,
            atlas,
            fill_sprite,
            bx + 1.0,
            by,
            layout.fill,
            13.0,
            u32::MAX,
        );
        sunrise_client::ui::nine_slice(
            batch,
            atlas,
            "container/bundle/bundle_progressbar_border",
            bx,
            by,
            96.0,
            13.0,
            u32::MAX,
        );
        if let Some(key) = layout.label {
            let label = sunrise_client::text::Text::translate(key, &[]);
            let w = atlas.font.width_text(&label);
            batch.text_styled(
                atlas,
                &label,
                bx + 48.0 - (w / 2.0).floor(),
                by + 3.0,
                u32::MAX,
                true,
            );
        }
    }
    /// `AbstractContainerScreen.showTooltipWithItemInHand`: a hovered stack's
    /// tooltip shows while a stack is carried only when its image says so, as
    /// a bundle's contents do (`ClientBundleTooltip`).
    pub(super) fn tooltip_with_carried(&self, stack: &ItemStack) -> bool {
        self.carried.is_none()
            || sunrise_client::bundle::Contents::of(self.items(), stack).is_some()
    }
    /// Hovered-slot tooltip on the plain inventory, crafting and container screens.
    pub(super) fn draw_slot_tooltip(&mut self, batch: &mut HudBatch) {
        let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        // The recipe book's tooltip is drawn after the slot's own, so a ghost wins.
        if let Some(ghost) = self.ghost_stack(slot) {
            self.draw_stack_tooltip(batch, &ghost, self.cursor.0, self.cursor.1);
            return;
        }
        let stack = self.slot_view(slot);
        if self.carried.is_some() && !stack.as_ref().is_some_and(|s| self.tooltip_with_carried(s)) {
            return;
        }
        if let Some(stack) = stack {
            let mut lines = self.stack_tooltip_lines(&stack, false, false);
            self.native_screen_tooltip_lines(batch, Some(slot), &mut lines);
            if lines.is_empty()
                && !sunrise_client::item_tooltip::tooltip_display(&stack).name_visible(false)
            {
                return;
            }
            self.draw_stack_tooltip_lines(batch, &stack, lines, self.cursor.0, self.cursor.1);
        } else if let Some(text) = self.crafter_tooltip(slot) {
            self.draw_tooltip_lines(
                batch,
                &[(sunrise_client::text::Text::literal(text), u32::MAX)],
                self.cursor.0,
                self.cursor.1,
            );
        }
    }

    // ---------------------------------------------------------------- brewing stand

    /// `BrewingStandBlockEntity.serverTick` of the brewing stand at `pos`, in
    /// its place among the level's block entity tickers.
    pub(super) fn tick_brewing_stand(
        &mut self,
        pos: (i32, i32, i32),
    ) -> sunrise_client::Result<()> {
        let fuel = self.brewing_fuel_remainder(pos);
        let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get_mut(&pos) else {
            return Ok(());
        };
        let was_idle = b.brew_time <= 0;
        // `serverTick` takes the fuel item only while the stand has no fuel
        // uses left, which `out.fuel_taken` reports.
        let fuel_item = b.items[4].as_ref().map(|s| s.item.clone());
        let (mut out, due) = b.tick_to_brew(
            &world_stations!(self).brewing,
            world_items!(self),
            self.res.providers(),
            &|s| fuel.resolve(world_items!(self), s),
        );
        let started = was_idle && b.brew_time > 0;
        let ingredient = started.then(|| b.ingredient.clone()).flatten();
        let hooked = !mod_api::__private::mods().is_empty();
        let stacks = (due && hooked).then(|| b.items.clone());
        let at = [pos.0, pos.1, pos.2];
        let mut post = false;
        if due {
            // `doBrew`, with PotionBrewEvent.Pre before it and .Post after.
            let asked = stacks.map(|mut stacks| {
                let mut cancel = false;
                self.native_potion_brew_pre(at, &mut stacks, &mut cancel);
                (stacks, cancel)
            });
            // A cancelled Pre fires Post itself, with the slots the mods left.
            let cancelled = matches!(asked, Some((_, true)));
            let rest = self.brewing_ingredient_remainder(pos, cancelled);
            let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get_mut(&pos) else {
                return Ok(());
            };
            let brewing = &world_stations!(self).brewing;
            b.brew_after_pre(asked, brewing, &|_| rest.clone(), &mut out);
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            post = hooked && (out.brewed || cancelled);
        }
        if out.fuel_taken
            && let Some(item) = fuel_item
        {
            self.native_brewing_fuel(at, &item);
        }
        if let Some(ingredient) = ingredient {
            self.native_brewing_start(at, &ingredient);
        }
        if !was_idle || started || out != brewing::BrewTick::default() {
            self.container_changed(pos);
        }
        let (x, y, z) = pos;
        // `doBrew` drops the ingredient's remainder through
        // `Containers.dropItemStack`, before the brew's level event: no
        // `block_drops` test and no pickup delay.
        for d in std::mem::take(&mut out.drops) {
            self.drop_item_stack_containers(at, d);
        }
        if out.brewed {
            self.native_brewing_complete(at);
            self.play_at("minecraft:block.brewing_stand.brew", at, 1.0, 1.0);
        }
        // PotionBrewEvent.Post ends `doBrew`, after its sound and the
        // ingredient's remainder; the bottles' block state follows it.
        if post && let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get(&pos) {
            let mut stacks = b.items.clone();
            self.native_potion_brew_post(at, &mut stacks);
            if let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get_mut(&pos) {
                b.items = stacks;
            }
        }
        let Some(BlockEntity::Brewing(b)) = self.world.block_entities.get_mut(&pos) else {
            return Ok(());
        };
        b.sync_bottles(&mut out);
        if out != brewing::BrewTick::default() {
            self.world.modified.insert((x >> 4, z >> 4));
            if let Some(bottles) = out.bottles {
                let state = self.world.block(x, y, z);
                if let Some(mut s) = self.pack.assets.registry.state(state).cloned()
                    && AsRef::<str>::as_ref(&s.name) == "minecraft:brewing_stand"
                {
                    for (i, has) in bottles.iter().enumerate() {
                        s.properties
                            .insert(format!("has_bottle_{i}"), has.to_string());
                    }
                    if let Ok(id) = self.pack.assets.registry.resolve(&s) {
                        self.level_set_with_flags(
                            [x, y, z],
                            id,
                            sunrise_world::level::flags::UPDATE_CLIENTS,
                        )?;
                    }
                }
            }
        }
        if matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Brewing(b)) if b.idle())
        {
            self.world.block_entities.sleep(pos);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn spawn_egg_tooltip_warns_for_the_mod_type_in_peaceful() {
        use crate::game::{commands::peaceful_tests, execution_role::ExecutionRole};
        use sunrise_client::hud::rgba;

        peaceful_tests::with_types(ExecutionRole::Client, |game| {
            let egg = peaceful_tests::trader_egg();
            let message = game
                .pack
                .names
                .display_get("item.spawn_egg.peaceful")
                .unwrap_or_else(|| "item.spawn_egg.peaceful".into());
            let warns = |game: &super::Game<'_>| {
                game.stack_tooltip_lines(&egg, false, false)
                    .iter()
                    .any(|(text, color)| {
                        text.plain() == message && *color == rgba(255, 85, 85, 255)
                    })
            };
            assert!(warns(game));
            game.set_difficulty_forced(2);
            assert!(!warns(game));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn anvil_subscriber_probe_and_answer_cache() {
        use mod_api::{self, AnvilUpdateEvent, Flow, HookId, Hooks, HostSide, ModEntry};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        struct DefaultMod;
        #[mod_api::__subscriptions]
        impl Hooks for DefaultMod {}
        struct Listener(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                event.edits.xp_cost = -7;
                Flow::Continue
            }
        }
        let defaults = vec![ModEntry::new("default", Box::new(DefaultMod))].into();
        mod_platform::testing::with_mods(defaults, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let mut station = super::Station::new(super::Kind::Anvil, [0; 3]);
                station.inputs[0] = Some(super::ItemStack::new("minecraft:iron_sword", 1));
                station.anvil.cost = 5;
                assert!(!mod_api::__private::listened(HookId::AnvilUpdate));
                game.update_anvil_answer(&mut station, "first");
                assert_eq!(station.anvil.cost, 5);
                game.update_anvil_answer(&mut station, "second");
                assert_eq!(station.anvil.cost, 5);
            });
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new("listener", Box::new(Listener(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let mut station = super::Station::new(super::Kind::Anvil, [0; 3]);
                station.inputs[0] = Some(super::ItemStack::new("minecraft:iron_sword", 1));
                station.anvil.cost = 5;
                game.update_anvil_answer(&mut station, "first");
                assert_eq!(station.anvil.cost, 0, "negative xp_cost clamps to zero");
                station.anvil.cost = 1;
                game.update_anvil_answer(&mut station, "first");
                assert_eq!(station.anvil.cost, 0);
                game.update_anvil_answer(&mut station, "second");
                assert_eq!(station.anvil.cost, 0);
                assert_eq!(calls.load(Ordering::Relaxed), 3);
                assert!(mod_api::__private::listened(HookId::AnvilUpdate));

                // Editing the rename box without changing the effective anvil_item_name
                // does not fire anvil_update (matching vanilla AnvilMenu.setItemName).
                game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "AnvilTest").unwrap();
                let display = game.pack.names.display_item("minecraft:iron_sword");
                super::anvil::input_changed(&mut station, display);
                game.station = Some(station);
                let before = calls.load(Ordering::Relaxed);
                game.station_text(None, false);
                assert_eq!(calls.load(Ordering::Relaxed), before);
                assert!(game.station_key(winit::keyboard::KeyCode::ArrowLeft));
                assert_eq!(calls.load(Ordering::Relaxed), before);
                game.station_text(Some("x"), false);
                assert_eq!(calls.load(Ordering::Relaxed), before + 1);
                game.station_text(None, true);
                assert_eq!(calls.load(Ordering::Relaxed), before + 2);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn anvil_update_fires_at_each_create_result_in_multi_slot_drag() {
        use super::{ItemStack, Kind, Station};
        use mod_api::{
            AnvilUpdateEvent, Flow, Hooks, HostSide, ItemStack as ApiStack, ModEntry, StackResult,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_client::menu::Input;

        struct DragListener(Arc<Mutex<Vec<(u32, Option<u32>)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for DragListener {
            fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
                self.0.lock().unwrap().push((
                    event.inputs.left.count(),
                    event.inputs.right.map(ApiStack::count),
                ));
                if event.inputs.right.is_some() {
                    let dirt = mod_api::__private::item_key("minecraft:dirt").unwrap();
                    event.edits.output = StackResult::Set(Some(ApiStack::of(dirt)));
                    event.edits.xp_cost = 3;
                }
                Flow::Continue
            }
        }

        let observed = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "drag-listener",
            Box::new(DragListener(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "AnvilDrag").unwrap();
                game.remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((1, "minecraft:anvil".to_owned())));
                game.station = Some(Station::new(Kind::Anvil, [0; 3]));
                game.carried = Some(ItemStack::new("minecraft:iron_ingot", 2));

                assert!(game.remote_menu_ops(&[
                    (None, 0, Input::QuickCraft),
                    (Some(super::INPUT), 1, Input::QuickCraft),
                    (Some(super::INPUT + 1), 1, Input::QuickCraft),
                    (None, 2, Input::QuickCraft),
                ]));

                assert_eq!(
                    *observed.lock().unwrap(),
                    vec![(1, None), (1, None), (1, None), (1, Some(1)), (1, Some(1))]
                );
                let station = game.station.as_ref().unwrap();
                assert_eq!(station.result, Some(ItemStack::new("minecraft:dirt", 1)));
                assert_eq!(station.anvil.cost, 3);
                assert_eq!(game.carried, None);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn anvil_prediction_updates_between_material_consumption_and_left_input_clear() {
        use super::{ItemStack, Kind, Station};
        use mod_api::{AnvilUpdateEvent, Flow, Hooks, HostSide, ModEntry};
        use std::sync::{Arc, Mutex};
        use sunrise_client::menu::Input;

        struct Observe(Arc<Mutex<Vec<Option<u32>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observe {
            fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.inputs.right.map(mod_api::ItemStack::count));
                Flow::Continue
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "take-observer",
            Box::new(Observe(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "AnvilTake").unwrap();
                game.remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((1, "minecraft:anvil".into())));
                let mut station = Station::new(Kind::Anvil, [0; 3]);
                let repaired = ItemStack::new("minecraft:iron_sword", 1);
                let mut worn = repaired.clone();
                worn.damage = 80;
                station.inputs[0] = Some(worn);
                station.inputs[1] = Some(ItemStack::new("minecraft:iron_ingot", 2));
                station.result = Some(repaired.clone());
                station.anvil.cost = 2;
                station.anvil.repair_item_count_cost = 2;
                game.station = Some(station);
                game.experience.level = 10;

                assert!(game.remote_menu_ops(&[(Some(super::RESULT), 0, Input::Pickup)]));
                assert_eq!(
                    *calls.lock().unwrap(),
                    [None],
                    "only the intermediate update still has the left input"
                );
                let station = game.station.as_ref().unwrap();
                assert!(station.inputs[..2].iter().all(Option::is_none));
                assert_eq!(station.result, None);
                assert_eq!(game.carried, Some(repaired));
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn anvil_update_validates_only_a_changed_output() {
        use super::{AnvilResult, ItemStack, Kind, Station};
        use mod_api::{
            AnvilUpdateEvent, Flow, Hooks, HostSide, ItemStack as ApiStack, ModEntry, StackResult,
        };

        struct Listener;
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
                assert_eq!(event.inputs.left.count(), 2, "captured an over-limit stack");
                assert_eq!(event.edits.output, StackResult::Keep);
                assert_eq!((event.edits.xp_cost, event.edits.material_cost), (1, 1));
                event.edits.xp_cost = -7;
                event.edits.material_cost = -2;
                let item = event.inputs.left.item();
                match event.inputs.name {
                    Some("same") => {
                        event.edits.output = StackResult::Set(event.inputs.vanilla_output.cloned());
                    }
                    Some("replace") => {
                        event.edits.output = StackResult::Set(Some(ApiStack::of(item)));
                    }
                    Some("clear") => event.edits.output = StackResult::Set(None),
                    Some("invalid") => {
                        let invalid = ApiStack::new(item, 3).unwrap();
                        event.edits.output = StackResult::Set(Some(invalid));
                        event.edits.material_cost = 4;
                    }
                    Some("cancel") => return Flow::Cancel,
                    _ => {}
                }
                Flow::Continue
            }
        }

        let mods = vec![ModEntry::new("listener", Box::new(Listener))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let mut station = Station::new(Kind::Anvil, [0; 3]);
                let sword = |count| ItemStack::new("minecraft:iron_sword", count);
                station.inputs[0] = Some(sword(2));
                let vanilla = AnvilResult {
                    result: Some(sword(2)),
                    cost: 1,
                    repair_item_count_cost: 1,
                    only_renaming: true,
                };
                for (name, output, only_renaming) in [
                    ("keep", Some(2), true),
                    ("same", Some(2), true),
                    ("replace", Some(1), false),
                    ("clear", None, false),
                ] {
                    station.anvil = vanilla.clone();
                    game.update_anvil_answer(&mut station, name);
                    assert_eq!(
                        station.anvil,
                        AnvilResult {
                            result: output.map(sword),
                            cost: 0,
                            repair_item_count_cost: 0,
                            only_renaming,
                        },
                        "{name}",
                    );
                }
                station.anvil = vanilla.clone();
                game.update_anvil_answer(&mut station, "invalid");
                assert_eq!(
                    station.anvil,
                    AnvilResult {
                        result: Some(sword(2)),
                        cost: 0,
                        repair_item_count_cost: 4,
                        only_renaming: true,
                    },
                    "invalid",
                );
                station.anvil = vanilla.clone();
                game.update_anvil_answer(&mut station, "cancel");
                assert_eq!(station.anvil, AnvilResult::default(), "cancel");
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resource_tooltip_uses_inventory_lines_and_explicit_flags() {
        use mod_api::{
            AddAttributeTooltipsEvent, ClientHost, Hooks, ItemResource, ItemTooltipEvent, ModEntry,
            Mods, TooltipElement,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        use sunrise_core::nbt::Tag;

        const BASE_NAME: &str = "base item line";
        const REENTERED: &str = "reentered tooltip listener";
        struct Listener(AtomicBool);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn add_attribute_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut AddAttributeTooltipsEvent<'_>,
            ) {
                event
                    .lines
                    .push((mod_api::Text::literal("attribute listener"), u32::MAX));
            }

            fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                if self.0.swap(true, Ordering::Relaxed) {
                    event
                        .lines
                        .push(mod_api::Text::literal(REENTERED), u32::MAX)
                        .unwrap();
                    return;
                }
                // Asking for the same stack must return base lines, not enter another listener.
                let nested = ItemResource::of(&event.stack.project().unwrap())
                    .tooltip(host, event.advanced, event.creative)
                    .unwrap();
                self.0.store(false, Ordering::Relaxed);
                assert!(matches!(
                    nested.first(),
                    Some(TooltipElement::Text(text, _)) if *text == mod_api::Text::literal(BASE_NAME)
                ));
                assert!(!nested.contains(&TooltipElement::Text(
                    mod_api::Text::literal(REENTERED),
                    u32::MAX,
                )));
                event
                    .lines
                    .push(mod_api::Text::literal("listener"), u32::MAX)
                    .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new(
            "listener",
            Box::new(Listener(AtomicBool::new(false))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let name = Tag::compound([("text", Tag::String(BASE_NAME.into()))]);
                let mut stack = super::ItemStack::new("minecraft:iron_sword", 1);
                stack
                    .data_mut()
                    .set_other("minecraft:item_name", Some(name.clone()));
                let resource = ItemResource::of(&crate::game::api_values::test_snapshot(&stack));
                let inventory: Vec<_> = game
                    .stack_tooltip_lines(&stack, false, false)
                    .into_iter()
                    .map(|(text, color)| TooltipElement::Text(text.try_into_api().unwrap(), color))
                    .collect();
                let advanced = game.options_advanced_tooltips();
                assert!(resource.tooltip(game, advanced, false).unwrap() == inventory);
                let attribute_line =
                    TooltipElement::Text(mod_api::Text::literal("attribute listener"), u32::MAX);
                assert!(inventory.contains(&attribute_line));
                let mut hidden = stack.clone();
                hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
                );
                let hidden = ItemResource::of(&crate::game::api_values::test_snapshot(&hidden));
                assert!(
                    !hidden
                        .tooltip(game, false, false)
                        .unwrap()
                        .contains(&attribute_line)
                );
                let mut attributes_hidden = stack.clone();
                attributes_hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(Tag::compound([(
                        "hidden_components",
                        Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
                    )])),
                );
                let attributes_hidden =
                    ItemResource::of(&crate::game::api_values::test_snapshot(&attributes_hidden));
                let lines = attributes_hidden.tooltip(game, false, false).unwrap();
                assert!(lines.contains(&attribute_line));
                assert!(lines.contains(&TooltipElement::Text(
                    mod_api::Text::literal("listener"),
                    u32::MAX,
                )));

                let mut crossbow = super::ItemStack::new("minecraft:crossbow", 1);
                crossbow
                    .data_mut()
                    .set_other("minecraft:item_name", Some(name));
                crossbow
                    .data_mut()
                    .charged
                    .push(super::ItemStack::new("minecraft:arrow", 1));
                let lines = game.stack_tooltip_lines(&crossbow, false, false);
                assert_eq!(
                    lines
                        .iter()
                        .filter(|(text, _)| text.plain().trim() == "listener")
                        .count(),
                    1,
                    "only the crossbow gets the final tooltip event",
                );

                for advanced in [false, true] {
                    let lines = resource.tooltip(game, advanced, false).unwrap();
                    assert!(lines.last().is_some_and(|line| {
                        *line == TooltipElement::Text(mod_api::Text::literal("listener"), u32::MAX)
                    }));
                    assert_eq!(
                        lines.iter().any(|line| matches!(
                            line,
                            TooltipElement::Text(text, _) if *text == mod_api::Text::literal("minecraft:iron_sword")
                        )),
                        advanced
                    );
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn tooltip_part_context_tracks_flags_and_client_tick_rate() {
        use mod_api::{
            ClientHost, Hooks, ItemPart, ModEntry, Registrar, TooltipAppendContext, TooltipEvent,
        };
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Part(Arc<Mutex<Vec<TooltipAppendContext>>>);
        #[mod_api::sunrise_part]
        impl ItemPart for Part {
            fn tooltip(&self, _: &dyn ClientHost, event: &mut TooltipEvent<'_>) {
                self.0.lock().unwrap().push(*event.context);
                event
                    .lines
                    .push((mod_api::Text::literal("context"), u32::MAX));
            }
        }
        struct Mod(Part);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let item = registrar.item_key("minecraft:stick");
                registrar.item_part(item, self.0.clone());
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "tooltip_context",
            Box::new(Mod(Part(heard.clone()))),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                use sunrise_protocol::{
                    connection::Pipe,
                    packets::play::{self, Clientbound as C},
                };

                let stack = super::ItemStack::new("minecraft:stick", 1);
                let assert_context = |game: &mut super::Game<'_>, advanced, rate| {
                    let lines = game.tooltip_lines(&stack, false, false, advanced);
                    assert!(lines.iter().any(|(text, _)| text.plain() == "context"));
                    let context = heard.lock().unwrap().pop().expect("item tooltip Part ran");
                    assert_eq!(context.advanced, advanced);
                    assert_eq!(context.tick_rate, rate);
                    assert!(!context.creative && !context.nested);
                };
                game.clock.set_rate(40.0);
                assert!(game.native_client_level().is_none());
                assert_context(game, false, 20.0);

                let (client, _server) = Pipe::pair();
                game.connect_local(client, "TooltipContextTest").unwrap();
                let ticking = |rate, frozen| C::TickingState(play::TickingState { rate, frozen });
                let step = || C::TickingStep(play::TickingStep { steps: 4 });
                game.handle_packet(ticking(37.5, true)).unwrap();
                game.handle_packet(step()).unwrap();
                assert_eq!(game.clock.rate(), 20.0, "no level has received a rate");
                assert_eq!(game.clock.frozen(), game.clock.still());
                assert!(!game.clock.stepping());

                let mut spawn = play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.handle_packet(C::Login(play::Login {
                    player_id: 7,
                    hardcore: false,
                    levels: vec![spawn.dimension.clone(), "test:other_level".into()],
                    max_players: 1,
                    chunk_radius: 2,
                    simulation_distance: 2,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: spawn.clone(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }))
                .unwrap();
                let first_level = game.native_client_level().expect("Login starts a level");
                assert_context(game, true, 20.0);

                game.handle_packet(ticking(37.5, true)).unwrap();
                game.handle_packet(step()).unwrap();
                assert_context(game, false, 37.5);

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn.clone(),
                    keep: 0,
                }))
                .unwrap();
                assert_eq!(game.native_client_level(), Some(first_level));
                assert_context(game, true, 37.5);
                assert!(game.clock.frozen() && game.clock.stepping());

                // A different key with the same dimension type still creates a fresh ClientLevel.
                spawn.dimension = "test:other_level".into();
                game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                    .unwrap();
                let next_level = game.native_client_level().expect("Respawn starts a level");
                assert_ne!(first_level, next_level);
                assert_context(game, false, 20.0);
                assert_eq!(game.clock.frozen(), game.clock.still());
                assert!(!game.clock.stepping());

                game.handle_packet(ticking(12.5, false)).unwrap();
                assert_context(game, true, 12.5);
                assert!(heard.lock().unwrap().is_empty());
            });
        });
    }

    /// A stick's part adds its line where `Item.appendHoverText` does, after the name and before
    /// the advanced id line; `item_tooltip` (`ItemTooltipEvent`) then gets the whole tooltip, with
    /// its flags, and its changes are what shows. A stick whose tooltip is hidden asks neither.
    #[test]
    #[ignore = "daily"]
    fn tooltip_part_lines_come_after_the_name_and_never_for_a_hidden_tooltip() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use mod_api::{
            self, ClientHost, Hooks, ItemPart, ItemTooltipEvent, ModEntry, Mods, Registrar,
            TooltipEvent,
        };

        if run_fresh_cases(
            "game::stations::tests::tooltip_part_lines_come_after_the_name_and_never_for_a_hidden_tooltip",
            "SUNRISE_TOOLTIP_PART_CHILD",
            &["tooltip"],
        ) {
            return;
        }

        #[derive(Clone)]
        struct Lore;
        #[mod_api::sunrise_part]
        impl ItemPart for Lore {
            fn tooltip(&self, _: &dyn ClientHost, event: &mut TooltipEvent<'_>) {
                assert!(event.lines.is_empty());
                event.lines.push((mod_api::Text::literal("part"), u32::MAX));
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let item = r.item_key("minecraft:stick");
                r.item_part(item, Lore);
            }
            fn item_tooltip(&self, _: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                assert_eq!(
                    event
                        .lines
                        .iter()
                        .take(3)
                        .map(|line| line.unwrap().0)
                        .collect::<Vec<_>>(),
                    [
                        mod_api::Text::translate("item.minecraft.stick", Vec::new()),
                        mod_api::Text::literal("part"),
                        mod_api::Text::literal("minecraft:stick"),
                    ]
                );
                assert!(event.advanced);
                // Only the hidden stack is asked from a creative slot, where its
                // display hides every component's lines.
                assert_eq!(event.display.shows("minecraft:lore"), !event.creative);
                // Every line is the mod's to change: drop the count, add one at the end.
                event.lines.pop();
                event
                    .lines
                    .push(mod_api::Text::literal("event"), u32::MAX)
                    .unwrap();
            }
        }
        let mods: Mods = vec![ModEntry::new("lore", Box::new(Mod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.settings.file.set(
                    "advancedItemTooltips",
                    sunrise_client::options_file::V::Bool(true),
                );
                let stick = super::ItemStack::new("minecraft:stick", 1);
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&stick, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert_eq!(lines, ["Stick", "part", "minecraft:stick", "event"]);
                // `hide_tooltip`: no name, Part line, or tooltip event line.
                let mut hidden = stick.clone();
                hidden.data_mut().set_other(
                    "minecraft:tooltip_display",
                    Some(sunrise_core::nbt::Tag::compound([(
                        "hide_tooltip",
                        sunrise_core::nbt::Tag::Byte(1),
                    )])),
                );
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(
                    !lines
                        .iter()
                        .any(|l| l == "Stick" || l == "part" || l == "event"),
                    "{lines:?}"
                );
                // The creative screen's inventory and hotbar slots aren't creative
                // tab slots: still hidden there.
                game.set_screen(super::Screen::Creative);
                let inventory: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(
                    !inventory
                        .iter()
                        .any(|l| l == "Stick" || l == "part" || l == "event"),
                    "{inventory:?}"
                );
                // A creative tab's own slot (`TooltipFlag.isCreative`) shows it anyway.
                let creative: Vec<String> = game
                    .stack_tooltip_lines(&hidden, false, true)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert_eq!(creative[..2], ["Stick", "part"]);
                assert_eq!(creative.last().map(String::as_str), Some("event"));
            });
        });
    }
    /// A helmet's armour lines go where `ItemStack.addAttributeTooltips` puts them; a mod's
    /// `gather_skipped_attribute_tooltips` takes a slot group out, heading and all, and its
    /// `add_attribute_tooltips` line lands right after what is left, with the local player
    /// handed to both.
    #[test]
    #[ignore = "daily"]
    fn attribute_tooltip_hooks_hide_modifiers_and_add_lines_after_them() {
        use mod_api::{
            self, AddAttributeTooltipsEvent, ClientHost, GatherSkippedAttributeTooltipsEvent,
            Hooks, ModEntry, Mods,
        };
        use std::sync::atomic::{AtomicBool, Ordering};

        static HIDE: AtomicBool = AtomicBool::new(false);
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn gather_skipped_attribute_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut GatherSkippedAttributeTooltipsEvent<'_>,
            ) {
                assert!(event.player.is_some(), "the ordinary tooltip has a player");
                if HIDE.load(Ordering::Relaxed) {
                    event.skipped_groups.push("head".into());
                }
            }
            fn add_attribute_tooltips(
                &self,
                _: &dyn ClientHost,
                event: &mut AddAttributeTooltipsEvent<'_>,
            ) {
                assert!(event.shown);
                event.lines.push((
                    {
                        let mut line = mod_api::Text::literal("Rune: warding");
                        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                            line = mod_api::Text::literal("").append(line);
                        }
                        line
                    },
                    u32::MAX,
                ));
            }
        }
        let mods: Mods = vec![ModEntry::new("runes", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let helmet = super::ItemStack::new("minecraft:diamond_helmet", 1);
                let plain = |game: &super::Game<'_>| -> Vec<String> {
                    game.stack_tooltip_lines(&helmet, false, false)
                        .iter()
                        .map(|(text, _)| text.plain())
                        .collect()
                };
                HIDE.store(false, Ordering::Relaxed);
                let shown = plain(game);
                let heading = shown
                    .iter()
                    .position(|l| l.contains("on Head"))
                    .unwrap_or_else(|| panic!("{shown:?}"));
                let rune = shown.iter().position(|l| l == "Rune: warding").unwrap();
                assert!(rune > heading, "after the attribute lines: {shown:?}");
                HIDE.store(true, Ordering::Relaxed);
                let hidden = plain(game);
                assert!(!hidden.iter().any(|l| l.contains("on Head")), "{hidden:?}");
                assert!(!hidden.iter().any(|l| l.contains("Armor")), "{hidden:?}");
                assert!(hidden.iter().any(|l| l == "Rune: warding"), "{hidden:?}");
            });
        });
    }
    /// `render_tooltip_gather_components` gets the lines and the part's image as one list before
    /// anything is wrapped or measured: unset, `max_width` leaves the lines alone; a mod that sets
    /// it wraps them with `Font.split`, can add an image of its own, and a cancel draws nothing.
    #[test]
    #[ignore = "daily"]
    fn gathered_tooltip_components_wrap_only_when_a_mod_sets_the_width() {
        use mod_api::{
            self, ClientHost, Hooks, ModEntry, Mods, RenderTooltipGatherComponentsEvent,
            TooltipElement, TooltipImage,
        };
        use std::sync::atomic::{AtomicU8, Ordering};
        use sunrise_client::text::Text;

        static MODE: AtomicU8 = AtomicU8::new(0);
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_tooltip_gather_components(
                &self,
                _: &dyn ClientHost,
                event: &mut RenderTooltipGatherComponentsEvent<'_>,
            ) -> mod_api::Flow {
                assert_eq!(
                    mod_api::__private::item_key_id(event.stack.item()).unwrap(),
                    "minecraft:stick"
                );
                assert_eq!((event.screen_width, event.screen_height), (320, 240));
                assert_eq!(event.max_width, -1);
                match MODE.load(Ordering::Relaxed) {
                    1 => {
                        event.max_width = 40;
                        event.elements.insert(
                            1,
                            TooltipElement::Image(TooltipImage {
                                source: mod_api::TooltipImageSource::ItemPart(event.stack.clone()),
                                size: [16.0, 8.0],
                            }),
                        );
                        event.elements.push(TooltipElement::Image(TooltipImage {
                            source: mod_api::TooltipImageSource::ItemPart(event.stack.clone()),
                            size: [f32::NAN, 8.0],
                        }));
                    }
                    3 => {
                        let mut name = mod_api::Text::literal("Name");
                        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                            name = mod_api::Text::literal("").append(name);
                        }
                        *event.elements = vec![
                            TooltipElement::Text(name, u32::MAX),
                            TooltipElement::Image(TooltipImage {
                                source: mod_api::TooltipImageSource::ItemPart(event.stack.clone()),
                                size: [12.0, 16.0],
                            }),
                            TooltipElement::Text(mod_api::Text::literal("Second"), u32::MAX),
                            TooltipElement::Text(mod_api::Text::literal("Third"), u32::MAX),
                        ];
                    }
                    2 => return mod_api::Flow::Cancel,
                    _ => {}
                }
                mod_api::Flow::Continue
            }
        }
        let mods: Mods = vec![ModEntry::new("gather", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let stick = super::ItemStack::new("minecraft:stick", 1);
                let long = Text::literal("one two three four five six seven eight");
                let lines = vec![(long.clone(), u32::MAX)];
                let gather = |game: &super::Game<'_>| {
                    game.gather_tooltip_components(
                        &stick,
                        lines.clone(),
                        Vec::new(),
                        (320.0, 240.0),
                    )
                };
                MODE.store(0, Ordering::Relaxed);
                let plain = gather(game).unwrap();
                assert_eq!(plain.len(), 1);
                assert!(matches!(plain[0], tooltip::Element::Text(_, _)));
                MODE.store(1, Ordering::Relaxed);
                let elements = gather(game).unwrap();
                let wrapped: Vec<_> = elements
                    .iter()
                    .filter_map(|e| match e {
                        tooltip::Element::Text(t, c) => Some((t, c)),
                        _ => None,
                    })
                    .collect();
                let images: Vec<_> = elements
                    .iter()
                    .filter_map(|e| match e {
                        tooltip::Element::Image(i) => Some(i),
                        _ => None,
                    })
                    .collect();
                let font = &game.gui.atlas.font;
                let expected = font.split(&long, 40.0);
                assert!(expected.len() > 1);
                assert_eq!(
                    wrapped.iter().map(|(t, _)| t.plain()).collect::<Vec<_>>(),
                    expected.iter().map(Text::plain).collect::<Vec<_>>()
                );
                assert!(wrapped.iter().all(|(t, _)| font.width_text(t) <= 40.0));
                // The invalid image is dropped, the valid one stays.
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].size, [16.0, 8.0]);
                // Wrapping expands text in place, never moving the image behind later text.
                assert!(matches!(elements.last(), Some(tooltip::Element::Image(_))));
                MODE.store(3, Ordering::Relaxed);
                let mixed = gather(game).unwrap();
                assert_eq!(mixed.len(), 4);
                assert!(
                    matches!(&mixed[0], tooltip::Element::Text(text, _) if text.plain() == "Name")
                );
                assert!(matches!(mixed[1], tooltip::Element::Image(_)));
                assert!(matches!(mixed[2], tooltip::Element::Text(_, _)));
                assert!(matches!(mixed[3], tooltip::Element::Text(_, _)));
                MODE.store(2, Ordering::Relaxed);
                assert!(gather(game).is_none());
            });
        });
    }
    #[test]
    fn measured_image_tooltips_position_the_whole_box() {
        use super::tooltip_position;
        assert_eq!(
            tooltip_position((50.0, 90.0), (200.0, 160.0), (40.0, 8.0)),
            (62.0, 78.0)
        );
        // A wide image makes the whole tooltip move to the other side.
        assert_eq!(
            tooltip_position((90.0, 90.0), (200.0, 160.0), (128.0, 42.0)),
            (4.0, 78.0)
        );
        // Its height participates in the existing bottom-edge adjustment too.
        assert_eq!(
            tooltip_position((90.0, 90.0), (200.0, 80.0), (128.0, 42.0)),
            (4.0, 35.0)
        );
    }
    #[test]
    fn image_tooltip_measurements_are_finite_and_nonempty() {
        use super::tooltip_component_size;
        assert_eq!(tooltip_component_size(None), None);
        assert_eq!(
            tooltip_component_size(Some([128.0, 32.0])),
            Some([128.0, 32.0])
        );
        for size in [
            [0.0, 32.0],
            [32.0, -1.0],
            [f32::NAN, 32.0],
            [32.0, f32::INFINITY],
        ] {
            assert_eq!(tooltip_component_size(Some(size)), None);
        }
    }
    use super::*;

    #[test]
    fn enchanting_book_opens_with_offers_and_turns_for_a_new_item() {
        let mut book = EnchantBook::new(LegacyRandom::new(0));
        // No offers: the book stays shut and on its page.
        book.tick(None, [0; 3]);
        assert_eq!(book.at(1.0), (0.0, 0.0));
        // An item with offers: it opens 0.2 a tick and picks a page
        // more than one away to turn to.
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        book.tick(Some(&sword), [3, 10, 30]);
        assert!((book.open - 0.2).abs() < 1e-6);
        assert!((book.flip_t - book.flip).abs() > 1.0 - 0.2);
        let target = book.flip_t;
        for _ in 0..60 {
            book.tick(Some(&sword), [3, 10, 30]);
        }
        assert_eq!(book.open, 1.0);
        assert!(
            (book.flip - target).abs() < 0.05,
            "{} {}",
            book.flip,
            target
        );
        // The same item keeps the page; taking it out closes the book.
        assert_eq!(book.flip_t, target);
        for _ in 0..6 {
            book.tick(None, [0; 3]);
        }
        assert_eq!(book.open, 0.0);
        assert_ne!(book.flip_t, target);
        let (open, _) = book.at(0.5);
        assert!(open < 1e-6);
    }

    #[test]
    fn a_step_leaves_the_visible_rows_only_from_the_end_one() {
        let rows = [[10.0, 0.0, 10.0, 10.0], [10.0, 10.0, 10.0, 10.0]];
        assert!(off_visible_rows(&rows, (15.0, 15.0), false));
        assert!(!off_visible_rows(&rows, (15.0, 15.0), true));
        assert!(off_visible_rows(&rows, (15.0, 5.0), true));
        assert!(!off_visible_rows(&rows, (15.0, 5.0), false));
        assert!(!off_visible_rows(&rows, (50.0, 5.0), false));
    }

    #[test]
    fn stonecutter_scroller_rows() {
        // 26 recipes = 7 rows, 4 below the grid.
        assert_eq!(stonecutter_offscreen_rows(26), 4);
        assert_eq!(stonecutter_start(26, 0.0), 0);
        assert_eq!(stonecutter_start(26, 0.5), 8);
        assert_eq!(stonecutter_start(26, 0.6), 8);
        assert_eq!(stonecutter_start(26, 0.65), 12);
        assert_eq!(stonecutter_start(26, 1.0), 16);
    }
    #[test]
    fn stonecutter_drag_follows_the_cursor() {
        // The scroller's middle sits under the cursor, within its track.
        assert_eq!(stonecutter_drag_offs(0.0), 0.0);
        assert_eq!(stonecutter_drag_offs(21.5), 0.0);
        assert_eq!(stonecutter_drag_offs(21.5 + 39.0 / 2.0), 0.5);
        assert_eq!(stonecutter_drag_offs(21.5 + 39.0), 1.0);
        assert_eq!(stonecutter_drag_offs(200.0), 1.0);
    }
    #[test]
    fn stonecutter_wheel_moves_one_row() {
        let mut s = Station::new(Kind::Stonecutter, [0, 0, 0]);
        s.inputs[0] = Some(ItemStack::new("minecraft:stone", 1));
        s.stone.recipes = (0..26).collect();
        assert!(s.stonecutter_scrolls());
        s.stonecutter_scroll_to(s.scroll + 1.0 / 4.0);
        assert_eq!(s.start, 4);
        s.stonecutter_scroll_to(s.scroll + 5.0);
        assert_eq!((s.scroll, s.start), (1.0, 16));
        s.stone.recipes.truncate(12);
        assert!(!s.stonecutter_scrolls());
    }
    #[test]
    #[ignore = "daily"]
    fn item_tooltip_invalid_edit_keeps_other_listeners_lines() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use mod_api::{ClientHost, Hooks, ItemTooltipEvent, ModEntry, Mods, Text, TextFormError};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_core::nbt::Tag;

        if run_fresh_cases(
            "game::stations::tests::item_tooltip_invalid_edit_keeps_other_listeners_lines",
            "SUNRISE_ITEM_TOOLTIP_INVALID_EDIT_CHILD",
            &["malformed"],
        ) {
            return;
        }

        struct ValidListener;
        #[mod_api::__subscriptions]
        impl Hooks for ValidListener {
            fn item_tooltip(&self, host: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                assert_eq!(event.stack.item().id(host), Some("minecraft:white_banner"));
                assert_eq!(event.stack.count(), 3);
                assert!(event.display.name_visible(false));
                assert!(!event.display.shows("minecraft:banner_patterns"));
                event
                    .lines
                    .push(Text::literal("listener one"), u32::MAX)
                    .unwrap();
            }
        }

        struct InvalidListener(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for InvalidListener {
            fn item_tooltip(&self, _: &dyn ClientHost, event: &mut ItemTooltipEvent<'_>) {
                let mut malformed = Text::literal("invalid");
                malformed.metadata = Tag::Int(7);
                assert_eq!(
                    event.lines.push(malformed.clone(), u32::MAX),
                    Err(TextFormError::Malformed)
                );
                assert_eq!(
                    event.lines.set(0, malformed, u32::MAX),
                    Err(TextFormError::Malformed)
                );
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let ran_second = Arc::new(AtomicBool::new(false));
        let mods: Mods = vec![
            ModEntry::new("first", Box::new(ValidListener)),
            ModEntry::new("second", Box::new(InvalidListener(ran_second.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.settings.file.set(
                    "advancedItemTooltips",
                    sunrise_client::options_file::V::Bool(false),
                );
                let mut banner = sunrise_client::mobs::raider::ominous_banner();
                banner.count = 3;
                let lines: Vec<String> = game
                    .stack_tooltip_lines(&banner, false, false)
                    .iter()
                    .map(|(text, _)| text.plain())
                    .collect();
                assert!(ran_second.load(Ordering::Relaxed));
                assert_eq!(lines, ["Ominous Banner", "listener one"]);
            });
        });
    }
}

#[cfg(test)]
mod cycling_tests {
    use super::*;

    #[test]
    fn an_empty_slot_cycles_its_icons_every_30_ticks_and_fades_the_next_in_over_4() {
        let icons = ["helmet", "sword", "chestplate"];
        let mut slot = CyclingSlot::default();
        slot.advance(100, &icons);
        // The first icon, fully shown, until the 30th tick.
        assert_eq!(slot.shown(0.5), vec![("helmet", 1.0)]);
        slot.advance(129, &icons);
        assert_eq!(slot.shown(0.5), vec![("helmet", 1.0)]);
        // The 30th tick moves to the next icon, which fades in over 4 ticks while the
        // one before fades out.
        slot.advance(130, &icons);
        assert_eq!(slot.shown(0.0), vec![("helmet", 1.0), ("sword", 0.0)]);
        assert_eq!(slot.shown(2.0), vec![("helmet", 0.5), ("sword", 0.5)]);
        assert_eq!(slot.shown(4.0), vec![("sword", 1.0)]);
        // A new list starts again from its first icon, keeping the tick count.
        slot.advance(131, &["ingot"]);
        assert_eq!(slot.shown(0.0), vec![("ingot", 1.0)]);
        // No icons, nothing shown.
        slot.advance(132, &[]);
        assert!(slot.shown(0.0).is_empty());
    }

    #[test]
    fn the_icons_of_a_smithing_slot_follow_the_template() {
        let (base, n, additions, m) =
            smithing_slot_icons(Some("minecraft:netherite_upgrade_smithing_template"));
        assert_eq!(base[..n].len(), 11);
        assert_eq!(additions[..m], ["ingot"]);
        let (base, n, additions, m) =
            smithing_slot_icons(Some("minecraft:coast_armor_trim_smithing_template"));
        assert_eq!(base[..n], ["helmet", "chestplate", "leggings", "boots"]);
        assert_eq!(additions[..m].len(), 7);
        assert_eq!(smithing_slot_icons(Some("minecraft:stick")).1, 0);
        assert_eq!(smithing_slot_icons(None).3, 0);
    }

    /// `render_tooltip_pre` sees a tooltip's lines, stack and screen, can move the point it hangs
    /// from, and cancels it; with no mod the point is the cursor's.
    #[test]
    #[ignore = "daily"]
    fn a_tooltip_pre_answer_moves_or_cancels_the_tooltip() {
        use mod_api::{
            self, ClientHost, Hooks, ModEntry, Mods, RenderTooltipPreEvent, TooltipElement,
        };
        use std::sync::atomic::{AtomicU8, Ordering};
        use sunrise_client::text::Text;

        static MODE: AtomicU8 = AtomicU8::new(0);
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_tooltip_pre(
                &self,
                _: &dyn ClientHost,
                event: &mut RenderTooltipPreEvent<'_>,
            ) -> mod_api::Flow {
                assert_eq!(event.elements.len(), 1);
                assert!(
                    matches!(&event.elements[0], TooltipElement::Text(t, _) if *t == mod_api::Text::literal("Tip"))
                );
                assert_eq!((event.place.x, event.place.y), (10, 20));
                assert_eq!((event.screen_width, event.screen_height), (320, 240));
                match MODE.load(Ordering::Relaxed) {
                    1 => {
                        event.place.x = 50;
                        event.place.y = 60;
                    }
                    2 => {
                        event.draw.gui_draw_item_with(
                            &mod_api::ItemStack::new(
                                mod_api::__private::item_key("minecraft:diamond").unwrap(),
                                5,
                            )
                            .unwrap(),
                            [16.0, 32.0],
                            Some("7"),
                        );
                        return mod_api::Flow::Cancel;
                    }
                    _ => {}
                }
                mod_api::Flow::Continue
            }
        }
        let lines = vec![(Text::literal("Tip"), u32::MAX)];
        let mods: Mods = vec![ModEntry::new("pre", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let mut batch = HudBatch::scratch([320, 240]);
                let mut pre = |game: &super::Game<'_>| {
                    game.tooltip_pre(&mut batch, None, &lines, &[], (10.6, 20.4), (320.0, 240.0))
                };
                MODE.store(0, Ordering::Relaxed);
                // Unmoved, the cursor's own fractional point stays.
                assert_eq!(pre(game), Some((10.6, 20.4)));
                MODE.store(1, Ordering::Relaxed);
                assert_eq!(pre(game), Some((50.0, 60.0)));
                MODE.store(2, Ordering::Relaxed);
                assert_eq!(pre(game), None);
                assert!(
                    !batch.vertices.is_empty(),
                    "custom drawing survives tooltip cancellation"
                );
            });
        });
    }

    #[test]
    fn tooltip_position_rounds_and_stays_within_bounds() {
        let (x, y) = tooltip_position((10.3, 20.7), (640.0, 360.0), (100.0, 20.0));
        assert_eq!((x, y), (22.0, 9.0));

        // When overflowing right boundary, flips to left
        let (x, _y) = tooltip_position((600.0, 50.0), (640.0, 360.0), (100.0, 20.0));
        assert_eq!(x, 484.0);

        // Clamps to min 4.0 margin
        let (x, y) = tooltip_position((50.0, 50.0), (100.0, 100.0), (120.0, 20.0));
        assert_eq!(x, 4.0);
        assert_eq!(y, 38.0);

        // Near top screen edge
        let (_x, y) = tooltip_position((50.0, 5.0), (640.0, 360.0), (100.0, 20.0));
        assert_eq!(y, 4.0);
    }
}

#[cfg(test)]
#[path = "tests/brewing_changed_tests.rs"]
mod brewing_changed_tests;

#[cfg(test)]
#[path = "stations/brewing_tick_tests.rs"]
mod brewing_tick_tests;

#[cfg(test)]
#[path = "stations/stonecutter_scroll_tests.rs"]
mod stonecutter_scroll_tests;
