//! The rule a connected texture is declared as, whatever file format says it: which blocks and
//! sprites it applies to, which faces, what it joins and how it picks a tile. A format's reader
//! (OptiFine's `.properties`, a mod's asset file) builds these; the client's mesher walks them.

use crate::{Symmetry, TableSheet};

/// In this order: the overlays last ([`Method::is_overlay`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    /// The 47 tiles of the 8-way template.
    Ctm,
    /// 5 tiles, cut into quarters, and `ctm.N` replacements.
    CtmCompact,
    Horizontal,
    Vertical,
    HorizontalVertical,
    VerticalHorizontal,
    Top,
    Random,
    Repeat,
    Fixed,
    /// A tile of a sheet picked by a lookup on the neighbours ([`Rule::table`]).
    Table,
    /// The 17 transition tiles drawn over a face beside the connecting blocks.
    Overlay,
    OverlayCtm,
    OverlayRandom,
    OverlayRepeat,
    OverlayFixed,
    /// Continuity's overlay forms of the 4-tile methods.
    OverlayHorizontal,
    OverlayVertical,
    OverlayHorizontalVertical,
    OverlayVerticalHorizontal,
}

impl Method {
    /// Draws a second quad over the face instead of replacing its sprite.
    pub fn is_overlay(self) -> bool {
        self >= Method::Overlay
    }
}

/// When two neighbouring faces join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connect {
    /// The same block.
    Block,
    /// The same sprite on that face.
    Tile,
    /// The same block state.
    State,
    /// Any block of `connect_blocks`.
    Blocks,
    /// The same block, with the same values of the state properties in `connect_properties`.
    Properties,
    /// By the collision height of the neighbour's block state (`max Y` of its shape).
    Height(HeightJoin),
}

/// The clauses of [`Connect::Height`]: a neighbour joins when any clause that is on holds.
/// Heights are the highest collision point of a block state's shape; an empty shape has none, so
/// it is below every height and its cell above never has a top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeightJoin {
    /// The neighbour is as high as this block, and in `Rule::connect_blocks` when that is not
    /// empty.
    pub same: bool,
    /// The neighbour is higher than this block.
    pub higher: bool,
    /// The neighbour is lower than this block.
    pub lower: bool,
    /// The cell above the neighbour has a collision top.
    pub above: bool,
    /// A neighbour on an edge of the face also joins when `higher` or `above` holds for either
    /// cell beside this block along the face's other axis.
    pub side: bool,
}

impl HeightJoin {
    /// Whether the rule says anything: a rule with no clause on joins nothing.
    pub fn any(self) -> bool {
        self.same || self.higher || self.lower || self.above
    }

    /// Whether a neighbour at `height` joins a block at `own`, `above` being the height of the
    /// cell above the neighbour and `listed` whether the neighbour is a block `same` compares:
    /// the one comparison, for the neighbour and for the cells beside this block (`side`, with
    /// `same` and `lower` off).
    pub fn joins(self, own: f64, height: f64, above: f64, listed: bool) -> bool {
        (self.same && listed && height == own)
            || (self.higher && height > own)
            || (self.lower && height < own)
            || (self.above && above > 0.0)
    }

    /// [`Self::joins`] as `side` asks it of a cell beside this block: only `higher` and `above`.
    pub fn joins_beside(self, own: f64, height: f64, above: f64) -> bool {
        Self {
            same: false,
            lower: false,
            ..self
        }
        .joins(own, height, above, false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tile {
    /// A texture's resource path (`assets/<ns>/....png`).
    Texture(String),
    /// Nothing from this rule: the next rule is tried (an overlay draws nothing).
    Skip,
    /// The face's own sprite.
    Default,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    #[default]
    Cutout,
    Translucent,
}

/// How a face's left, right, up and down are found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    /// The face's directions on a vanilla cube.
    None,
    /// [`Orient::None`], turned with the block's `axis` as a cube column's model is.
    StateAxis,
    /// The quad's own texture directions.
    Texture,
}

/// Bits of [`Rule::faces`], by `Direction` order: down, up, north, south, west, east.
pub mod faces {
    use crate::Face;

    pub const DOWN: u8 = Face::Down.bit();
    pub const UP: u8 = Face::Up.bit();
    pub const NORTH: u8 = Face::North.bit();
    pub const SOUTH: u8 = Face::South.bit();
    pub const WEST: u8 = Face::West.bit();
    pub const EAST: u8 = Face::East.bit();
    pub const SIDES: u8 = NORTH | SOUTH | WEST | EAST;
    pub const ALL: u8 = DOWN | UP | NORTH | SOUTH | WEST | EAST;
}

/// A block, and the values its state properties must have: `oak_log` with `axis` `x` or `y`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSel {
    /// The block's full id (`minecraft:oak_log`).
    pub id: String,
    /// A state test admits any of its values.
    pub states: Vec<(String, Vec<String>)>,
}

impl BlockSel {
    /// `state` answers a block state property's value.
    pub fn matches<'a>(&self, id: &str, state: impl Fn(&str) -> Option<&'a str>) -> bool {
        id == self.id
            && self
                .states
                .iter()
                .all(|(key, values)| state(key).is_some_and(|v| values.iter().any(|w| w == v)))
    }
}

