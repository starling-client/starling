//! Connected textures at mesh time: OptiFine's (`optifine/ctm`) and mods'. The active rules are read
//! once at load, their tiles become sprite layers, and each block state and sprite layer gets
//! the short list of rules that can touch it; meshing a face then walks only those lists
//! ([`ConnectedTextures::pick`]). Without CTM files there are no rules and nothing changes.
//!
//! The rules and the tile geometry are `sunrise_connected`'s.

mod files;

use crate::resource_source::ResourceSource;
use crate::sprites::Sprites;
use std::collections::HashMap;
use sunrise_connected::{
    self as geometry, Axes, Axis, BlockSel, Connect, Face, Layer, Method, Orient, Rule, Tile,
};
use sunrise_world::block_state_properties::BlockProperties;
use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// The highest collision point of every block state of `registry`, by id, from the prepared
/// `cached-collision.nbt`: `f64::NEG_INFINITY` for an empty shape and for a state with no cached
/// shape, which is one whose shape depends on the world (`cached_shape` answers `None`: vanilla
/// asks the block). Such a state counts as having no shape, like air.
pub fn collision_heights(
    prepared: &std::path::Path,
    registry: &BlockRegistry,
) -> std::io::Result<Vec<f64>> {
    let file = sunrise_world::registry::BlockTableFile::open(
        &prepared.join("cached-collision.nbt"),
        registry,
    )?;
    heights(
        &sunrise_world::collision::CachedCollisionRegistry::from_table(file.table("collision")?)?,
    )
}

/// [`collision_heights`] of a collision table already read: the states a mod block declares are
/// in it with their declared shapes, as the content root rewrites the table.
fn heights(
    shapes: &sunrise_world::collision::CachedCollisionRegistry<'_>,
) -> std::io::Result<Vec<f64>> {
    (0..shapes.registry().len() as u32)
        .map(|id| {
            Ok(shapes
                .cached_shape(BlockStateId(id))?
                .map_or(f64::NEG_INFINITY, |s| {
                    s.max(sunrise_core::geometry::Axis::Y)
                }))
        })
        .collect()
}

/// The rules to load for `root`: those the installed pack formats read, then the mods' asset
/// files, in the order they are tried.
pub fn read_rules(root: &ResourceSource) -> Vec<Rule> {
    let mut rules = crate::pack_hooks::formats().connected(root);
    let files = files::read(root);
    if !files.is_empty() {
        rules.extend(files);
        geometry::sort(&mut rules);
    }
    rules
}

/// A tile as the mesher draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileLayer {
    Layer {
        layer: u16,
        /// The sprite has fractional alpha (drawn in the translucent pass).
        translucent: bool,
        /// Every texel is opaque (drawn without an alpha test).
        opaque: bool,
    },
    /// Try the next rule.
    Skip,
    /// The face's own sprite.
    Default,
}

/// What a face draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Its own sprite.
    Keep,
    Tile(TileLayer),
    /// `ctm_compact`: a tile per quarter of the sprite (top-left, top-right, bottom-left,
    /// bottom-right).
    Quarters([TileLayer; 4]),
    /// Nothing but its overlays: a Fusion face whose tile is empty, or of the overlay layout.
    Nothing,
}

/// A tile drawn over a face, after its own (`overlay*` methods).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub layer: u16,
    /// `layer=translucent`: drawn in the translucent pass, else with an alpha cutout.
    pub translucent: bool,
    /// `tintIndex` and the state whose tint it is (`tintBlock`, else the face's block).
    pub tint: Option<(i32, Option<u32>)>,
}

struct LoadedRule {
    rule: Rule,
    tiles: Vec<TileLayer>,
    /// `Method::Table`: the tiles of the sheet, by cell; `None` for a cell with nothing in it.
    cells: Vec<Option<TileLayer>>,
    /// `connect=Blocks`: the states of `connect_blocks` (sorted).
    listed: Vec<u32>,
    /// `Connect::Properties`: by state, which block and values it has (equal keys join).
    keys: Vec<u32>,
    /// By tile: the rules whose `matchTiles` name it (a range of `lists`), which chain on it.
    chain: Vec<(u32, u32)>,
    overlay: OverlayJoin,
}

/// What an overlay rule joins: `connectBlocks` as state ids and `connectTiles` as sprite
/// layers (sorted), and its tint.
#[derive(Default)]
struct OverlayJoin {
    states: Vec<u32>,
    tiles: Vec<u16>,
    tint: Option<(i32, Option<u32>)>,
}

impl OverlayJoin {
    fn any(&self) -> bool {
        !self.states.is_empty() || !self.tiles.is_empty()
    }
}

#[derive(Clone, Copy, Default)]
struct StateInfo {
    /// Index of the state's block (`connect=block`).
    block: u32,
    axis: Option<Axis>,
    /// `half=upper`: `linked` random tiles follow the lower half.
    upper: bool,
    /// A pane's arms (`north`, `east`, `south`, `west` true), as bits of [`ARMS`].
    arms: u8,
    /// Rules name the state or one of its sides' sprites ([`ConnectedTextures::stacked_pane_hides`]).
    has_rules: bool,
}

/// A pane's arm properties, in [`StateInfo::arms`]' bit order.
const ARMS: [&str; 4] = ["north", "east", "south", "west"];

#[derive(Default)]
pub struct ConnectedTextures {
    rules: Vec<LoadedRule>,
    /// By block state, then by sprite layer: ranges of `lists` (rule indices, in rule order).
    by_state: Vec<(u32, u32)>,
    by_layer: Vec<(u32, u32)>,
    lists: Vec<u16>,
    states: Vec<StateInfo>,
    /// By block state: the sprite of its model's face on each side (`connect=tile`); empty
    /// when no rule connects by tile.
    face_layers: Vec<[u16; 6]>,
    /// By block state: the highest point of its collision shape, for the rules that join by
    /// height ([`Self::set_heights`]); an empty shape has none.
    heights: Vec<f64>,
    /// True if any loaded rule has `connect_group: true`. Cached at build time so the mesher
    /// skips group-map checks completely when no rule uses connection groups.
    has_group_rules: bool,
}

/// `list` appended to `lists`, as its range there.
fn push_list(lists: &mut Vec<u16>, list: &[u16]) -> (u32, u32) {
    let start = lists.len() as u32;
    lists.extend_from_slice(list);
    (start, list.len() as u32)
}

/// A sprite layer as the mesher draws it.
fn tile_layer(sprites: &Sprites, layer: u32) -> crate::Result<TileLayer> {
    Ok(TileLayer::Layer {
        layer: u16::try_from(layer)?,
        translucent: sprites.translucent[layer as usize],
        opaque: sprites.opaque[layer as usize],
    })
}

/// The sprite name of a texture resource path: `ns:path` under `textures/`, else the path
/// itself (as [`Sprites::insert_file`] names a tile).
fn sprite_name(resource: &str) -> String {
    let parts = resource
        .strip_prefix("assets/")
        .and_then(|p| p.split_once('/'))
        .and_then(|(ns, p)| Some((ns, p.strip_prefix("textures/")?.strip_suffix(".png")?)));
    match parts {
        Some((ns, path)) => format!("{ns}:{path}"),
        None => resource.to_owned(),
    }
}

/// Lazy, statically dispatched samples for a face at the block's origin.
pub struct FaceNeighbours<State, Group, FullFaces, Biome, Texture> {
    /// State at the given relative offset.
    pub state: State,
    /// Published group at that offset, including the origin; only sampled by group rules.
    pub group: Group,
    /// Full-face mask for a sampled state.
    pub full_faces: FullFaces,
    /// Biome at the origin; sampled by rules that test one.
    pub biome: Biome,
    /// Quad texture axes; sampled by rules with `orient=texture`.
    pub texture: Texture,
}

#[derive(Clone, Copy)]
struct FaceSample {
    state: u32,
    layer: u16,
    face: Face,
    pos: [i32; 3],
}

impl ConnectedTextures {
    /// The sprites `rules` match by name (`matchTiles`, `connectTiles`), for
    /// [`Sprites::keep_apart`]: before the models insert theirs.
    pub fn matched_sprites(rules: &[Rule]) -> Vec<String> {
        rules
            .iter()
            .flat_map(|r| r.match_tiles.iter().chain(&r.connect_tiles))
            .map(|t| sprite_name(t))
            .collect()
    }

    /// Loaded rules with their tiles inserted into `sprites` (before it finishes).
    /// `face_layer` answers a block state's sprite on a side, for rules that connect by tile.
    /// `is_pane` says which states are pane-shaped (`StateFacts::is_pane`).
    pub fn load(
        _root: &ResourceSource,
        mut rules: Vec<Rule>,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        mut face_layer: impl FnMut(BlockStateId, Face) -> Option<u32>,
        is_pane: impl Fn(BlockStateId) -> bool,
    ) -> Self {
        // Rule indices are `u16`.
        rules.truncate(usize::from(u16::MAX));
        if rules.is_empty() {
            return Self::default();
        }
        // The tiles decode side by side; the loop below inserts them in rule order, under the
        // same names.
        let tiles: Vec<String> = rules
            .iter()
            .flat_map(|rule| &rule.tiles)
            .filter_map(|tile| match tile {
                Tile::Texture(path) => Some(sprite_name(path)),
                _ => None,
            })
            .collect();
        sprites.prefetch(tiles.iter().map(String::as_str));
        // A sheet shared by rules is cut once: by sprite and grid.
        let mut sheets: HashMap<(String, (u32, u32)), crate::Result<Vec<Option<TileLayer>>>> =
            HashMap::new();
        let loaded = rules
            .into_iter()
            .filter_map(|rule| {
                let tiles = rule
                    .tiles
                    .iter()
                    .map(|tile| match tile {
                        Tile::Skip => Ok(TileLayer::Skip),
                        Tile::Default => Ok(TileLayer::Default),
                        Tile::Texture(path) => {
                            // A tile among the textures is that sprite, under its own name.
                            let layer = match sprite_name(path) {
                                name if name != *path => sprites.insert(&name)?,
                                _ => sprites.insert_file(path)?,
                            };
                            tile_layer(sprites, layer)
                        }
                    })
                    .collect::<crate::Result<Vec<_>>>();
                let tiles = match tiles {
                    Ok(tiles) => tiles,
                    Err(e) => {
                        log::warn!("Sunrise: connected textures {}: a tile: {e}", rule.path);
                        return None;
                    }
                };
                // A table's sheet, cut into one sprite per cell.
                let cells = match &rule.table {
                    None => Vec::new(),
                    Some(sheet) => {
                        let key = (sheet.sprite.clone(), sheet.table.grid());
                        let cut = sheets.entry(key).or_insert_with_key(|(sprite, grid)| {
                            sprites
                                .insert_tiles(sprite, *grid, false)?
                                .into_iter()
                                .map(|l| l.map(|l| tile_layer(sprites, l)).transpose())
                                .collect()
                        });
                        match cut {
                            Ok(cells) => cells.clone(),
                            Err(e) => {
                                log::warn!(
                                    "Sunrise: connected textures {}: sheet {}: {e}",
                                    rule.path,
                                    sheet.sprite
                                );
                                return None;
                            }
                        }
                    }
                };
                Some(LoadedRule {
                    rule,
                    tiles,
                    cells,
                    listed: Vec::new(),
                    keys: Vec::new(),
                    chain: Vec::new(),
                    overlay: OverlayJoin::default(),
                })
            })
            .collect();
        let states = (0..registry.len() as u32).map(|id| {
            registry
                .state(BlockStateId(id))
                .map(|s| (s.name.as_str(), &s.properties))
        });
        let mut out = Self::build(loaded, states, |name| sprites.model_layer(name));
        if out
            .rules
            .iter()
            .any(|r| r.rule.connect == Connect::Tile || !r.overlay.tiles.is_empty())
        {
            out.face_layers = (0..registry.len() as u32)
                .map(|id| {
                    Face::ALL
                        .map(|f| face_layer(BlockStateId(id), f).map_or(u16::MAX, |l| l as u16))
                })
                .collect();
        }
        out.connect_panes(face_layer, is_pane);
        out
    }