/// Inclusive ranges: `(low, high)` pairs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges(pub Vec<(i32, i32)>);

impl Ranges {
    pub fn contains(&self, n: i32) -> bool {
        self.0.iter().any(|&(lo, hi)| (lo..=hi).contains(&n))
    }
}

/// Resource ids: the ones listed (as full, lowercase ids), but not the excluded. An empty include
/// list admits every id that is not excluded. A queried id without a namespace is in `minecraft`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ids {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Ids {
    pub fn matches(&self, id: &str) -> bool {
        let id = id.to_ascii_lowercase();
        let id = if id.contains(':') {
            id
        } else {
            format!("minecraft:{id}")
        };
        (self.include.is_empty() || self.include.contains(&id)) && !self.exclude.contains(&id)
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// Where the rule was read from: rules are ordered by it, and refusals name it.
    pub path: String,
    pub method: Method,
    pub tiles: Vec<Tile>,
    /// Sprites (as texture resource paths) whose faces the rule replaces.
    pub match_tiles: Vec<String>,
    pub match_blocks: Vec<BlockSel>,
    pub weight: i32,
    /// [`faces`] bits.
    pub faces: u8,
    pub biomes: Option<Ids>,
    pub heights: Option<Ranges>,
    pub connect: Connect,
    /// Overlays: join only to faces with these sprites (texture resource paths) ...
    pub connect_tiles: Vec<String>,
    /// ... or these blocks.
    pub connect_blocks: Vec<BlockSel>,
    /// [`Connect::Properties`]: the state properties that must be equal.
    pub connect_properties: Vec<String>,
    /// Also require the block and each joining neighbour to hold the same mod-published
    /// per-position connection group (`LevelRef::set_connection_group`). Blocks without a group
    /// join nothing.
    pub connect_group: bool,
    pub inner_seams: bool,
    /// One per tile (`random`); empty when every tile counts the same.
    pub weights: Vec<u32>,
    pub random_loops: u32,
    pub symmetry: Symmetry,
    pub linked: bool,
    /// `repeat`'s pattern size, in blocks.
    pub width: u32,
    pub height: u32,
    /// `ctm_compact`'s `ctm.<case>=<tile index>`, by 47-tile case.
    pub compact_overrides: Vec<(u8, usize)>,
    /// Overlays: the tint index and the block whose tint it is (`None`: the face's block).
    pub tint_index: i32,
    pub tint_block: Option<String>,
    pub layer: Layer,
    pub orient: Orient,
    /// Applied before every rule without it.
    pub prioritize: bool,
    /// [`Method::Table`]'s sheet and table.
    pub table: Option<TableSheet>,
}

impl Rule {
    /// A rule that draws `sheet`'s tiles on every face of the blocks and sprites it is given,
    /// joining the same block.
    pub fn table(path: String, sheet: TableSheet) -> Self {
        Self {
            path,
            method: Method::Table,
            tiles: Vec::new(),
            match_tiles: Vec::new(),
            match_blocks: Vec::new(),
            weight: 0,
            faces: faces::ALL,
            biomes: None,
            heights: None,
            connect: Connect::Block,
            connect_tiles: Vec::new(),
            connect_blocks: Vec::new(),
            connect_properties: Vec::new(),
            connect_group: false,
            inner_seams: false,
            weights: Vec::new(),
            random_loops: 0,
            symmetry: Symmetry::None,
            linked: false,
            width: 0,
            height: 0,
            compact_overrides: Vec::new(),
            tint_index: -1,
            tint_block: None,
            layer: Layer::Cutout,
            orient: Orient::None,
            prioritize: false,
            table: Some(sheet),
        }
    }

    /// Whether the rule applies to `face_bit` (a [`faces`] bit) at altitude `y` in the biome
    /// `biome` returns. The biome is looked up only when the rule tests one.
    pub fn applies_at<'a>(
        &self,
        face_bit: u8,
        y: i32,
        biome: impl FnOnce() -> Option<&'a str>,
    ) -> bool {
        self.faces & face_bit != 0
            && self.heights.as_ref().is_none_or(|h| h.contains(y))
            && self
                .biomes
                .as_ref()
                .is_none_or(|ids| biome().is_some_and(|name| ids.matches(name)))
    }

    /// The rule names this block state (`state` answers its properties).
    pub fn matches_block<'a>(&self, id: &str, state: impl Fn(&str) -> Option<&'a str>) -> bool {
        self.match_blocks.iter().any(|m| m.matches(id, &state))
    }

    /// The rule names this sprite (a texture resource path).
    pub fn matches_tile(&self, sprite: &str) -> bool {
        self.match_tiles.iter().any(|t| t == sprite)
    }

    /// The order rules are tried in, smallest first: prioritized (tile) rules, then by weight
    /// (highest first), then by path.
    pub fn order(&self) -> (std::cmp::Reverse<bool>, std::cmp::Reverse<i32>, &str) {
        (
            std::cmp::Reverse(self.prioritize),
            std::cmp::Reverse(self.weight),
            &self.path,
        )
    }
}

/// `rules` in the order they are tried ([`Rule::order`]).
pub fn sort(rules: &mut [Rule]) {
    rules.sort_by(|a, b| a.order().cmp(&b.order()));
}