    /// Panes whose sprites tile rules name (`matchTiles=glass`) are connected too, by a rule that
    /// can draw on them: here, one naming no block (a rule naming the pane is in its own list
    /// already). `is_pane` says which states are panes, whatever their name.
    fn connect_panes(
        &mut self,
        mut face_layer: impl FnMut(BlockStateId, Face) -> Option<u32>,
        is_pane: impl Fn(BlockStateId) -> bool,
    ) {
        for id in (0..self.states.len()).filter(|_| !self.by_layer.is_empty()) {
            if is_pane(BlockStateId(id as u32)) && !self.states[id].has_rules {
                self.states[id].has_rules = Face::ALL.into_iter().any(|f| {
                    face_layer(BlockStateId(id as u32), f).is_some_and(|l| {
                        self.list(&self.by_layer, l as usize)
                            .iter()
                            .any(|&i| self.rules[usize::from(i)].rule.match_blocks.is_empty())
                    })
                });
            }
        }
    }

    /// The lookups for `rules` (already ordered) over every block state (`None` for an id the
    /// registry skips), `layer` answering a sprite name's layer.
    fn build<'a>(
        rules: Vec<LoadedRule>,
        states: impl Iterator<Item = Option<(&'a str, &'a BlockProperties)>>,
        layer: impl Fn(&str) -> Option<u32>,
    ) -> Self {
        let mut out = Self {
            rules,
            ..Self::default()
        };
        let states: Vec<_> = states.collect();
        // Each block's states, for `connectBlocks` and `tintBlock`.
        let mut block_states: HashMap<&str, Vec<u32>> = HashMap::new();
        if out.rules.iter().any(|r| {
            r.rule.method.is_overlay()
                || matches!(r.rule.connect, Connect::Blocks | Connect::Height(_))
        }) {
            for (id, state) in states.iter().enumerate() {
                if let Some((name, _)) = state {
                    block_states.entry(name).or_default().push(id as u32);
                }
            }
        }
        // The sorted states some entries name.
        let listed = |entries: &[BlockSel]| {
            let mut ids = Vec::new();
            for m in entries {
                let candidates = block_states
                    .get(m.id.as_str())
                    .map_or(&[][..], Vec::as_slice);
                ids.extend(candidates.iter().filter(|&&id| {
                    states[id as usize].is_some_and(|(name, props)| {
                        m.matches(name, |k| props.get(k).map(String::as_str))
                    })
                }));
            }
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        for r in out.rules.iter_mut() {
            match r.rule.connect {
                Connect::Blocks | Connect::Height(_) => r.listed = listed(&r.rule.connect_blocks),
                Connect::Properties => {
                    // States are alike when they are one block with the same values of the
                    // properties the rule names.
                    let mut keys = HashMap::new();
                    r.keys = states
                        .iter()
                        .map(|state| {
                            let Some((name, props)) = state else {
                                return u32::MAX;
                            };
                            let values: Vec<_> = r
                                .rule
                                .connect_properties
                                .iter()
                                .map(|k| props.get(k).map(String::as_str))
                                .collect();
                            let next = keys.len() as u32;
                            *keys.entry((*name, values)).or_insert(next)
                        })
                        .collect();
                }
                _ => {}
            }
        }
        for r in out.rules.iter_mut().filter(|r| r.rule.method.is_overlay()) {
            let rule = &r.rule;
            // `connectBlocks` and `connectTiles` are "only for method overlay".
            if rule.method == Method::Overlay {
                r.overlay.states = listed(&rule.connect_blocks);
                r.overlay.tiles = rule
                    .connect_tiles
                    .iter()
                    .filter_map(|t| layer(&sprite_name(t)).map(|l| l as u16))
                    .collect();
                r.overlay.tiles.sort_unstable();
            }
            // A `tintBlock` the game doesn't have tints nothing.
            let tint = match rule.tint_block.as_deref() {
                None => Some(None),
                Some(block) => block_states.get(block).map(|ids| Some(ids[0])),
            };
            r.overlay.tint = tint
                .filter(|_| rule.tint_index >= 0)
                .map(|b| (rule.tint_index, b));
        }
        let mut blocks = HashMap::new();
        let mut by_layer: Vec<Vec<u16>> = Vec::new();
        // Block rules by the block each entry names, so a state tests only its own.
        let mut by_block: HashMap<&str, Vec<u16>> = HashMap::new();
        // Tile rules by the sprite or tile file each names, for chaining on a picked tile.
        let mut by_name: HashMap<String, Vec<u16>> = HashMap::new();
        for (i, r) in out.rules.iter().enumerate() {
            let i = i as u16;
            for tile in &r.rule.match_tiles {
                let name = sprite_name(tile);
                let list = by_name.entry(name.clone()).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
                // A tile file (`./1.png`) is no model's sprite: only chaining reaches it, by
                // name. By layer it would match every face whose sprite has the same pixels
                // (identical stills share a layer: Conquest's blank placeholders, dozens).
                if !name.contains(':') {
                    continue;
                }
                if let Some(l) = layer(&name) {
                    let l = l as usize;
                    if by_layer.len() <= l {
                        by_layer.resize(l + 1, Vec::new());
                    }
                    if by_layer[l].last() != Some(&i) {
                        by_layer[l].push(i);
                    }
                }
            }
            for m in &r.rule.match_blocks {
                let list = by_block.entry(m.id.as_str()).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
        for list in by_layer {
            let range = push_list(&mut out.lists, &list);
            out.by_layer.push(range);
        }
        let by_name: HashMap<String, (u32, u32)> = by_name
            .into_iter()
            .map(|(name, list)| (name, push_list(&mut out.lists, &list)))
            .collect();
        // Set at the end: `by_block` borrows the rules' names.
        let chains: Vec<Vec<(u32, u32)>> = out
            .rules
            .iter()
            .map(|r| {
                r.rule
                    .tiles
                    .iter()
                    .map(|t| match t {
                        Tile::Texture(path) => by_name.get(&sprite_name(path)).copied(),
                        _ => None,
                    })
                    .map(Option::unwrap_or_default)
                    .collect()
            })
            .collect();
        let mut list = Vec::new();
        for state in states {
            let Some((name, props)) = state else {
                out.by_state.push((0, 0));
                out.states.push(StateInfo::default());
                continue;
            };
            let next = blocks.len() as u32;
            let block = *blocks.entry(name).or_insert(next);
            let prop = |k: &str| props.get(k).map(String::as_str);
            out.states.push(StateInfo {
                block,
                axis: match prop("axis") {
                    Some("x") => Some(Axis::X),
                    Some("y") => Some(Axis::Y),
                    Some("z") => Some(Axis::Z),
                    _ => None,
                },
                upper: prop("half") == Some("upper"),
                arms: ARMS
                    .iter()
                    .enumerate()
                    .filter(|&(_, arm)| prop(arm) == Some("true"))
                    .fold(0, |bits, (i, _)| bits | 1 << i),
                // Set below, with the state's rule list.
                has_rules: false,
            });
            list.clear();
            if let Some(candidates) = by_block.get(name) {
                list.extend(
                    candidates
                        .iter()
                        .filter(|&&i| out.rules[usize::from(i)].rule.matches_block(name, prop)),
                );
            }
            out.states.last_mut().expect("pushed above").has_rules = !list.is_empty();
            let range = push_list(&mut out.lists, &list);
            out.by_state.push(range);
        }
        for (r, chain) in out.rules.iter_mut().zip(chains) {
            r.chain = chain;
        }
        out.has_group_rules = out.rules.iter().any(|r| r.rule.connect_group);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Whether any loaded rule checks per-position mod connection groups (`connect_group`).
    #[inline]
    pub fn has_group_rules(&self) -> bool {
        self.has_group_rules
    }

    /// Whether a rule joins by height, and the collision heights are needed.
    pub fn joins_by_height(&self) -> bool {
        self.rules
            .iter()
            .any(|r| matches!(r.rule.connect, Connect::Height(_)))
    }

    /// The highest collision point of every block state, by id (`f64::NEG_INFINITY` for a
    /// state without a collision shape). Read when meshing: nothing is worked out per face.
    pub fn set_heights(&mut self, heights: Vec<f64>) {
        self.heights = heights;
    }

    fn height(&self, state: u32) -> f64 {
        self.heights
            .get(state as usize)
            .copied()
            .unwrap_or(f64::NEG_INFINITY)
    }

    /// Whether a pane's top or bottom strip, centred at `centre` (block-local x, z), is hidden by
    /// the pane `other` stacked on it (Continuity's "Glass Pane Culling Fix"): only for a pane
    /// with connected-texture rules, under or over the same block, where `other` has the same
    /// part (the post, or the arm toward the strip).
    pub fn stacked_pane_hides(&self, state: u32, other: u32, [x, z]: [f32; 2]) -> bool {
        let (Some(own), Some(theirs)) = (
            self.states.get(state as usize),
            self.states.get(other as usize),
        ) else {
            return false;
        };
        if own.block != theirs.block || !own.has_rules {
            return false;
        }
        let (dx, dz) = (x - 0.5, z - 0.5);
        // The post is 2 pixels wide; an arm runs from it to the block's edge.
        if dx.abs() <= 1.0 / 16.0 && dz.abs() <= 1.0 / 16.0 {
            return true;
        }
        let arm = if dz.abs() > dx.abs() {
            if dz < 0.0 { 0 } else { 2 }
        } else if dx > 0.0 {
            1
        } else {
            3
        };
        theirs.arms & 1 << arm != 0
    }

    fn list(&self, ranges: &[(u32, u32)], i: usize) -> &[u16] {
        ranges.get(i).map_or(&[], |&range| self.range(range))
    }

    fn range(&self, (start, len): (u32, u32)) -> &[u16] {
        &self.lists[start as usize..(start + len) as usize]
    }

    /// What the `face` of the block `state` at `pos` draws, for a quad with sprite `layer`.
    pub fn pick<'b>(
        &self,
        state: u32,
        layer: u16,
        face: Face,
        pos: [i32; 3],
        neighbours: FaceNeighbours<
            impl Fn([i32; 3]) -> u32,
            impl Fn([i32; 3]) -> Option<u64>,
            impl Fn(u32) -> u8,
            impl Fn() -> Option<&'b str>,
            impl Fn() -> Option<Axes>,
        >,
        mut overlay: impl FnMut(Overlay),
    ) -> Pick {
        let sample = FaceSample {
            state,
            layer,
            face,
            pos,
        };
        let (mut by_state, mut by_layer) = (
            self.list(&self.by_state, state as usize),
            self.list(&self.by_layer, layer as usize),
        );
        // Both lists are in rule order: walk them as one. A rule naming blocks and tiles needs
        // both (it is in both lists); one naming either needs that one.
        loop {
            let (i, block, tile) = match (by_state.split_first(), by_layer.split_first()) {
                (None, None) => return Pick::Keep,
                (Some((&x, rest)), Some((&y, other))) if x == y => {
                    (by_state, by_layer) = (rest, other);
                    (x, true, true)
                }
                (Some((&x, rest)), Some((&y, _))) if x < y => {
                    by_state = rest;
                    (x, true, false)
                }
                (Some((&x, rest)), None) => {
                    by_state = rest;
                    (x, true, false)
                }
                (_, Some((&y, rest))) => {
                    by_layer = rest;
                    (y, false, true)
                }
            };
            let r = &self.rules[usize::from(i)];
            if (block || r.rule.match_blocks.is_empty())
                && (tile || r.rule.match_tiles.is_empty())
                && let Some((pick, index)) = self.apply(r, sample, &neighbours, &mut overlay)
            {
                // "Tiles output by CTM rules can also be matched by another rule": once, by the
                // tile rules naming the picked tile by name (and this block, if they name blocks).
                // A tile on the face's own sprite (`tiles=stone`) is no new tile.
                let Pick::Tile(TileLayer::Layer { layer: picked, .. }) = pick else {
                    return pick;
                };
                if picked == layer {
                    return pick;
                }
                let own = self.list(&self.by_state, state as usize);
                // Rules tried on the face's sprite already (those before this one) are not tried
                // again: an overlay would draw twice.
                let first = self.list(&self.by_layer, layer as usize);
                let again = self
                    .range(r.chain.get(index).copied().unwrap_or_default())
                    .iter()
                    .map(|&j| (j, &self.rules[usize::from(j)]))
                    .filter(|&(j, r)| {
                        j != i
                            && !(j < i && first.contains(&j))
                            && (r.rule.match_blocks.is_empty() || own.contains(&j))
                    });
                for (_, r) in again {
                    // Joins by tile still compare the face's own sprite with its neighbours'.
                    match self.apply(r, sample, &neighbours, &mut overlay) {
                        Some((Pick::Keep, _)) => return pick,
                        Some((chained, _)) => return chained,
                        None => {}
                    }
                }
                return pick;
            }
        }
    }

    /// One rule on a face: `None` when it doesn't apply, picks `<skip>` or draws an overlay
    /// (handed to `overlay`: the rules after it still choose the face's own sprite).
    fn apply<'b>(
        &self,
        r: &LoadedRule,
        sample: FaceSample,
        neighbours: &FaceNeighbours<
            impl Fn([i32; 3]) -> u32,
            impl Fn([i32; 3]) -> Option<u64>,
            impl Fn(u32) -> u8,
            impl Fn() -> Option<&'b str>,
            impl Fn() -> Option<Axes>,
        >,
        overlay: &mut impl FnMut(Overlay),
    ) -> Option<(Pick, usize)> {
        let FaceSample {
            state,
            layer,
            face,
            pos,
        } = sample;
        let FaceNeighbours {
            state: neighbour,
            group,
            full_faces,
            biome,
            texture,
        } = neighbours;
        let rule = &r.rule;
        // `Rule::applies_at`, with the biome looked up only for rules that test one. Rules with
        // a custom name are not loaded.
        if !rule.applies_at(face.bit(), pos[1], biome) {
            return None;
        }
        let info = self.states.get(state as usize).copied().unwrap_or_default();
        let join = &r.overlay;
        let own_group = if rule.connect_group {
            group([0, 0, 0])
        } else {
            None
        };
        let joins = |offset: [i32; 3]| {
            let other = neighbour(offset);
            let base = if join.any() {
                // An overlay joins the blocks it is drawn beside.
                join.states.binary_search(&other).is_ok()
                    || self
                        .face_layers
                        .get(other as usize)
                        .is_some_and(|f| join.tiles.binary_search(&f[face as usize]).is_ok())
            } else {
                match rule.connect {
                    Connect::Block => {
                        self.states.get(other as usize).map(|s| s.block) == Some(info.block)
                    }
                    Connect::State => other == state,
                    Connect::Blocks => r.listed.binary_search(&other).is_ok(),
                    Connect::Height(h) => {
                        let own = self.height(state);
                        let at = |cell: [i32; 3]| {
                            let above = self.height(neighbour([cell[0], cell[1] + 1, cell[2]]));
                            (self.height(neighbour(cell)), above)
                        };
                        let (height, above) = at(offset);
                        let listed = r.rule.connect_blocks.is_empty()
                            || r.listed.binary_search(&other).is_ok();
                        h.joins(own, height, above, listed)
                            || (h.side
                                && geometry::beside(face.normal(), offset).is_some_and(|beside| {
                                    beside.into_iter().any(|cell| {
                                        let (height, above) = at(cell);
                                        h.joins_beside(own, height, above)
                                    })
                                }))
                    }
                    Connect::Properties => r
                        .keys
                        .get(other as usize)
                        .is_some_and(|&k| k != u32::MAX && Some(&k) == r.keys.get(state as usize)),
                    Connect::Tile => self
                        .face_layers
                        .get(other as usize)
                        .is_some_and(|f| f[face as usize] == layer),
                }
            };
            if !base {
                return false;
            }
            if rule.connect_group {
                let Some(own) = own_group else {
                    return false;
                };
                return group(offset) == Some(own);
            }
            true
        };
        let axes = |face| match (rule.orient, info.axis) {
            (Orient::StateAxis, Some(axis)) => Axes::column(face, axis),
            // The quad's own texture directions, whatever face `symmetry` stands it for.
            // Not for `overlay`, which Continuity's `orient` leaves out: its 17 tiles always face
            // the standard way.
            (Orient::Texture, _) if rule.method != Method::Overlay => {
                texture().unwrap_or(Axes::cube(face))
            }
            _ => Axes::cube(face),
        };
        let mask = || geometry::neighbours(axes(face), face.normal(), rule.inner_seams, joins);
        let tile = |i: usize| r.tiles.get(i).copied().unwrap_or(TileLayer::Default);
        let mut draw = |t: TileLayer| {
            if let TileLayer::Layer { layer, .. } = t {
                overlay(Overlay {
                    layer,
                    translucent: rule.layer == Layer::Translucent,
                    tint: join.tint,
                });
            }
        };
        // An overlay picks its tile as the method it is the overlay of.
        let method = match rule.method {
            Method::Overlay => {
                // The 17 transition tiles, toward the blocks it names (none: it draws nothing).
                if join.any() {
                    let mask = geometry::neighbours(axes(face), face.normal(), false, joins);
                    geometry::overlay_tiles(mask).for_each(|i| draw(tile(i)));
                }
                return None;
            }
            Method::OverlayCtm => Method::Ctm,
            Method::OverlayRandom => Method::Random,
            Method::OverlayRepeat => Method::Repeat,
            Method::OverlayFixed => Method::Fixed,
            Method::OverlayHorizontal => Method::Horizontal,
            Method::OverlayVertical => Method::Vertical,
            Method::OverlayHorizontalVertical => Method::HorizontalVertical,
            Method::OverlayVerticalHorizontal => Method::VerticalHorizontal,
            m => m,
        };
        let index = match method {
            Method::Table => {
                let sheet = rule.table.as_ref()?;
                let axes = axes(face);
                let normal = face.normal();
                // Only the neighbours the table tests are probed, the cover check included. With
                // `cover`, a neighbour does not join when the block in front of it, on the
                // face's side, joins too and has a full face toward it.
                let tested = sheet.table.tested();
                let toward = face.opposite().bit();
                let mut mask = 0;
                for (bit, offset) in geometry::offsets(axes) {
                    if tested & bit == 0 || !joins(offset) {
                        continue;
                    }
                    let front = [0, 1, 2].map(|k| offset[k] + normal[k]);
                    if sheet.table.cover()
                        && joins(front)
                        && full_faces(neighbour(front)) & toward != 0
                    {
                        continue;
                    }
                    mask |= bit;
                }
                return Some(
                    match r.cells.get(sheet.table.tile(mask)).copied().flatten() {
                        Some(tile) => (Pick::Tile(tile), 0),
                        None => (Pick::Nothing, 0),
                    },
                );
            }
            Method::Ctm => geometry::ctm_index(mask()),
            Method::CtmCompact => {
                let mask = mask();
                let case = geometry::ctm_index(mask) as u8;
                match rule.compact_overrides.iter().find(|&&(c, _)| c == case) {
                    Some(&(_, t)) => t,
                    None => {
                        let quarters = geometry::compact_quarters(mask).map(|q| tile(q.into()));
                        return if quarters.contains(&TileLayer::Skip) {
                            None
                        } else {
                            Some((Pick::Quarters(quarters), 0))
                        };
                    }
                }
            }
            Method::Horizontal => geometry::horizontal(mask()),
            Method::Vertical => geometry::vertical(mask()),
            Method::HorizontalVertical => geometry::horizontal_vertical(mask()),
            Method::VerticalHorizontal => geometry::vertical_horizontal(mask()),
            Method::Top => {
                // Only sides join the block above: the top and bottom go on to the next rule.
                if matches!(face, Face::Up | Face::Down) {
                    return None;
                }
                if !joins([0, 1, 0]) {
                    return Some((Pick::Keep, 0));
                }
                0
            }
            Method::Fixed => 0,
            Method::Random => {
                let face = geometry::symmetric_face(face, rule.symmetry);
                let pos = if rule.linked && info.upper {
                    [pos[0], pos[1] - 1, pos[2]]
                } else {
                    pos
                };
                geometry::random_index(pos, face, rule.random_loops, &rule.weights, r.tiles.len())
            }
            Method::Repeat => {
                let face = geometry::symmetric_face(face, rule.symmetry);
                geometry::repeat_index(pos, axes(face), rule.width, rule.height)
            }
            // Every overlay was mapped to its method above.
            _ => return None,
        };
        if rule.method.is_overlay() {
            draw(tile(index));
            return None;
        }
        match tile(index) {
            TileLayer::Skip => None,
            TileLayer::Default => Some((Pick::Keep, index)),
            t => Some((Pick::Tile(t), index)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_connected::{Table, TableSheet};
    use sunrise_optifine::Properties;
    use sunrise_optifine::ctm::{self, Rule};

    /// A tile whose layer says which tile of its rule it is: rule `r`'s tile `i` is layer
    /// `1000 * (r + 1) + i`.
    fn rules(files: &[(&str, &str)]) -> Vec<LoadedRule> {
        let mut rules: Vec<Rule> = files
            .iter()
            .map(|(path, text)| {
                Rule::parse(
                    &format!("assets/minecraft/optifine/ctm/{path}"),
                    &Properties::parse(text),
                )
                .unwrap()
                .unwrap()
            })
            .collect();
        ctm::sort(&mut rules);
        rules
            .into_iter()
            .enumerate()
            .map(|(n, rule)| LoadedRule {
                cells: Vec::new(),
                listed: Vec::new(),
                keys: Vec::new(),
                overlay: OverlayJoin::default(),
                chain: Vec::new(),
                tiles: rule
                    .tiles
                    .iter()
                    .enumerate()
                    .map(|(i, t)| match t {
                        Tile::Skip => TileLayer::Skip,
                        Tile::Default => TileLayer::Default,
                        Tile::Texture(_) => TileLayer::Layer {
                            layer: (1000 * (n + 1) + i) as u16,
                            translucent: false,
                            opaque: false,
                        },
                    })
                    .collect(),
                rule: rule.rule,
            })
            .collect()
    }

    /// State 0 air, 1 glass, 2 stone, 3 and 4 oak logs (axis y, x), 5 and 6 a door (lower,
    /// upper); the stone sprite is layer 7, glass 3.
    fn world(files: &[(&str, &str)]) -> ConnectedTextures {
        let props = |pairs: &[(&str, &str)]| -> BlockProperties {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let states = [
            ("minecraft:air", props(&[])),
            ("minecraft:glass", props(&[])),
            ("minecraft:stone", props(&[])),
            ("minecraft:oak_log", props(&[("axis", "y")])),
            ("minecraft:oak_log", props(&[("axis", "x")])),
            ("minecraft:oak_door", props(&[("half", "lower")])),
            ("minecraft:oak_door", props(&[("half", "upper")])),
        ];
        ConnectedTextures::build(
            rules(files),
            states.iter().map(|(n, p)| Some((*n, p))),
            |name| match name {
                "minecraft:block/stone" => Some(7),
                "minecraft:block/glass" => Some(3),
                // The tile `b0` of a rule second in order (`a_picked_tile_can_be_picked_again`).
                "assets/minecraft/optifine/ctm/b0.png" => Some(2000),
                // And `a0` of the first rule.
                "assets/minecraft/optifine/ctm/a0.png" => Some(1000),
                // Two blank placeholders with the same pixels, so one layer: the tile `grass/1`
                // of the third rule (`chaining_follows_the_tile_not_its_pixels`).
                "assets/minecraft/optifine/ctm/grass/1.png"
                | "assets/minecraft/optifine/ctm/aaa/1.png" => Some(3000),
                _ => None,
            },
        )
    }

    fn tile(layer: u16) -> Pick {
        Pick::Tile(TileLayer::Layer {
            layer,
            translucent: false,
            opaque: false,
        })
    }

    /// `pick` on `face` of the block at the origin among `blocks` (offset, state).
    fn pick_in(
        ctm: &ConnectedTextures,
        state: u32,
        layer: u16,
        face: Face,
        blocks: &[([i32; 3], u32)],
    ) -> Pick {
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        // Glass is a full cube; stone, here, is not.
        let full = |s: u32| if s == 1 { 0x3f } else { 0 };
        ctm.pick(
            state,
            layer,
            face,
            [0, 64, 0],
            FaceNeighbours {
                state: at,
                group: |_| None,
                full_faces: full,
                biome: || None,
                texture: || None,
            },
            |_| {},
        )
    }

    /// Glass (state 1) with a `table` rule of `tests` over a sheet whose cell `i` is layer
    /// `500 + i` (`cells[0]` is empty when `clear`), joining `connect`.
    fn table_world(
        table: Table,
        clear: bool,
        connect: Connect,
        connect_blocks: Vec<BlockSel>,
    ) -> ConnectedTextures {
        table_rule_world(
            table,
            clear,
            "minecraft:glass",
            connect,
            connect_blocks,
            &[],
        )
    }

    fn table_rule_world(
        table: Table,
        clear: bool,
        block: &str,
        connect: Connect,
        connect_blocks: Vec<BlockSel>,
        connect_properties: &[&str],
    ) -> ConnectedTextures {
        let (columns, rows) = table.grid();
        let mut rule = sunrise_connected::Rule::table(
            "assets/demo/sunrise/connected/glass.json".to_owned(),
            TableSheet {
                sprite: "demo:block/sheet".to_owned(),
                table: std::sync::Arc::new(table),
            },
        );
        let block_id = block;
        let block = |id: &str| BlockSel {
            id: id.to_owned(),
            states: Vec::new(),
        };
        rule.match_blocks = vec![block(block_id)];
        rule.connect_properties = connect_properties.iter().map(|k| k.to_string()).collect();
        rule.connect = connect;
        rule.connect_blocks = connect_blocks;
        let cells = (0..columns * rows)
            .map(|i| {
                (!(clear && i == 0)).then_some(TileLayer::Layer {
                    layer: 500 + i as u16,
                    translucent: false,
                    opaque: false,
                })
            })
            .collect();
        let rules = vec![LoadedRule {
            rule,
            tiles: Vec::new(),
            cells,
            listed: Vec::new(),
            keys: Vec::new(),
            chain: Vec::new(),
            overlay: OverlayJoin::default(),
        }];
        let none = BlockProperties::default();
        let axis = |v: &str| BlockProperties::from([("axis".to_owned(), v.to_owned())]);
        let (y, x) = (axis("y"), axis("x"));
        let states = [
            ("minecraft:air", &none),
            ("minecraft:glass", &none),
            ("minecraft:stone", &none),
            ("minecraft:oak_log", &y),
            ("minecraft:oak_log", &x),
        ];
        ConnectedTextures::build(rules, states.iter().map(|s| Some(*s)), |_| None)
    }

    #[test]
    fn a_table_reads_its_tile_off_the_masks_of_its_tests() {
        use geometry::{L, R};
        // Left and right: the outcome is left + 2 * right; its tiles, by outcome: 3 alone, 2 left
        // only, 0 right only, 1 both. A north face's right is west, its left east.
        let row = |cover| Table::new(vec![L, R], cover, (4, 1), vec![3, 2, 0, 1]).unwrap();
        let ctm = table_world(row(false), false, Connect::Block, Vec::new());
        let pick = |blocks: &[([i32; 3], u32)]| pick_in(&ctm, 1, 3, Face::North, blocks);
        assert_eq!(pick(&[]), tile(503));
        assert_eq!(pick(&[([-1, 0, 0], 1)]), tile(500));
        assert_eq!(pick(&[([1, 0, 0], 1)]), tile(502));
        assert_eq!(pick(&[([-1, 0, 0], 1), ([1, 0, 0], 1)]), tile(501));
        // Stone is another block; the glass above and below is not tested; the other faces
        // read their own directions (the south face's right is east).
        assert_eq!(pick(&[([-1, 0, 0], 2), ([0, 1, 0], 1)]), tile(503));
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::South, &[([1, 0, 0], 1)]),
            tile(500)
        );
        // A neighbour with a glass block in front of it (north) does not join with `cover`.
        let covered = [([-1, 0, 0], 1), ([-1, 0, -1], 1)];
        assert_eq!(pick(&covered), tile(500));
        let ctm = table_world(row(true), false, Connect::Block, Vec::new());
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &covered), tile(503));
        // A front block that joins but has no full face toward the neighbour does not cover it.
        let stone = BlockSel {
            id: "minecraft:stone".to_owned(),
            states: Vec::new(),
        };
        let ctm = table_world(row(true), false, Connect::Blocks, vec![stone]);
        let front = [([-1, 0, 0], 2), ([-1, 0, -1], 2)];
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &front), tile(500));
        // An empty cell draws nothing.
        let ctm = table_world(row(false), true, Connect::Block, Vec::new());
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            Pick::Nothing
        );
    }

    /// Glass (state 1, a block 0.5 high) with a rule joining by `join`, the heights of states
    /// 0 to 4 literal: air has none, glass 0.5, stone 1.0, the logs 0.5 and 0.25. The north
    /// face's right is west: its one test joins the cell at `[-1, 0, 0]`.
    fn height_world(join: sunrise_connected::HeightJoin, of: &[&str]) -> ConnectedTextures {
        let table = Table::new(vec![geometry::R], false, (2, 1), vec![1, 0]).unwrap();
        let of = of
            .iter()
            .map(|id| BlockSel {
                id: id.to_string(),
                states: Vec::new(),
            })
            .collect();
        let mut ctm = table_world(table, false, Connect::Height(join), of);
        ctm.set_heights(vec![f64::NEG_INFINITY, 0.5, 1.0, 0.5, 0.25]);
        ctm
    }

    /// Whether the rule joins the cell west of the glass block, among `blocks`.
    fn joins_west(ctm: &ConnectedTextures, blocks: &[([i32; 3], u32)]) -> bool {
        match pick_in(ctm, 1, 3, Face::North, blocks) {
            p if p == tile(500) => true,
            p if p == tile(501) => false,
            p => panic!("{p:?}"),
        }
    }

    /// A mod block's state whose declared collision is a box 0.5 high, in the table the content
    /// root writes: its height is 0.5, and air's is none.
    #[test]
    fn a_mod_blocks_declared_collision_top_is_its_height() {
        use sunrise_core::nbt::Tag;
        let names = || {
            Tag::List(vec![
                Tag::String("minecraft:air".into()),
                Tag::String("demo:slab".into()),
            ])
        };
        let version = || Tag::Int(sunrise_core::version::DATA_VERSION);
        let registry = sunrise_world::registry::BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", version()),
            ("states", names()),
            (
                "defaults",
                Tag::compound([("minecraft:air", Tag::Int(0)), ("demo:slab", Tag::Int(1))]),
            ),
        ]))
        .unwrap();
        let bits =
            |values: &[f64]| Tag::LongArray(values.iter().map(|v| v.to_bits() as i64).collect());
        let half = Tag::compound([
            ("canonical", Tag::Byte(0)),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("x", bits(&[0.0, 1.0])),
            ("y", bits(&[0.0, 0.5])),
            ("z", bits(&[0.0, 1.0])),
            ("occupied", Tag::ByteArray(vec![1])),
            ("discrete_min", Tag::IntArray(vec![0, 0, 0])),
            ("discrete_max", Tag::IntArray(vec![1, 1, 1])),
        ]);
        let table = sunrise_world::collision::CachedCollisionRegistry::from_nbt(
            &Tag::compound([
                ("DataVersion", version()),
                ("states", names()),
                ("shapes", Tag::List(vec![half])),
                ("cached_shapes", Tag::IntArray(vec![-1, 0])),
            ]),
            &registry,
        )
        .unwrap();
        assert_eq!(heights(&table).unwrap(), [f64::NEG_INFINITY, 0.5]);
    }

    #[test]
    fn a_table_can_join_a_neighbour_of_the_same_height() {
        let same = sunrise_connected::HeightJoin {
            same: true,
            ..Default::default()
        };
        let ctm = height_world(same, &[]);
        // Glass and the first log are 0.5 high; stone 1.0, the second log 0.25, air has none.
        assert!(joins_west(&ctm, &[([-1, 0, 0], 1)]));
        assert!(joins_west(&ctm, &[([-1, 0, 0], 3)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 2)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 4)]));
        assert!(!joins_west(&ctm, &[]));
        // `of` limits the blocks it compares: only the logs.
        let ctm = height_world(same, &["minecraft:oak_log"]);
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 1)]));
        assert!(joins_west(&ctm, &[([-1, 0, 0], 3)]));
    }

    #[test]
    fn a_table_can_join_a_higher_or_a_lower_neighbour() {
        let higher = sunrise_connected::HeightJoin {
            higher: true,
            ..Default::default()
        };
        // A block 0.5 high beside a 1.0 high one (Create's bottom slab beside a taller block).
        let ctm = height_world(higher, &[]);
        assert!(joins_west(&ctm, &[([-1, 0, 0], 2)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 1)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 4)]));
        // Only blocks with a collision top are higher than this one: air is not.
        assert!(!joins_west(&ctm, &[]));
        let lower = sunrise_connected::HeightJoin {
            lower: true,
            ..Default::default()
        };
        let ctm = height_world(lower, &[]);
        assert!(joins_west(&ctm, &[([-1, 0, 0], 4)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 1)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 2)]));
        // Air has no top at all, so it is lower than every block.
        assert!(joins_west(&ctm, &[]));
    }

    #[test]
    fn a_table_can_join_a_neighbour_with_a_top_above_it() {
        let above = sunrise_connected::HeightJoin {
            above: true,
            ..Default::default()
        };
        let ctm = height_world(above, &[]);
        // Whatever the neighbour is, the cell above it has a collision top.
        assert!(joins_west(&ctm, &[([-1, 1, 0], 2)]));
        assert!(joins_west(&ctm, &[([-1, 0, 0], 4), ([-1, 1, 0], 4)]));
        assert!(!joins_west(&ctm, &[([-1, 0, 0], 2)]));
    }

    #[test]
    fn a_table_can_join_an_edge_by_the_cells_beside_the_block() {
        use geometry::{R, UR};
        let side = sunrise_connected::HeightJoin {
            higher: true,
            above: true,
            side: true,
            ..Default::default()
        };
        // The right edge (west) and the up-right corner (above it). The glass block's two cells
        // beside it, along the north face's other axis, are the ones above and below it.
        let table = Table::new(vec![R, UR], false, (4, 1), vec![0, 1, 2, 3]).unwrap();
        let mut ctm = table_world(table, false, Connect::Height(side), Vec::new());
        ctm.set_heights(vec![f64::NEG_INFINITY, 0.5, 1.0, 0.5, 0.25]);
        let at = |blocks: &[([i32; 3], u32)]| pick_in(&ctm, 1, 3, Face::North, blocks);
        // Nothing around: neither joins.
        assert_eq!(at(&[]), tile(500));
        // A higher block above joins the right edge, whatever is west, and not the corner.
        assert_eq!(at(&[([0, 1, 0], 2)]), tile(501));
        // The same one below the block.
        assert_eq!(at(&[([0, -1, 0], 2)]), tile(501));
        // A block that is not higher beside it joins nothing.
        assert_eq!(at(&[([0, 1, 0], 4)]), tile(500));
        // Without `side` the edge needs its own cell.
        let plain = sunrise_connected::HeightJoin {
            side: false,
            ..side
        };
        let table = Table::new(vec![R, UR], false, (4, 1), vec![0, 1, 2, 3]).unwrap();
        let mut ctm = table_world(table, false, Connect::Height(plain), Vec::new());
        ctm.set_heights(vec![f64::NEG_INFINITY, 0.5, 1.0, 0.5, 0.25]);
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([0, 1, 0], 2)]),
            tile(500)
        );
    }

    #[test]
    fn a_table_can_join_the_blocks_of_a_list() {
        use geometry::R;
        let stone = BlockSel {
            id: "minecraft:stone".to_owned(),
            states: Vec::new(),
        };
        let table = Table::new(vec![R], false, (2, 1), vec![1, 0]).unwrap();
        let ctm = table_world(table, false, Connect::Blocks, vec![stone]);
        // Stone (2) joins, glass (1) does not: the rule's own block is not in the list.
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 2)]),
            tile(500)
        );
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(501)
        );
    }

    #[test]
    fn a_table_can_join_by_equal_properties() {
        use geometry::R;
        let table = Table::new(vec![R], false, (2, 1), vec![1, 0]).unwrap();
        let ctm = table_rule_world(
            table,
            false,
            "minecraft:oak_log",
            Connect::Properties,
            Vec::new(),
            &["axis"],
        );
        // Logs (3 along y, 4 along x): a y log joins a y log beside it, not an x log.
        let at = |other| pick_in(&ctm, 3, 3, Face::North, &[([-1, 0, 0], other)]);
        assert_eq!(at(3), tile(500));
        assert_eq!(at(4), tile(501));
        assert_eq!(at(2), tile(501));
    }

    const GLASS: &str = "matchBlocks=glass\nmethod=ctm\ntiles=0-46";

    #[test]
    fn glass_joins_its_neighbours() {
        let ctm = world(&[("glass.properties", GLASS)]);
        // Alone: tile 0. A north face's right is west: glass there is the left end of a row.
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &[]), tile(1000));
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1001)
        );
        // Glass all around in the face's plane: the middle tile, 26.
        let ring: Vec<_> = [-1, 0, 1]
            .into_iter()
            .flat_map(|x| [-1, 0, 1].map(|y| ([x, y, 0], 1)))
            .collect();
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &ring), tile(1026));
        // Stone beside it doesn't join; stone has no rule.
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 2)]),
            tile(1000)
        );
        assert_eq!(pick_in(&ctm, 2, 7, Face::North, &[]), Pick::Keep);
    }

    #[test]
    fn rule_order_faces_and_skips() {
        let ctm = world(&[
            (
                "a_sides.properties",
                "matchBlocks=stone\nmethod=fixed\ntiles=<skip>\nfaces=sides",
            ),
            ("b_glass.properties", GLASS),
            ("stone.properties", "method=fixed\ntiles=x\nfaces=top"),
            ("z.properties", "matchBlocks=stone\nmethod=fixed\ntiles=y"),
        ]);
        // The tile rule (stone.properties, by its file name) comes first, on the top face only.
        assert_eq!(pick_in(&ctm, 2, 7, Face::Up, &[]), tile(1000));
        // Sides: a_sides skips, so z draws.
        assert_eq!(pick_in(&ctm, 2, 7, Face::North, &[]), tile(4000));
        // The tile rule follows the sprite, whatever the block: glass with the stone sprite.
        assert_eq!(pick_in(&ctm, 1, 7, Face::Up, &[]), tile(1000));
    }

    #[test]
    fn compact_quarters_and_overrides() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=ctm_compact\ntiles=0-5\nctm.1=5",
        )]);
        let layer = |i: u16| TileLayer::Layer {
            layer: 1000 + i,
            translucent: false,
            opaque: false,
        };
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[]),
            Pick::Quarters([layer(0); 4])
        );
        // Joined right only is case 1, replaced whole by tile 5.
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1005)
        );
        // Joined left only (east of a north face): quarters [3, 0, 3, 0].
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([1, 0, 0], 1)]),
            Pick::Quarters([layer(3), layer(0), layer(3), layer(0)])
        );
    }

    #[test]
    fn logs_turn_with_their_axis() {
        let ctm = world(&[(
            "log.properties",
            "matchBlocks=oak_log\nmethod=vertical\ntiles=0-3",
        )]);
        // An upright log with a log above: the bottom of a column.
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 3)]),
            tile(1000)
        );
        // A log lying along x: on its north face the texture's up points east, so a log to the
        // east (same block, other axis state) is above it.
        assert_eq!(
            pick_in(&ctm, 4, 3, Face::North, &[([1, 0, 0], 4)]),
            tile(1000)
        );
        assert_eq!(
            pick_in(&ctm, 4, 3, Face::North, &[([0, 1, 0], 4)]),
            tile(1003)
        );
    }

    #[test]
    fn top_and_connect_state() {
        let ctm = world(&[(
            "block_oak_log.properties",
            "method=top\ntiles=0\nconnect=state",
        )]);
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 3)]),
            tile(1000)
        );
        // The other axis is another state.
        assert_eq!(
            pick_in(&ctm, 3, 3, Face::North, &[([0, 1, 0], 4)]),
            Pick::Keep
        );
        assert_eq!(pick_in(&ctm, 3, 3, Face::Up, &[([0, 1, 0], 3)]), Pick::Keep);
    }

    #[test]
    fn linked_random_follows_the_lower_half() {
        let ctm = world(&[(
            "door.properties",
            "matchBlocks=oak_door\nmethod=random\ntiles=0-7\nlinked=true",
        )]);
        let mut tiles = std::collections::HashSet::new();
        for x in 0..20 {
            let lower = ctm.pick(
                5,
                3,
                Face::North,
                [x, 64, 0],
                FaceNeighbours {
                    state: |_| 0,
                    group: |_| None,
                    full_faces: |_| 0,
                    biome: || None,
                    texture: || None,
                },
                |_| {},
            );
            let upper = ctm.pick(
                6,
                3,
                Face::North,
                [x, 65, 0],
                FaceNeighbours {
                    state: |_| 0,
                    group: |_| None,
                    full_faces: |_| 0,
                    biome: || None,
                    texture: || None,
                },
                |_| {},
            );
            assert_eq!(lower, upper, "{x}");
            tiles.insert(format!("{lower:?}"));
        }
        assert!(tiles.len() > 3, "the doors are not all alike");
    }

    #[test]
    fn heights_and_biomes() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=fixed\ntiles=0\nheights=0-63\nbiomes=desert",
        )]);
        let at = |y, biome: &'static str| {
            ctm.pick(
                1,
                3,
                Face::Up,
                [0, y, 0],
                FaceNeighbours {
                    state: |_| 0,
                    group: |_| None,
                    full_faces: |_| 0,
                    biome: || Some(biome),
                    texture: || None,
                },
                |_| {},
            )
        };
        assert_eq!(at(10, "minecraft:desert"), tile(1000));
        assert_eq!(at(64, "minecraft:desert"), Pick::Keep);
        assert_eq!(at(10, "minecraft:plains"), Pick::Keep);
    }

    #[test]
    fn connect_by_tile_reads_the_neighbours_same_side() {
        let mut ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=horizontal\nconnect=tile\ntiles=0-3",
        )]);
        // Stone shows the glass sprite (3) on its north side; the door only on its south.
        let mut faces = vec![[9u16; 6]; 7];
        faces[1][Face::North as usize] = 3;
        faces[2][Face::North as usize] = 3;
        faces[5][Face::South as usize] = 3;
        ctm.face_layers = faces;
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 2)]),
            tile(1000)
        );
        assert_eq!(
            pick_in(&ctm, 1, 3, Face::North, &[([-1, 0, 0], 5)]),
            tile(1003)
        );
    }

    #[test]
    fn repeat_and_rows_then_columns() {
        let ctm = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=repeat\nwidth=2\nheight=2\ntiles=0-3",
        )]);
        let at = |pos| {
            ctm.pick(
                1,
                3,
                Face::North,
                pos,
                FaceNeighbours {
                    state: |_| 0,
                    group: |_| None,
                    full_faces: |_| 0,
                    biome: || None,
                    texture: || None,
                },
                |_| {},
            )
        };
        assert_eq!(at([0, 64, 0]), tile(1000));
        // The north face's right is west; rows run down.
        assert_eq!(at([-1, 64, 0]), tile(1001));
        assert_eq!(at([0, 63, 0]), tile(1002));
        let hv = world(&[(
            "glass.properties",
            "matchBlocks=glass\nmethod=horizontal+vertical\ntiles=0-6",
        )]);
        // Alone in its row, glass above: the column's bottom, tile 4.
        assert_eq!(
            pick_in(&hv, 1, 3, Face::North, &[([0, 1, 0], 1)]),
            tile(1004)
        );
        assert_eq!(
            pick_in(&hv, 1, 3, Face::North, &[([-1, 0, 0], 1)]),
            tile(1000)
        );
    }

    #[test]
    fn skips_defaults_and_narrowed_rules() {
        let ctm = world(&[
            (
                "a.properties",
                "matchBlocks=glass\nmethod=ctm_compact\ntiles=<skip> 1-4",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=0\nfaces=sides",
            ),
            (
                "c.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=<default>",
            ),
            ("d.properties", "matchBlocks=glass\nmethod=fixed\ntiles=0"),
            (
                "e.properties",
                "matchBlocks=stone\nmatchTiles=glass_pane_top\nmethod=fixed\ntiles=0",
            ),
        ]);
        // e names a tile, so it is tried first (tile 1000s), but needs a stone with the pane
        // sprite. a's skipped quarter drops it; b (3000s) draws the sides; on top c draws the
        // glass itself, which stops d.
        assert_eq!(pick_in(&ctm, 1, 3, Face::North, &[]), tile(3000));
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), Pick::Keep);
        assert_eq!(pick_in(&ctm, 2, 7, Face::Up, &[]), Pick::Keep);
        let both = world(&[(
            "e.properties",
            "matchBlocks=glass\nmatchTiles=stone\nmethod=fixed\ntiles=0",
        )]);
        assert_eq!(pick_in(&both, 1, 7, Face::Up, &[]), tile(1000));
        assert_eq!(pick_in(&both, 1, 3, Face::Up, &[]), Pick::Keep);
        assert_eq!(pick_in(&both, 2, 7, Face::Up, &[]), Pick::Keep);
        // `top` passes the top face on.
        let top = world(&[
            ("block_glass.properties", "method=top\ntiles=0"),
            (
                "z.properties",
                "matchBlocks=glass\nmethod=fixed\ntiles=0\nfaces=top",
            ),
        ]);
        assert_eq!(pick_in(&top, 1, 3, Face::Up, &[]), tile(2000));
    }

    /// The overlays `pick` hands out, with what it picks for the face itself.
    fn overlays_in(
        ctm: &ConnectedTextures,
        state: u32,
        face: Face,
        blocks: &[([i32; 3], u32)],
    ) -> (Vec<u16>, Pick) {
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        let mut out = Vec::new();
        let pick = ctm.pick(
            state,
            3,
            face,
            [0, 64, 0],
            FaceNeighbours {
                state: at,
                group: |_| None,
                full_faces: |_| 0,
                biome: || None,
                texture: || None,
            },
            |o| out.push(o.layer),
        );
        (out, pick)
    }

    #[test]
    fn overlays_draw_toward_the_blocks_they_name() {
        // Stone gets glass's edge where glass is beside it; then a fixed tile for the stone.
        let ctm = world(&[
            (
                "a_overlay.properties",
                "matchBlocks=stone\nmethod=overlay\nconnectBlocks=glass\ntiles=0-16\ntintIndex=0\ntintBlock=oak_log",
            ),
            ("b.properties", "matchBlocks=stone\nmethod=fixed\ntiles=0"),
        ]);
        // Nothing beside: no overlay, and b draws.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[]), (vec![], tile(2000)));
        // Glass to the north of a top face (its texture's up): tile 15. Glass east and west
        // too: one tile for the three sides, 13.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[([0, 0, -1], 1)]).0, [1015]);
        let three = [([0, 0, -1], 1), ([1, 0, 0], 1), ([-1, 0, 0], 1)];
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &three).0, [1013]);
        // Glass only at the north-east corner: the corner tile, 14.
        assert_eq!(overlays_in(&ctm, 2, Face::Up, &[([1, 0, -1], 1)]).0, [1014]);
        // Stone beside stone draws nothing.
        assert_eq!(
            overlays_in(&ctm, 2, Face::Up, &[([0, 0, -1], 2)]).0,
            [0u16; 0]
        );
        let mut tint = None;
        ctm.pick(
            2,
            3,
            Face::Up,
            [0, 64, 0],
            FaceNeighbours {
                state: |o| if o == [0, 0, -1] { 1 } else { 0 },
                group: |_| None,
                full_faces: |_| 0,
                biome: || None,
                texture: || None,
            },
            |o| {
                tint = Some((o.tint, o.translucent));
            },
        );
        assert_eq!(
            tint,
            Some((Some((0, Some(3))), false)),
            "oak_log's first state is 3"
        );
    }

    #[test]
    fn overlay_joins_layers_and_tints() {
        let mut ctm = world(&[
            (
                "a.properties",
                "matchBlocks=stone\nmethod=overlay\nconnectTiles=glass\ntiles=0-16\nlayer=translucent\ntintIndex=0\ntintBlock=no_such_block",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=overlay_ctm\nconnectBlocks=stone\ntiles=0-46",
            ),
        ]);
        // connectTiles joins neighbours showing a named sprite on the same side: the door to the
        // north shows glass (sprite 3) on top.
        let mut faces = vec![[9u16; 6]; 7];
        faces[5][Face::Up as usize] = 3;
        ctm.face_layers = faces;
        let mut got = Vec::new();
        let door_north = |o: [i32; 3]| if o == [0, 0, -1] { 5 } else { 0 };
        ctm.pick(
            2,
            7,
            Face::Up,
            [0, 64, 0],
            FaceNeighbours {
                state: door_north,
                group: |_| None,
                full_faces: |_| 0,
                biome: || None,
                texture: || None,
            },
            |o| got.push(o),
        );
        assert_eq!(
            got,
            [Overlay {
                layer: 1015,
                translucent: true,
                // A tintBlock the game lacks tints nothing.
                tint: None,
            }]
        );
        // connectBlocks is only for `overlay`: overlay_ctm joins glass by `connect`, not stone.
        let (layers, _) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 2)]);
        assert_eq!(layers, [2000]);
        let (layers, _) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 1)]);
        assert_eq!(layers, [2001]);
    }

    #[test]
    fn overlay_methods_pick_like_their_own() {
        let ctm = world(&[
            (
                "a.properties",
                "matchBlocks=glass\nmethod=overlay_ctm\ntiles=0-46\nlayer=translucent",
            ),
            (
                "b.properties",
                "matchBlocks=glass\nmethod=overlay_fixed\ntiles=<skip>",
            ),
            (
                "c.properties",
                "matchBlocks=glass\nmethod=overlay_fixed\ntiles=0",
            ),
        ]);
        // overlay_ctm joins glass (connect=block): the left end of a row, tile 1; b's <skip>
        // draws nothing; c draws; the face keeps its own sprite.
        let (layers, pick) = overlays_in(&ctm, 1, Face::North, &[([-1, 0, 0], 1)]);
        assert_eq!((layers, pick), (vec![1001, 3000], Pick::Keep));
    }

    #[test]
    fn a_picked_tile_can_be_picked_again() {
        // b (rule 1) gives glass its tile b0, layer 2000; t (rule 0, a tile rule, tried first)
        // names that tile, so it draws in b0's place.
        let chain = |t: &str| {
            world(&[
                ("b.properties", "matchBlocks=glass\nmethod=fixed\ntiles=b0"),
                ("t.properties", t),
            ])
        };
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmethod=fixed\ntiles=0");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(1000));
        // A chained rule keeping the tile (<default>) leaves b's.
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmethod=fixed\ntiles=<default>");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
        // One that names another block doesn't chain on glass.
        let ctm = chain("matchTiles=optifine/ctm/b0.png\nmatchBlocks=stone\nmethod=fixed\ntiles=0");
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
        // An overlay tried on the face's own sprite and on the picked tile draws once.
        let ctm = chain("matchTiles=glass optifine/ctm/b0.png\nmethod=overlay_fixed\ntiles=0");
        let (overlays, pick) = overlays_in(&ctm, 1, Face::Up, &[]);
        assert_eq!((overlays, pick), (vec![1000], tile(2000)));
        // A rule after the one that picked, naming both the face's sprite and the tile, wasn't
        // tried: it chains.
        let ctm = world(&[
            ("a.properties", "matchTiles=glass\nmethod=fixed\ntiles=a0"),
            (
                "c.properties",
                "matchTiles=glass optifine/ctm/a0.png\nmethod=fixed\ntiles=0",
            ),
        ]);
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
    }

    #[test]
    fn chaining_follows_the_tile_not_its_pixels() {
        // Conquest's grass top: a placeholder tile that a rule beside it replaces. Another
        // folder's placeholder has the same pixels, hence the same layer, and its rule sorts
        // first; it must not draw on the grass.
        let ctm = world(&[
            (
                "grass/top.properties",
                "matchTiles=glass\nmethod=fixed\ntiles=1",
            ),
            (
                "grass/random_1.properties",
                "matchTiles=./1.png\nmethod=fixed\ntiles=17\nfaces=top",
            ),
            (
                "aaa/random_1.properties",
                "matchTiles=./1.png\nmethod=fixed\ntiles=5\nfaces=top",
            ),
        ]);
        // Rules in path order: aaa/random_1 (1000), grass/random_1 (2000), grass/top (3000).
        assert_eq!(pick_in(&ctm, 1, 3, Face::Up, &[]), tile(2000));
        // A face whose own sprite has a placeholder's pixels is no placeholder.
        assert_eq!(pick_in(&ctm, 2, 3000, Face::Up, &[]), Pick::Keep);
    }

    #[test]
    fn orient_texture_follows_the_quads_uvs() {
        let rule = |orient: &str| {
            world(&[(
                "glass.properties",
                &*format!("matchBlocks=glass\nmethod=horizontal\ntiles=0-3\norient={orient}"),
            )])
        };
        // A mirrored north face (a vanilla pane's): its texture's right is east.
        let mirrored = || {
            Some(Axes {
                right: [1, 0, 0],
                up: [0, 1, 0],
            })
        };
        let east = |o: [i32; 3]| if o == [1, 0, 0] { 1 } else { 0 };
        let at = |ctm: &ConnectedTextures| {
            ctm.pick(
                1,
                3,
                Face::North,
                [0, 64, 0],
                FaceNeighbours {
                    state: east,
                    group: |_| None,
                    full_faces: |_| 0,
                    biome: || None,
                    texture: mirrored,
                },
                |_| {},
            )
        };
        // Glass to the east is to the texture's right: the row's left end.
        assert_eq!(at(&rule("texture")), tile(1000));
        // By the cube's directions it is to the left: the row's right end.
        assert_eq!(at(&rule("none")), tile(1002));
        // A plain overlay keeps the standard directions: glass north of a top face is above it
        // (tile 15) whatever the quad's texture says.
        let overlay = world(&[(
            "a.properties",
            "matchBlocks=stone\nmethod=overlay\nconnectBlocks=glass\ntiles=0-16\norient=texture",
        )]);
        let turned = || {
            Some(Axes {
                right: [0, 0, 1],
                up: [1, 0, 0],
            })
        };
        let north = |o: [i32; 3]| if o == [0, 0, -1] { 1 } else { 0 };
        let mut got = Vec::new();
        overlay.pick(
            2,
            7,
            Face::Up,
            [0, 64, 0],
            FaceNeighbours {
                state: north,
                group: |_| None,
                full_faces: |_| 0,
                biome: || None,
                texture: turned,
            },
            |o| got.push(o.layer),
        );
        assert_eq!(got, [1015]);
    }

    #[test]
    fn stacked_panes_hide_the_strips_they_share() {
        let props = |pairs: &[(&str, &str)]| -> BlockProperties {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // 0 a pane with north and east arms, 1 the same pane with only a north arm, 2 another
        // pane block, 3 a pane no rule names.
        let states = [
            (
                "minecraft:glass_pane",
                props(&[("north", "true"), ("east", "true")]),
            ),
            ("minecraft:glass_pane", props(&[("north", "true")])),
            (
                "minecraft:white_stained_glass_pane",
                props(&[("north", "true")]),
            ),
            ("minecraft:iron_bars", props(&[("north", "true")])),
        ];
        let ctm = ConnectedTextures::build(
            rules(&[(
                "pane.properties",
                "matchBlocks=glass_pane white_stained_glass_pane\nmethod=ctm\ntiles=0-46",
            )]),
            states.iter().map(|(n, p)| Some((*n, p))),
            |_| None,
        );
        let (post, north, east) = ([0.5, 0.5], [0.5, 0.2], [0.8, 0.5]);
        assert!(ctm.stacked_pane_hides(0, 1, post));
        assert!(ctm.stacked_pane_hides(0, 1, north));
        // The pane above has no east arm: the strip shows; one with it hides it.
        assert!(!ctm.stacked_pane_hides(0, 1, east));
        assert!(ctm.stacked_pane_hides(1, 0, east));
        let (south, west) = ([0.5, 0.8], [0.2, 0.5]);
        assert!(!ctm.stacked_pane_hides(1, 0, south));
        assert!(!ctm.stacked_pane_hides(1, 0, west));
        assert!(ctm.stacked_pane_hides(1, 0, north));
        // Another block, or a pane without rules, hides nothing.
        assert!(!ctm.stacked_pane_hides(1, 2, post));
        assert!(!ctm.stacked_pane_hides(3, 3, post));
    }

    #[test]
    fn a_pane_is_what_the_block_facts_say_not_what_its_name_ends_with() {
        let none = BlockProperties::default();
        // 0 a mod's pane, named like no vanilla one; 1 a block named like a glass pane that
        // is not one.
        let states = [
            ("mymod:crystal_lattice", &none),
            ("mymod:fake_glass_pane", &none),
        ];
        let mut ctm = ConnectedTextures::build(
            rules(&[("pane.properties", "matchTiles=glass\nmethod=fixed\ntiles=0")]),
            states.iter().map(|&s| Some(s)),
            |name| (name == "minecraft:block/glass").then_some(3),
        );
        // Both states draw the sprite a rule that names no block matches.
        ctm.connect_panes(|_, _| Some(3), |id| id == BlockStateId(0));
        assert!(ctm.states[0].has_rules);
        assert!(!ctm.states[1].has_rules);
    }

    #[test]
    fn rules_name_the_sprites_they_match() {
        // `matchTiles` among the textures and beside the file (a chain onto another rule's
        // tile), and `connectTiles`: each as the sprite name the textures and tiles are inserted
        // under.
        let dir = "assets/minecraft/optifine/ctm/pack/crops";
        let rule = |file: &str, text: &str| {
            Rule::parse(
                &format!("{dir}/{file}"),
                &sunrise_optifine::properties::Properties::parse(text),
            )
            .unwrap()
            .unwrap()
        };
        let rules = [
            rule(
                "1.properties",
                "matchTiles=./19.png\nmethod=random\ntiles=5-6",
            ),
            rule(
                "2.properties",
                "matchTiles=textures/block/extra/placeholder.png\nmethod=overlay\n\
                 connectTiles=textures/block/stone.png\ntiles=0-16",
            ),
        ];
        assert_eq!(
            ConnectedTextures::matched_sprites(&rules.map(|r| r.rule)),
            [
                format!("{dir}/19.png"),
                "minecraft:block/extra/placeholder".to_owned(),
                "minecraft:block/stone".to_owned(),
            ]
        );
    }

    #[test]
    fn sprite_names() {
        assert_eq!(
            sprite_name("assets/minecraft/textures/block/dirt.png"),
            "minecraft:block/dirt"
        );
        assert_eq!(
            sprite_name("assets/minecraft/optifine/ctm/glass/0.png"),
            "assets/minecraft/optifine/ctm/glass/0.png"
        );
    }
    #[test]
    fn a_named_rule_matches_the_baked_face_and_draws_its_unstitched_tile() {
        // A red/green sheet: the face's model material is the items atlas's red cut, not the
        // blocks atlas's green cut of the same id. The replacement tile is another named cut.
        let root =
            std::env::temp_dir().join(format!("sunrise-ctm-unstitch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/block");
        let atlases = root.join("assets/minecraft/atlases");
        let models = root.join("assets/demo/models/block");
        std::fs::create_dir_all(&textures).unwrap();
        std::fs::create_dir_all(&atlases).unwrap();
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(
            models.join("panel.json"),
            r##"{"textures":{"panel":"demo:block/panel"},"elements":[
                {"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#panel"}}}]}"##,
        )
        .unwrap();
        let pixels: Vec<u8> = (0..32 * 16)
            .flat_map(|i| {
                if i % 32 < 16 {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 255]
                }
            })
            .collect();
        let mut encoder = png::Encoder::new(
            std::fs::File::create(textures.join("sheet.png")).unwrap(),
            32,
            16,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        std::fs::write(
            atlases.join("items.json"),
            r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,
                "regions":[{"sprite":"demo:block/panel","x":0,"y":0,"width":1,"height":1}]}]}"#,
        )
        .unwrap();
        std::fs::write(
            atlases.join("blocks.json"),
            r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,
                "regions":[{"sprite":"demo:block/panel","x":1,"y":0,"width":1,"height":1},
                           {"sprite":"demo:block/panel_0","x":1,"y":0,"width":1,"height":1}]}]}"#,
        )
        .unwrap();
        let rule = Rule::parse(
            "assets/minecraft/optifine/ctm/panel.properties",
            &Properties::parse(
                "matchTiles=demo:block/panel\nmethod=fixed\ntiles=demo:block/panel_0",
            ),
        )
        .unwrap()
        .unwrap();
        let registry = BlockRegistry::from_nbt(&sunrise_core::nbt::Tag::compound([
            (
                "DataVersion",
                sunrise_core::nbt::Tag::Int(sunrise_world::DATA_VERSION),
            ),
            (
                "states",
                sunrise_core::nbt::Tag::List(vec![sunrise_core::nbt::Tag::String(
                    "minecraft:air".into(),
                )]),
            ),
            (
                "defaults",
                sunrise_core::nbt::Tag::compound([(
                    "minecraft:air",
                    sunrise_core::nbt::Tag::Int(0),
                )]),
            ),
        ]))
        .unwrap();
        let source = ResourceSource::directory(root.clone());
        let mut sprites = Sprites::new(source.clone());
        let rules = vec![rule.rule];
        sprites.keep_apart(ConnectedTextures::matched_sprites(&rules));
        let mut catalog = crate::models::StateModels::default();
        catalog
            .bake_standalone(
                &source,
                &mut sprites,
                &crate::model_library::ParsedModels::default(),
                &["demo:block/panel".into()],
            )
            .unwrap();
        let face = &catalog.standalone("demo:block/panel").unwrap().faces[0];
        assert_ne!(face.layer, sprites.layers["demo:block/panel"]);
        let ctm = ConnectedTextures::load(
            &source,
            rules,
            &registry,
            &mut sprites,
            |_, _| None,
            |_| false,
        );
        let Pick::Tile(TileLayer::Layer { layer, .. }) = ctm.pick(
            0,
            u16::try_from(face.layer).unwrap(),
            Face::North,
            [0; 3],
            FaceNeighbours {
                state: |_| 0,
                group: |_| None,
                full_faces: |_| 0,
                biome: || None,
                texture: || None,
            },
            |_| {},
        ) else {
            panic!("the named rule must match the baked face");
        };
        sprites.finish();
        let at = layer as usize * (sprites.size * sprites.size * 4) as usize;
        assert_eq!(sprites.pixels[at..at + 4], [0, 255, 0, 255]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_table_rule_cuts_its_sheet_into_cells_and_a_non_grid_sheet_is_refused() {
        let root = std::env::temp_dir().join(format!("sunrise-ct-table-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/block");
        std::fs::create_dir_all(&textures).unwrap();
        let write = |name: &str, (width, height): (u32, u32)| {
            let mut encoder = png::Encoder::new(
                std::fs::File::create(textures.join(name)).unwrap(),
                width,
                height,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255].repeat((width * height) as usize))
                .unwrap();
        };
        // Two 2 x 2 tiles side by side, and a sheet 3 wide that two columns cannot cut.
        write("row.png", (4, 2));
        write("odd.png", (3, 2));
        let registry = BlockRegistry::from_facts_json(
            &br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#[..],
        )
        .unwrap();
        let rule = |sprite: &str| {
            use std::sync::Arc;
            let table = sunrise_connected::Table::new(
                vec![sunrise_connected::R],
                false,
                (2, 1),
                vec![0, 1],
            )
            .unwrap();
            let sheet = sunrise_connected::TableSheet {
                sprite: sprite.into(),
                table: Arc::new(table),
            };
            let mut rule = sunrise_connected::Rule::table(format!("{sprite}.json"), sheet);
            rule.match_blocks = vec![sunrise_connected::BlockSel {
                id: "demo:casing".into(),
                states: Vec::new(),
            }];
            rule
        };
        let load = |sprite: &str| {
            let source = ResourceSource::directory(root.clone());
            let mut sprites = Sprites::new(source.clone());
            ConnectedTextures::load(
                &source,
                vec![rule(sprite)],
                &registry,
                &mut sprites,
                |_, _| None,
                |_| false,
            )
        };
        assert!(!load("demo:block/row").is_empty());
        assert!(
            load("demo:block/odd").is_empty(),
            "a sheet that is not a grid of square tiles drops its rule"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn connect_group_requires_matching_group_including_zero_and_cover() {
        use std::sync::Arc;
        use sunrise_connected::{L, R, Rule, Table, TableSheet};

        let none = BlockProperties::default();
        let states = [
            ("minecraft:air", &none),
            ("demo:tank", &none),
            ("minecraft:stone", &none),
        ];
        // Table with left/right tests and cover=true: 4 outcomes -> tiles 0..4.
        let table = Table::new(vec![L, R], true, (4, 1), vec![0, 1, 2, 3]).unwrap();
        let mut rule = Rule::table(
            "assets/demo/sunrise/connected/tank.json".into(),
            TableSheet {
                sprite: "demo:block/tank_sheet".into(),
                table: Arc::new(table),
            },
        );
        rule.match_blocks = vec![sunrise_connected::BlockSel {
            id: "demo:tank".into(),
            states: Vec::new(),
        }];
        rule.connect_group = true;
        let mut plain = rule.clone();
        plain.connect_group = false;

        let build = |r: Rule| {
            let cells = (0u16..4)
                .map(|i| {
                    Some(TileLayer::Layer {
                        layer: 100 + i,
                        translucent: false,
                        opaque: false,
                    })
                })
                .collect();
            let loaded = LoadedRule {
                rule: r,
                tiles: Vec::new(),
                cells,
                listed: Vec::new(),
                keys: Vec::new(),
                chain: Vec::new(),
                overlay: OverlayJoin::default(),
            };
            ConnectedTextures::build(vec![loaded], states.iter().map(|&s| Some(s)), |_| None)
        };

        let plain_ctm = build(plain);
        assert!(!plain_ctm.has_group_rules());
        // A rule without `connect_group` must join normally and never invoke the group callback.
        assert_eq!(
            plain_ctm.pick(
                1,
                0,
                Face::Up,
                [0, 64, 0],
                FaceNeighbours {
                    state: |o| if o == [-1, 0, 0] { 1 } else { 0 },
                    group: |_| panic!(
                        "group callback must not run for rules without connect_group"
                    ),
                    full_faces: |_| 0,
                    biome: || None,
                    texture: || None,
                },
                |_| {},
            ),
            tile(101)
        );
        let ctm = build(rule);
        assert!(ctm.has_group_rules());

        // On Face::Up, left is [-1, 0, 0], right is [1, 0, 0], front (cover) is [0, 1, 0].
        let full_down = 1u8 << (Face::Down as u8);
        let pick_with =
            |neighbours: &[([i32; 3], u32)], groups: &[([i32; 3], u64)], cover_full: bool| {
                ctm.pick(
                    1,
                    0,
                    Face::Up,
                    [0, 64, 0],
                    FaceNeighbours {
                        state: |o| {
                            neighbours
                                .iter()
                                .find(|(p, _)| *p == o)
                                .map_or(0, |&(_, s)| s)
                        },
                        group: |o| groups.iter().find(|(p, _)| *p == o).map(|&(_, g)| g),
                        full_faces: |s| if cover_full && s == 1 { full_down } else { 0 },
                        biome: || None,
                        texture: || None,
                    },
                    |_| {},
                )
            };

        // Group None joins nothing — not even another None neighbour of the same block.
        assert_eq!(
            pick_with(&[([-1, 0, 0], 1), ([1, 0, 0], 1)], &[], false),
            tile(100)
        );
        // Same group (including group 0) joins; different group does not.
        assert_eq!(
            pick_with(
                &[([-1, 0, 0], 1), ([1, 0, 0], 1)],
                &[([0, 0, 0], 0), ([-1, 0, 0], 0), ([1, 0, 0], 7)],
                false,
            ),
            tile(101)
        );
        // Different block with the same group still does NOT join under Connect::Block.
        assert_eq!(
            pick_with(
                &[([-1, 0, 0], 2)],
                &[([0, 0, 0], 5), ([-1, 0, 0], 5)],
                false,
            ),
            tile(100)
        );
        // Cover suppresses left join only when the front-of-left cell [-1, 1, 0] ALSO matches
        // the block and shares the connection group.
        assert_eq!(
            pick_with(
                &[([-1, 0, 0], 1), ([-1, 1, 0], 1)],
                &[([0, 0, 0], 5), ([-1, 0, 0], 5), ([-1, 1, 0], 9)],
                true,
            ),
            tile(101)
        );
        assert_eq!(
            pick_with(
                &[([-1, 0, 0], 1), ([-1, 1, 0], 1)],
                &[([0, 0, 0], 5), ([-1, 0, 0], 5), ([-1, 1, 0], 5)],
                true,
            ),
            tile(100)
        );
    }
}
