//! Shared nearest-sampled sprite array for block and item textures.
use crate::{
    Result,
    resource_source::{ResourceRoot, ResourceSource},
};
#[cfg(test)]
use std::path::Path;
use std::{collections::BTreeMap, sync::Arc};
use sunrise_assets::animation::{AnimationClip, AnimationMetadata};

#[derive(Clone)]
pub struct SpriteAnimation {
    pub layer: u32,
    frames: Arc<Vec<Vec<u8>>>,
    clip: Arc<AnimationClip>,
    materials: [Option<Arc<Vec<Vec<u8>>>>; 2],
}
/// What an animated sprite shows: `frame`, mixed `millis` thousandths of the
/// way towards `next` (0 for a single frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBlend {
    pub frame: usize,
    pub next: usize,
    pub millis: u32,
}
impl SpriteAnimation {
    pub(crate) fn has_materials(&self) -> bool {
        self.materials.iter().any(Option::is_some)
    }
    /// Material categories follow the selected colour frame, without colour interpolation.
    pub(crate) fn material_pixels(&self, kind: usize, tick: u64) -> Option<&[u8]> {
        let frames = self.materials[kind].as_ref()?;
        Some(&frames[self.frame(tick) % frames.len()])
    }
    /// Polytone's `mode` on this animation, when it has one.
    pub fn mode(&self) -> Option<&sunrise_assets::animation::ClipMode> {
        self.clip.mode()
    }
    pub fn frame(&self, tick: u64) -> usize {
        self.clip.sample(tick).frame as usize
    }
    /// `SpriteContents.AnimationState.drawToAtlas`: an animation with
    /// `interpolate` mixes the frame with the next one by
    /// `(int)(subFrame / time * 1000)` thousandths; others show the frame.
    pub fn blend(&self, tick: u64) -> FrameBlend {
        self.blend_sample(self.clip.sample(tick))
    }
    /// [`Self::blend`], or what a Polytone `mode` (time of day, weather, an expression) picks
    /// from `inputs` for an animation that has one.
    pub fn blend_with(
        &self,
        tick: u64,
        inputs: Option<&sunrise_assets::animation::ModeInputs>,
    ) -> FrameBlend {
        match inputs.and_then(|i| self.clip.sample_mode(i)) {
            Some(sample) => self.blend_sample(sample),
            None => self.blend(tick),
        }
    }
    fn blend_sample(&self, sample: sunrise_assets::animation::AnimationSample) -> FrameBlend {
        let frame = sample.frame as usize;
        let next = sample.next_frame as usize;
        if self.clip.interpolates() && sample.interpolation_millis > 0 && next != frame {
            FrameBlend {
                frame,
                next,
                millis: sample.interpolation_millis,
            }
        } else {
            FrameBlend {
                frame,
                next: frame,
                millis: 0,
            }
        }
    }
    pub fn resized_pixels(&self, frame: usize, width: u32, height: u32) -> Vec<u8> {
        let source = self.pixels(frame);
        let side = (source.len() / 4).isqrt() as u32;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let offset = ((y * side / height * side + x * side / width) * 4) as usize;
                out.extend_from_slice(&source[offset..offset + 4]);
            }
        }
        out
    }
    pub fn pixels(&self, frame: usize) -> &[u8] {
        &self.frames[frame]
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StillKey {
    frame: u64,
    pbr: u64,
    side: u32,
    settings: [u32; 2],
    flags: [bool; 4],
}

#[derive(Default)]
pub struct Sprites {
    source: ResourceSource,
    /// Whether this load draws nothing (a server's): the mods' sprite sources are not run for it.
    serving: bool,
    /// Edge length of every array layer and the layers' current frames, set
    /// by [`Sprites::finish`].
    pub size: u32,
    /// Smallest native layer before array-size resampling.
    pub minimum_size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    /// The layers of texture files read on their own (`insert_limited`), by name: apart from
    /// `layers`, as a file is not the model material of its name.
    pub files: BTreeMap<String, u32>,
    /// The model materials of ids the items atlas lists, by name; `layers` keeps the blocks
    /// atlas's sprite of a name.
    items_layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
    pub opaque: Vec<bool>,
    pub translucent: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub mips: Vec<crate::mipmap::Settings>,
    /// Each layer's frames at the texture's own resolution (square), and the
    /// frame shown first.
    native: Vec<(u32, Arc<Vec<Vec<u8>>>, usize)>,
    /// Each layer's LabPBR maps (`_n` normals, `_s` specular) beside its texture: their
    /// frames at their own resolution (square).
    pbr: Vec<[Option<(u32, Vec<Vec<u8>>)>; 2]>,
    /// The layers' normal and specular maps at `pbr_size`, layer after layer, set by
    /// [`Sprites::finish`]: empty when no texture has either.
    pub normals: Vec<u8>,
    pub specular: Vec<u8>,
    pub pbr_size: u32,
    /// Still textures by their pixels, their LabPBR maps and mip settings: mods copy vanilla's
    /// textures under their own names, and one layer serves them all (the GPU's array may
    /// hold as few as 2048).
    still: std::collections::HashMap<StillKey, u32>,
    /// Names a connected-texture rule matches by name (`matchTiles`, `connectTiles`): each keeps a
    /// layer of its own ([`Sprites::keep_apart`]).
    apart: std::collections::HashSet<String>,
    /// Decoded textures kept on disk between launches ([`Sprites::with_disk_cache`]).
    disk: Option<crate::sprite_cache::SpriteCache>,
    /// Textures [`Sprites::prefetch`] decoded ahead of their insert, by sprite name or
    /// resource path.
    ready: std::collections::HashMap<String, crate::sprite_cache::Decoded>,
    /// What the merged blocks and items definitions list, once per load, on the first decode
    /// ([`Sprites::materials`]).
    materials: std::sync::OnceLock<crate::atlas_sources::Materials>,
}
/// Where a sprite's pixels are looked up: as a model's material (the items atlas, then the
/// blocks atlas), in the blocks atlas alone, or as a texture file read on its own.
#[derive(Clone, Copy)]
enum Lookup<'a> {
    Model,
    Blocks,
    File(&'a [u8]),
}
/// Which of the layer maps by name a sprite is kept in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Layers,
    Items,
    Files,
}
/// Edge length of vanilla's block textures: the smallest array layer.
pub const SPRITE_SIZE: u32 = 16;
/// The largest sprite array (all layers of the top mip level) when the settings leave it to
/// Sunrise: resource packs with bigger textures are sampled down to fit.
pub const DEFAULT_TEXTURE_BUDGET: u64 = 256 << 20;
/// The budget in bytes the next resource load keeps to (`set_texture_budget`).
static TEXTURE_BUDGET: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_TEXTURE_BUDGET);
/// The block textures' budget from the `textureBudgetMiB` setting (0 for the default), for
/// resources loaded from now on.
pub fn set_texture_budget(mib: u32) {
    let bytes = match mib {
        0 => DEFAULT_TEXTURE_BUDGET,
        mib => u64::from(mib) << 20,
    };
    TEXTURE_BUDGET.store(bytes, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the resource packs' LabPBR maps (`set_labpbr_maps`).
static LABPBR_MAPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether resource loads from now on read the resource packs' LabPBR maps beside their block
/// textures. Native material stages and terrain shader packs read them; without a pack they cost
/// neither loading time nor video memory.
pub fn set_labpbr_maps(on: bool) {
    LABPBR_MAPS.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the LabPBR maps (`set_labpbr_maps`).
pub fn labpbr_maps() -> bool {
    LABPBR_MAPS.load(std::sync::atomic::Ordering::Relaxed)
}
/// Budget for either complete normal or specular image set: a quarter of the block textures'
/// budget, so raising `textureBudgetMiB` sharpens a pack's LabPBR maps along with its colours.
fn map_bytes(texture_budget: u64) -> u64 {
    texture_budget / 4
}
/// [`map_bytes`] for the budget the next resource load keeps to.
fn max_map_bytes() -> u64 {
    map_bytes(TEXTURE_BUDGET.load(std::sync::atomic::Ordering::Relaxed))
}
/// Fewer layers than any game has (vanilla's are about 1300): what a map is kept at while
/// loading assumes at least these many share the array.
const LAYERS_AT_LEAST: u64 = 1024;
/// A layer's normal map where its texture has none: flat, unoccluded, at full height.
pub const FLAT_NORMAL: [u8; 4] = [128, 128, 255, 255];
/// A layer's specular map where its texture has none: rough, not metal, not glowing.
pub const NO_SPECULAR: [u8; 4] = [0, 0, 0, 0];
/// The largest power-of-two side from `start` down (at least `SPRITE_SIZE`) at which `layers`
/// square RGBA layers take at most `bytes`.
fn side_within(start: u32, layers: u64, bytes: u64) -> u32 {
    let mut size = start.max(SPRITE_SIZE).next_power_of_two();
    while size > SPRITE_SIZE && layers * u64::from(size) * u64::from(size) * 4 > bytes {
        size /= 2;
    }
    size
}
/// Reduce data maps by selecting whole texels: material IDs and sentinel bytes cannot be averaged.
fn shrink_map(frame: Vec<u8>, side: u32, max: u32) -> (u32, Vec<u8>) {
    if side <= max || !side.is_power_of_two() || !max.is_power_of_two() {
        return (side, frame);
    }
    (max, resample(&frame, side, max))
}
/// Nearest-resample a square RGBA frame to another square size.
pub(crate) fn resample(frame: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    for row in 0..to {
        let source_row = row * from / to;
        for column in 0..to {
            let offset = ((source_row * from + column * from / to) * 4) as usize;
            out.extend_from_slice(&frame[offset..offset + 4]);
        }
    }
    out
}
impl Sprites {
    /// A sprite atlas retains the source used for this resource load.
    pub fn new(source: ResourceSource) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }
    /// The sprites of a load that draws nothing (a server's): the mods' sprite sources are
    /// declared but not run for it, and no decoded textures are kept.
    pub fn for_server(source: ResourceSource) -> Self {
        Self {
            source,
            serving: true,
            ..Self::default()
        }
    }
    /// Sprites that reuse textures decoded by an earlier launch, kept in a
    /// cache file in `root`. A texture's decoded form stands for the sprite lists this load's
    /// atlas sources gave, mods' included, and no other.
    pub fn with_disk_cache(root: &ResourceRoot) -> Self {
        let listing = crate::atlas_sources::listing(root.source());
        Self {
            source: root.source().clone(),
            disk: crate::sprite_cache::SpriteCache::open(root, listing.fingerprint()),
            materials: crate::atlas_sources::Materials::over(listing, root.source()).into(),
            ..Self::default()
        }
    }
    /// Keeps each of `names` on a layer of its own, never shared with an identical texture: the
    /// connected-texture rules that name them find their faces by layer, so a texture sharing it
    /// would be matched too (a tile a rule chains onto, and a model's transparent placeholder
    /// with the same pixels). Before the textures that bear these names are inserted.
    pub fn keep_apart(&mut self, names: impl IntoIterator<Item = String>) {
        self.apart.extend(names);
    }
    /// Write what this run decoded to the cache file.
    pub fn save_disk_cache(&mut self) {
        if let Some(disk) = &mut self.disk {
            disk.save();
        }
    }
    fn materials(&self) -> &crate::atlas_sources::Materials {
        self.materials.get_or_init(|| {
            if self.serving {
                let listing = crate::atlas_sources::Listing::build(
                    &self.source,
                    &crate::mod_sprite_sources::Declared,
                );
                crate::atlas_sources::Materials::over(Arc::new(listing), &self.source)
            } else {
                crate::atlas_sources::Materials::new(&self.source)
            }
        })
    }
    /// Every sprite a mod's sprite source made in the blocks atlas, by pixels or by naming a
    /// texture, added though no model names it: the mod that made it looks it up after the
    /// stitch. Each is the blocks atlas's sprite of its id, whatever the items atlas lists.
    pub fn insert_mod_sprites(&mut self) -> Result<()> {
        for id in self.materials().mod_sprites() {
            self.insert_in_blocks(&id)?;
        }
        Ok(())
    }
    /// How many array layers the textures take.
    pub fn count(&self) -> usize {
        self.native.len()
    }
    /// `SpriteContents.computeTransparency(u0, v0, u1, v1)` is NONE: every texel under
    /// the quad's UV box is fully opaque in every frame, so the quad needs no alpha test.
    pub fn region_opaque(&self, layer: u32, uv: &[[f32; 2]; 4]) -> bool {
        if self.opaque[layer as usize] {
            return true;
        }
        let (side, frames, _) = &self.native[layer as usize];
        let side = *side as usize;
        let range = |axis: usize| {
            let lo = uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
            let hi = uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max);
            let clamp = |v: f32| (v.max(0.0) as usize).min(side);
            clamp((lo * side as f32).floor())..clamp((hi * side as f32).ceil())
        };
        let (columns, rows) = (range(0), range(1));
        frames.iter().all(|frame| {
            rows.clone().all(|y| {
                columns
                    .clone()
                    .all(|x| frame[(y * side + x) * 4 + 3] == 255)
            })
        })
    }
    /// Lay every layer out at one size: the largest texture's (resource packs
    /// may ship 32x, 64x... textures, like vanilla's stitched atlas), rounded
    /// up to a power of two and limited to the texture budget (`set_texture_budget`). Smaller
    /// textures are scaled up by whole pixels, so UVs stay per layer.
    #[sunrise_mod_api_macros::replaceable]
    pub fn finish(&mut self) {
        self.lay_out(true);
    }
    /// [`Sprites::finish`] for a server, which draws nothing: the layers'
    /// size, but neither their pixels nor the animations' frames.
    pub fn finish_without_pixels(&mut self) {
        self.lay_out(false);
    }
    fn lay_out(&mut self, pixels: bool) {
        // Prefetched textures nothing inserted.
        self.ready = Default::default();
        let layers = self.native.len().max(1) as u64;
        let largest = self.native.iter().map(|n| n.0).max().unwrap_or(SPRITE_SIZE);
        self.minimum_size = self.native.iter().map(|n| n.0).min().unwrap_or(SPRITE_SIZE);
        let budget = TEXTURE_BUDGET.load(std::sync::atomic::Ordering::Relaxed);
        let size = side_within(largest, layers, budget);
        if pixels {
            log::info!(
                "Sunrise: block sprites: {layers} layers at {size} px (largest {largest} px, budget {} MiB)",
                budget >> 20
            );
        }
        if !pixels {
            self.pixels = Vec::new();
            self.animations = Vec::new();
            self.pbr = Vec::new();
            self.size = size;
            return;
        }
        let mut resized = Vec::with_capacity(self.native.len());
        self.pixels.clear();
        for (from, frames, initial) in &self.native {
            // Native frames already match the atlas size: share the immutable
            // buffers with animations rather than copying every frame again.
            let frames: Arc<Vec<Vec<u8>>> = if *from == size {
                Arc::clone(frames)
            } else {
                Arc::new(frames.iter().map(|f| resample(f, *from, size)).collect())
            };
            self.pixels.extend_from_slice(&frames[*initial]);
            resized.push(frames);
        }
        for animation in &mut self.animations {
            animation.frames = resized[animation.layer as usize].clone();
        }
        self.size = size;
        self.normals.clear();
        self.specular.clear();
        let largest_map = self.pbr.iter().flatten().flatten().map(|m| m.0).max();
        self.pbr_size = largest_map.map_or(SPRITE_SIZE, |l| {
            side_within(l.min(size), layers, max_map_bytes())
        });
        if largest_map.is_some() {
            let pbr_size = self.pbr_size;
            log::info!("Sunrise: LabPBR maps at {pbr_size} px, {layers} layers");
            for animation in &mut self.animations {
                animation.materials = self.pbr[animation.layer as usize].each_ref().map(|map| {
                    map.as_ref().map(|(from, frames)| {
                        Arc::new(
                            frames
                                .iter()
                                .map(|frame| resample(frame, *from, pbr_size))
                                .collect(),
                        )
                    })
                });
            }
            for (layer, maps) in std::mem::take(&mut self.pbr).into_iter().enumerate() {
                for (map, (out, none)) in maps.into_iter().zip([
                    (&mut self.normals, FLAT_NORMAL),
                    (&mut self.specular, NO_SPECULAR),
                ]) {
                    match map {
                        Some((from, frames)) => {
                            let initial = self.native[layer].2 % frames.len();
                            let (from, pixels) =
                                shrink_map(frames[initial].clone(), from, pbr_size);
                            out.extend(resample(&pixels, from, pbr_size));
                        }
                        None => out.extend(none.repeat((pbr_size * pbr_size) as usize)),
                    }
                }
            }
        }
    }
    /// The layer already loaded for a model material, rather than the explicit blocks atlas's.
    pub(crate) fn model_layer(&self, name: &str) -> Option<u32> {
        self.layer_of(self.slot_of(Lookup::Model, name), name)
    }
    /// [`Sprites::insert`] without the missing-texture fallback: a file that doesn't load is an
    /// error, and nothing is added.
    pub fn try_insert(&mut self, name: &str) -> Result<u32> {
        match self.model_layer(name) {
            Some(layer) => Ok(layer),
            None => self.load(name, Lookup::Model),
        }
    }
    /// Load a sprite id or resource file only when its PNG fits the byte limit,
    /// including cached layers. The checked bytes are reused when decoding.
    pub fn try_insert_limited(&mut self, name: &str, max: u64) -> Result<u32> {
        let bytes = self.limited_png(name, max)?;
        match self.files.get(name) {
            Some(layer) => Ok(*layer),
            None => self.load(name, Lookup::File(&bytes)),
        }
    }
    fn limited_png(&self, name: &str, max: u64) -> Result<Vec<u8>> {
        let path = if name.contains(':') {
            crate::atlas_sources::texture_resource(name, "textures")
        } else {
            name.to_owned()
        };
        Ok(self.source.read_limited(&path, max)?)
    }
    /// Like `insert`, but refuses unreadable or oversized PNG data before decoding. The layer
    /// is the file's own, in [`Sprites::files`]: not a model material, which an atlas
    /// definition may not list.
    pub fn insert_limited(&mut self, name: &str, max: u64) -> Result<u32> {
        let bytes = self.limited_png(name, max)?;
        if let Some(layer) = self.files.get(name) {
            return Ok(*layer);
        }
        let loaded = self.load(name, Lookup::File(&bytes));
        self.or_missing(Slot::Files, name, loaded)
    }
    /// [`Sprites::insert`] for what names the blocks atlas itself, as the fluid renderer does:
    /// the blocks atlas's sprite, in [`Sprites::layers`], not the items atlas's of the same id.
    pub fn insert_in_blocks(&mut self, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        let loaded = self.load(name, Lookup::Blocks);
        self.or_missing(Slot::Layers, name, loaded)
    }
    /// The layer of a texture, added on first use. A texture that is missing
    /// or does not load (a resource pack's filter or a broken file) becomes
    /// vanilla's missing texture, as in a stitched atlas.
    pub fn insert(&mut self, name: &str) -> Result<u32> {
        let slot = self.slot_of(Lookup::Model, name);
        if let Some(layer) = self.layer_of(slot, name) {
            return Ok(layer);
        }
        let loaded = self.load(name, Lookup::Model);
        self.or_missing(slot, name, loaded)
    }
    /// Where a lookup's layer is kept by name. A model's material is the items atlas's first,
    /// so one the items atlas lists is kept apart, and the blocks atlas's sprite of that name
    /// (what mods and the fluids look up) stays in [`Sprites::layers`].
    fn slot_of(&self, lookup: Lookup, name: &str) -> Slot {
        match lookup {
            Lookup::File(_) => Slot::Files,
            Lookup::Model if self.materials().in_items(name) => Slot::Items,
            _ => Slot::Layers,
        }
    }
    fn layer_of(&self, slot: Slot, name: &str) -> Option<u32> {
        match slot {
            Slot::Layers => &self.layers,
            Slot::Items => &self.items_layers,
            Slot::Files => &self.files,
        }
        .get(name)
        .copied()
    }
    fn slot_mut(&mut self, slot: Slot) -> &mut BTreeMap<String, u32> {
        match slot {
            Slot::Layers => &mut self.layers,
            Slot::Items => &mut self.items_layers,
            Slot::Files => &mut self.files,
        }
    }
    fn or_missing(&mut self, slot: Slot, name: &str, loaded: Result<u32>) -> Result<u32> {
        match loaded {
            Ok(layer) => Ok(layer),
            Err(e) => {
                // `MissingTextureAtlasSprite.getLocation()`: vanilla models name the
                // missing texture on purpose (block/air.json's particle), #853.
                if name != "minecraft:missingno" {
                    log::warn!("Sunrise: using the missing texture for {name}: {e}");
                }
                // MissingTextureAtlasSprite.generateMissingImage: 16x16,
                // magenta and black quarters.
                let frame: Vec<u8> = (0..SPRITE_SIZE * SPRITE_SIZE)
                    .flat_map(|i| {
                        let (x, y) = (i % SPRITE_SIZE, i / SPRITE_SIZE);
                        if (y < SPRITE_SIZE / 2) ^ (x < SPRITE_SIZE / 2) {
                            [248, 0, 248, 255]
                        } else {
                            [0, 0, 0, 255]
                        }
                    })
                    .collect();
                let layer = u32::try_from(self.native.len())?;
                self.opaque.push(true);
                self.pbr.push([None, None]);
                self.translucent.push(false);
                self.mips.push(crate::mipmap::Settings::default());
                self.slot_mut(slot).insert(name.to_owned(), layer);
                self.native.push((SPRITE_SIZE, Arc::new(vec![frame]), 0));
                Ok(layer)
            }
        }
    }
    /// Decode on every thread the textures about to be inserted: sprite names, or resource
    /// paths outside `textures/` (as [`Sprites::insert_file`] takes them). A loader that adds
    /// many (the block models, an OptiFine pack's tiles) calls this first and then inserts
    /// them in its own order, so the layers are the same as without it. What is already a
    /// layer or in the disk cache is skipped; what fails to decode is left for the insert,
    /// which reports it.
    pub fn prefetch<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let names: Vec<&str> = names
            .into_iter()
            .filter(|name| {
                !self.layers.contains_key(*name)
                    && !self.ready.contains_key(*name)
                    && self.disk.as_ref().is_none_or(|d| d.get(name).is_none())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let decoded = crate::par::par_map(&names, |name| {
            if name.contains(':') {
                self.decode(name, false).ok()
            } else {
                Self::decode_file(&self.source, name, name).ok()
            }
        });
        for (name, decoded) in names.into_iter().zip(decoded) {
            if let Some(decoded) = decoded {
                self.ready.insert(name.to_owned(), decoded);
            }
        }
    }
    fn load(&mut self, name: &str, lookup: Lookup) -> Result<u32> {
        // Custom Item Textures' files are outside `textures/`, which the disk cache's
        // fingerprint covers: like `insert_file`'s, they skip it. So do a file read on its own
        // and the blocks atlas's explicit lookups, whose results are not the model material
        // the cache keeps under this name.
        let uncached = name.starts_with("sunrise_cit:")
            || !name.contains(':')
            || !matches!(lookup, Lookup::Model);
        let cached = self
            .disk
            .as_ref()
            .filter(|_| !uncached)
            .and_then(|d| d.get(name))
            .cloned();
        let decoded = match cached {
            Some(decoded) => decoded,
            None => {
                let prefetched = match lookup {
                    Lookup::Model => self.ready.remove(name),
                    _ => None,
                };
                let decoded = match (prefetched, lookup) {
                    (Some(decoded), _) => decoded,
                    (None, Lookup::File(bytes)) => {
                        let path = if name.contains(':') {
                            crate::atlas_sources::texture_resource(name, "textures")
                        } else {
                            name.to_owned()
                        };
                        Self::decode_file_data(&self.source, &path, name, Some(bytes))?
                    }
                    (None, lookup) => self.decode(name, matches!(lookup, Lookup::Blocks))?,
                };
                if let Some(disk) = self.disk.as_mut().filter(|_| !uncached) {
                    disk.insert(name, decoded.clone());
                }
                decoded
            }
        };
        let slot = self.slot_of(lookup, name);
        if slot == Slot::Layers {
            return self.register(name, decoded);
        }
        // `register` names the layer in `layers`: the blocks atlas's sprite of that name, if
        // any, keeps its place.
        let blocks = self.layers.get(name).copied();
        let layer = self.register(name, decoded)?;
        match blocks {
            Some(blocks) => self.layers.insert(name.to_owned(), blocks),
            None => self.layers.remove(name),
        };
        self.slot_mut(slot).insert(name.to_owned(), layer);
        if slot == Slot::Items && !self.layers.contains_key(name) {
            // The name still answers with the blocks atlas's sprite.
            self.load(name, Lookup::Blocks).ok();
        }
        Ok(layer)
    }
    /// The layer of a texture outside `textures/` (an OptiFine pack's connected-texture tile),
    /// named by its resource path and added on first use. Unlike [`Sprites::insert`], a texture
    /// that does not load is an error: the caller drops what needed it. Such files bypass the
    /// disk cache, whose fingerprint covers only `textures/`.
    pub fn insert_file(&mut self, resource: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(resource) {
            return Ok(*layer);
        }
        let decoded = match self.ready.remove(resource) {
            Some(decoded) => decoded,
            None => Self::decode_file(&self.source, resource, resource)?,
        };
        self.register(resource, decoded)
    }
    /// The tiles of the texture `name` (`ns:path`) cut into a `columns` x `rows` grid, row by
    /// row from the top left: a Fusion connecting texture's sheet. Each tile is its own layer
    /// (named `fusion/<ns>/<path>#<tile>`); a fully transparent one is `None`, as Fusion skips
    /// empty tiles. Tiles are square, as wide as a column. An animated sheet's frames are whole
    /// grids one below the other, or with `per_tile` each cell is a column of its tile's frames.
    /// The sheet's own layer, if models already show it, shows its first tile from then on.
    pub fn insert_tiles(
        &mut self,
        name: &str,
        (columns, rows): (u32, u32),
        per_tile: bool,
    ) -> Result<Vec<Option<u32>>> {
        let path = crate::atlas_sources::texture_resource(name, "textures");
        let metadata = match self.source.read_to_string(&format!("{path}.mcmeta")) {
            Ok(text) => Some(crate::packs::first_json::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(text.as_bytes())?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata
            .as_ref()
            .and_then(|m| m.get("animation"))
            .map(|v| v.get().to_owned());
        // The sheet's `texture` section is each tile's.
        let texture = texture_section(metadata.as_ref());
        let strategy = texture
            .as_ref()
            .and_then(|t| t["mipmap_strategy"].as_str())
            .and_then(crate::mipmap::Strategy::parse)
            .unwrap_or_default();
        let alpha_cutoff_bias = texture
            .as_ref()
            .and_then(|t| t["alpha_cutoff_bias"].as_f64())
            .unwrap_or(0.0) as f32;
        let (width, height, pixels) = read_rgba(&self.source, &path)?;
        let side = width / columns.max(1);
        // How far down the next frame of a tile is, and the next row of tiles.
        let (frame_step, row_step) = match per_tile {
            true => (side, height / rows.max(1)),
            false => (rows * side, side),
        };
        let frames = match per_tile {
            true => row_step / side.max(1),
            false => height / frame_step.max(1),
        };
        let whole = match per_tile {
            true => height % rows.max(1) == 0 && row_step % side.max(1) == 0,
            false => height % frame_step.max(1) == 0,
        };
        if side == 0 || frames == 0 || width % columns.max(1) != 0 || !whole {
            return Err(format!("{name} isn't a {columns} x {rows} grid of square tiles").into());
        }
        let tiles = (0..columns * rows)
            .map(|tile| {
                let frames: Vec<Vec<u8>> = (0..frames)
                    .map(|f| {
                        let (x, y) = (
                            tile % columns * side,
                            tile / columns * row_step + f * frame_step,
                        );
                        (y..y + side)
                            .flat_map(|row| {
                                let start = ((row * width + x) * 4) as usize;
                                pixels[start..start + side as usize * 4].iter().copied()
                            })
                            .collect()
                    })
                    .collect();
                let alphas = || frames.iter().flat_map(|f| f.iter().skip(3).step_by(4));
                if alphas().all(|&a| a == 0) {
                    return Ok(None);
                }
                let decoded = crate::sprite_cache::Decoded {
                    side,
                    animation: animation.clone().filter(|_| frames.len() > 1),
                    strategy,
                    alpha_cutoff_bias,
                    has_transparent: alphas().any(|&a| a == 0),
                    opaque: alphas().all(|&a| a == 255),
                    translucent: alphas().any(|&a| a != 0 && a != 255),
                    frames,
                };
                let tile_name = format!("fusion/{}#{tile}", name.replace(':', "/"));
                self.register(&tile_name, decoded).map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        // Where Fusion doesn't draw the sheet (items, other models), the lone tile rather than
        // the whole sheet squeezed square, whose size would set every layer's.
        if let (Some(own), Some(&first)) = (self.model_layer(name), tiles.iter().flatten().next()) {
            self.show_as(own, first);
        }
        Ok(tiles)
    }
    /// Layer `layer` shows what `source` does, and is no longer the layer of its old pixels.
    fn show_as(&mut self, layer: u32, source: u32) {
        self.still.retain(|_, l| *l != layer);
        let (l, s) = (layer as usize, source as usize);
        self.native[l] = self.native[s].clone();
        self.opaque[l] = self.opaque[s];
        self.translucent[l] = self.translucent[s];
        self.mips[l] = self.mips[s];
        self.pbr[l] = self.pbr[s].clone();
        self.animations.retain(|a| a.layer != layer);
        if let Some(animation) = self.animations.iter().find(|a| a.layer == source) {
            let animation = SpriteAnimation {
                layer,
                ..animation.clone()
            };
            self.animations.push(animation);
        }
    }
    /// A model material's sprite as the blocks and items atlases finish it (items first): the
    /// file of the origin that wins, with its own `.mcmeta`, or the pixels a cut or palette
    /// makes, which have none. An id no atlas gives a sprite is an error, and the missing
    /// texture; only where neither atlas has a definition (a bare resource tree) is a name its
    /// own file. Custom Item Textures' ids are files outside the atlases.
    fn decode(&self, name: &str, blocks_only: bool) -> Result<crate::sprite_cache::Decoded> {
        if !name.contains(':') {
            return Err("invalid sprite identifier".into());
        }
        // `texture_resource`, so Custom Item Textures' `sunrise_cit:` sprites find their file.
        let path = crate::atlas_sources::texture_resource(name, "textures");
        let materials = self.materials();
        if materials.is_bare() || name.starts_with("sunrise_cit:") {
            return Self::decode_file(&self.source, &path, name);
        }
        let finish = |origin: &crate::atlas_sources::SpriteOrigin| match origin {
            crate::atlas_sources::SpriteOrigin::File(resource) => {
                let file = crate::atlas_sources::texture_resource(resource, "textures");
                Self::decode_file(&self.source, &file, name).ok()
            }
            generated => {
                let image = crate::atlas_sources::generate(generated, &|texture| {
                    let file = crate::atlas_sources::texture_resource(texture, "textures");
                    read_rgba(&self.source, &file).ok()
                })?;
                Self::decode_image(name, None, image).ok()
            }
        };
        match blocks_only {
            true => materials.resolve_in_blocks(name, finish),
            false => materials.resolve(name, finish),
        }
        .ok_or_else(|| format!("sprite {name} is in no atlas").into())
    }
    fn decode_file(
        source: &ResourceSource,
        path: &str,
        name: &str,
    ) -> Result<crate::sprite_cache::Decoded> {
        Self::decode_file_data(source, path, name, None)
    }
    fn decode_file_data(
        source: &ResourceSource,
        path: &str,
        name: &str,
        png: Option<&[u8]>,
    ) -> Result<crate::sprite_cache::Decoded> {
        let metadata = match source.read_to_string(&format!("{path}.mcmeta")) {
            Ok(text) => Some(crate::packs::first_json::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(text.as_bytes())?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let image = match png {
            Some(bytes) => decode_rgba(bytes)?,
            None => read_rgba(source, path)?,
        };
        Self::decode_image(name, metadata, image)
    }
    /// What decoding a sprite's pixels and `.mcmeta` sections gives.
    fn decode_image(
        name: &str,
        metadata: Option<BTreeMap<String, Box<serde_json::value::RawValue>>>,
        (png_width, png_height, pixels): (u32, u32, Vec<u8>),
    ) -> Result<crate::sprite_cache::Decoded> {
        let animation_text = metadata
            .as_ref()
            .and_then(|v| v.get("animation"))
            .map(|v| v.get().to_owned());
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let (width, height) = if let Some(animation) = &animation {
            let [width, height] =
                animation.frame_size(i32::try_from(png_width)?, i32::try_from(png_height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (png_width, png_height)
        };
        if width == 0 || height == 0 || png_width % width != 0 || png_height % height != 0 {
            return Err(format!("sprite {name} has invalid frame dimensions").into());
        }
        // Layers are square; a frame of another shape is nearest-resampled to
        // its longer side so UVs (normalized per layer) are unchanged.
        let side = width.max(height);
        let frames = square_frames(
            &pixels,
            png_width,
            png_height,
            width,
            height,
            side,
            crate::hud::MAX_IMAGE_BYTES,
        )?;
        let texture = texture_section(metadata.as_ref());
        Ok(crate::sprite_cache::Decoded {
            side,
            frames,
            animation: animation_text,
            strategy: texture
                .as_ref()
                .and_then(|t| t["mipmap_strategy"].as_str())
                .and_then(crate::mipmap::Strategy::parse)
                .unwrap_or_default(),
            alpha_cutoff_bias: texture
                .as_ref()
                .and_then(|t| t["alpha_cutoff_bias"].as_f64())
                .unwrap_or(0.0) as f32,
            has_transparent: pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0),
            opaque: pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            translucent: pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[3] != 255),
        })
    }
    /// A 16 x 16 opaque white sprite under `name`: what a flat colour is drawn from, tinted.
    pub fn insert_flat(&mut self, name: &str) -> Result<u32> {
        self.register(
            name,
            crate::sprite_cache::Decoded {
                side: 16,
                frames: vec![[255u8; 16 * 16 * 4].to_vec()],
                animation: None,
                strategy: Default::default(),
                alpha_cutoff_bias: 0.0,
                has_transparent: false,
                opaque: true,
                translucent: false,
            },
        )
    }
    /// Give a decoded texture its layer (or the one an identical still has).
    #[sunrise_mod_api_macros::replaceable]
    fn register(&mut self, name: &str, decoded: crate::sprite_cache::Decoded) -> Result<u32> {
        let crate::sprite_cache::Decoded {
            side,
            frames,
            animation: animation_text,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            opaque,
            translucent,
        } = decoded;
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let count = frames.len() as u32;
        let layer = u32::try_from(self.native.len())?;
        let pbr = match name.split_once(':') {
            Some(_) if labpbr_maps() => {
                let path = crate::atlas_sources::texture_resource(name, "textures");
                ["_n", "_s"]
                    .map(|suffix| pbr_map(&self.source, &path, suffix, count, animation.as_ref()))
            }
            None if labpbr_maps() && !name.starts_with("fusion/") => ["_n", "_s"]
                .map(|suffix| pbr_map(&self.source, name, suffix, count, animation.as_ref())),
            _ => [None, None],
        };
        let settings = crate::mipmap::Settings {
            native_size: side,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            // Custom Item Textures' ids are all item textures.
            item: name.starts_with("sunrise_cit:")
                || name
                    .split_once(':')
                    .is_some_and(|(_, p)| p.starts_with("item/")),
            dither_cutouts: crate::mipmap::dither_cutouts(),
        };
        let still =
            (animation.is_none() && frames.len() == 1 && !self.apart.contains(name)).then(|| {
                use std::hash::{Hash, Hasher};
                let mut frame = std::collections::hash_map::DefaultHasher::new();
                frames[0].hash(&mut frame);
                let mut maps = std::collections::hash_map::DefaultHasher::new();
                pbr.hash(&mut maps);
                StillKey {
                    frame: frame.finish(),
                    pbr: maps.finish(),
                    side,
                    settings: [
                        settings.strategy as u32,
                        settings.alpha_cutoff_bias.to_bits(),
                    ],
                    flags: [settings.has_transparent, settings.item, opaque, translucent],
                }
            });
        if let Some(&same) = still.as_ref().and_then(|k| self.still.get(k)) {
            self.layers.insert(name.to_owned(), same);
            return Ok(same);
        }
        if let Some(key) = still {
            self.still.insert(key, layer);
        }
        let frames = Arc::new(frames);
        let mut initial = 0;
        if let Some(clip) = animation
            .as_ref()
            .map(|m| m.clip(count))
            .transpose()?
            .flatten()
        {
            initial = clip.sample(0).frame as usize;
            self.animations.push(SpriteAnimation {
                layer,
                frames: frames.clone(),
                clip: Arc::new(clip),
                materials: [None, None],
            });
        }
        self.mips.push(settings);
        self.opaque.push(opaque);
        self.translucent.push(translucent);
        self.pbr.push(pbr);
        self.layers.insert(name.to_owned(), layer);
        self.native.push((side, frames, initial));
        Ok(layer)
    }
}

/// A PNG's width, height and pixels as RGBA. From the prepared data file
/// when it holds the texture: no file opened per texture, nor for its absent
/// LabPBR maps.
fn read_rgba(source: &ResourceSource, path: &str) -> Result<(u32, u32, Vec<u8>)> {
    decode_rgba(&source.read(path)?)
}
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    use crate::hud::{MAX_IMAGE_BYTES, image_buffer};
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    crate::hud::check_pack_size(reader.info().width, reader.info().height)?;
    let size = reader.output_buffer_size().unwrap_or(usize::MAX);
    let mut bytes = image_buffer(&[size], MAX_IMAGE_BYTES)?;
    bytes.resize(size, 0);
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut pixels = image_buffer(
        &[info.width as usize, info.height as usize, 4],
        MAX_IMAGE_BYTES,
    )?;
    for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
        let rgba = match info.color_type {
            png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
            png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
            png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
            png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => return Err("unsupported expanded sprite format".into()),
        };
        pixels.extend(rgba);
    }
    Ok((info.width, info.height, pixels))
}

/// Companion frames use the colour image's grid; a single-frame map stays static.
fn pbr_map(
    source: &ResourceSource,
    path: &str,
    suffix: &str,
    count: u32,
    animation: Option<&AnimationMetadata>,
) -> Option<(u32, Vec<Vec<u8>>)> {
    let stem = path.strip_suffix(".png")?;
    let map = format!("{stem}{suffix}.png");
    let (map_width, map_height, pixels) = read_rgba(source, &map).ok()?;
    let (width, height, _) = read_rgba(source, path).ok()?;
    let [frame_width, frame_height] = animation.map_or([width as i32, height as i32], |a| {
        a.frame_size(width as i32, height as i32)
    });
    let (frame_width, frame_height) = (
        u32::try_from(frame_width).ok()?,
        u32::try_from(frame_height).ok()?,
    );
    if frame_width == 0 || frame_height == 0 {
        return None;
    }
    let (columns, rows) = if count <= 1 || (map_width == frame_width && map_height == frame_height)
    {
        (1, 1)
    } else if u64::from(map_width) * u64::from(height) == u64::from(map_height) * u64::from(width) {
        (width / frame_width, height / frame_height)
    } else if u64::from(map_width) * u64::from(frame_height)
        == u64::from(map_height) * u64::from(frame_width)
    {
        (1, 1)
    } else {
        return None;
    };
    if columns == 0 || rows == 0 || map_width % columns != 0 || map_height % rows != 0 {
        return None;
    }
    let (w, h) = (map_width / columns, map_height / rows);
    if w == 0 || h == 0 {
        return None;
    }
    let side = w.max(h);
    let output_side = side_within(side, LAYERS_AT_LEAST, max_map_bytes()).min(side);
    let frames = square_frames(
        &pixels,
        map_width,
        map_height,
        w,
        h,
        output_side,
        crate::hud::MAX_IMAGE_BYTES,
    )
    .ok()?;
    Some((output_side, frames))
}
/// Each `width` x `height` frame of a `png_width` x `png_height` image, in
/// row-major order, sampled to `output_side`. The table and all its pixels
/// must fit `budget` before any frame is made.
fn square_frames(
    pixels: &[u8],
    png_width: u32,
    png_height: u32,
    width: u32,
    height: u32,
    output_side: u32,
    budget: usize,
) -> Result<Vec<Vec<u8>>> {
    let columns = png_width / width;
    let count = columns * (png_height / height);
    let side = output_side as usize;
    let pixel_bytes = crate::hud::image_bytes(&[side, side, 4, count as usize], budget)?;
    let mut frames = crate::hud::image_buffer(&[count as usize], budget - pixel_bytes)?;
    for index in 0..count {
        let (x, y) = (index % columns * width, index / columns * height);
        frames.push(crate::hud::strip_frame(
            pixels,
            png_width,
            x,
            y,
            width,
            height,
            output_side,
            budget,
        )?);
    }
    Ok(frames)
}
/// SpriteContents: the `.mcmeta` `texture` section (TextureMetadataSection's
/// mipmap strategy and alpha cutoff bias).
fn texture_section(
    metadata: Option<&BTreeMap<String, Box<serde_json::value::RawValue>>>,
) -> Option<serde_json::Value> {
    metadata
        .and_then(|v| v.get("texture"))
        .and_then(|v| serde_json::from_str(v.get()).ok())
}

#[cfg(test)]
pub(crate) fn test_animation(size: u32, layer: u32) -> SpriteAnimation {
    test_clip(size, layer, r#"{"frametime":1,"frames":[0,1]}"#)
}
#[cfg(test)]
pub(crate) fn test_clip(size: u32, layer: u32, metadata: &str) -> SpriteAnimation {
    let frame = |blue: u8| {
        (0..size * size)
            .flat_map(|i| [(i % size) as u8, (i / size) as u8, blue, 255])
            .collect()
    };
    test_frames(layer, vec![frame(0), frame(99)], metadata)
}
#[cfg(test)]
pub(crate) fn test_frames(layer: u32, frames: Vec<Vec<u8>>, metadata: &str) -> SpriteAnimation {
    let metadata = AnimationMetadata::from_json(metadata).unwrap();
    SpriteAnimation {
        layer,
        clip: Arc::new(metadata.clip(frames.len() as u32).unwrap().unwrap()),
        frames: Arc::new(frames),
        materials: [None, None],
    }
}

/// How many reduced copies a sprite atlas keeps: the Mipmap Levels option plus the
/// resampling-only levels an upscaled pack needs, within what the atlas holds.
pub(crate) fn sprite_mipmap_limit(requested: u32, size: u32, minimum: u32, available: u32) -> u32 {
    // Only the atlas native minimum determines how many resampling-only mips to skip.
    let upscale = (size / minimum.max(1)).max(1).ilog2();
    requested.saturating_add(upscale).min(available - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    /// A sprite is refused by the side its PNG header claims when no atlas
    /// would take it, before the reader sizes a buffer from that header.
    #[test]
    fn a_sprite_png_claiming_a_side_no_atlas_takes_is_refused() {
        let png = |width: u32| {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, width, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![0; width as usize]).unwrap();
            writer.finish().unwrap();
            bytes
        };
        assert_eq!(decode_rgba(&png(32_768)).unwrap().0, 32_768);
        let refused = decode_rgba(&png(40_000)).map(|(width, height, _)| (width, height));
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.is::<crate::hud::OverSizeLimit>()),
            "{refused:?}"
        );
    }
    /// A narrow image's frames widen to squares: a 64x2 strip of 64x1 frames
    /// makes two 64x64 layers, all of them held to the budget before any is
    /// made.
    #[test]
    fn square_frames_of_a_narrow_strip_are_held_to_the_budget() {
        let pixels = vec![5; 64 * 2 * 4];
        let frames = |budget| square_frames(&pixels, 64, 2, 64, 1, 64, budget);
        let budget = 2 * (64 * 64 * 4 + size_of::<Vec<u8>>());
        let made = frames(budget).unwrap();
        assert_eq!(made.len(), 2);
        assert!(made.iter().all(|frame| frame.len() == 64 * 64 * 4));
        assert!(frames(budget - 1).is_err_and(|e| e.is::<crate::hud::OverSizeLimit>()));
    }

    #[test]
    fn shrinking_rectangular_frames_keeps_the_squared_frames_sampling() {
        let pixels: Vec<u8> = (0..20).flat_map(|n| [n, 0, 0, 255]).collect();
        let frames = square_frames(&pixels, 5, 4, 5, 4, 2, 16 + size_of::<Vec<u8>>()).unwrap();
        assert_eq!(
            frames,
            [vec![0, 0, 0, 255, 2, 0, 0, 255, 5, 0, 0, 255, 7, 0, 0, 255]]
        );
    }
    #[test]
    fn archive_sprites_keep_deferred_reads_and_cache_on_their_reload_snapshot() -> Result<()> {
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("sunrise-archive-sprites-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let write = |path: &Path, red: u8| -> Result<()> {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 2, 4);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let pixels = [[red, 0, 0, 255].repeat(4), [0, red, 0, 255].repeat(4)].concat();
                encoder.write_header()?.write_image_data(&pixels)?;
            }
            let mut writer = zip::ZipWriter::new(File::create(path)?);
            for name in [
                "assets/demo/textures/block/flow.png",
                "assets/demo/textures/block/flow_n.png",
                "assets/demo/optifine/cit/flow.png",
                "assets/demo/optifine/ctm/flow.png",
            ] {
                writer.start_file(name, zip::write::SimpleFileOptions::default())?;
                writer.write_all(&bytes)?;
                writer.start_file(
                    format!("{name}.mcmeta"),
                    zip::write::SimpleFileOptions::default(),
                )?;
                writer.write_all(br#"{"animation":{"frametime":3,"frames":[1,0],"interpolate":true},"texture":{"alpha_cutoff_bias":0.25}}"#)?;
            }
            writer.finish()?;
            Ok(())
        };
        let jar = root.join("textures.jar");
        write(&jar, 10)?;
        let before = ResourceRoot::new(
            root.join("prepared"),
            root.join("cache"),
            ResourceSource::archive(&jar)?,
        );
        let mut sprites = Sprites::with_disk_cache(&before);
        sprites.prefetch(["demo:block/flow"]);
        let replacement = root.join("replacement.jar");
        write(&replacement, 20)?;
        fs::rename(&jar, root.join("old.jar"))?;
        fs::rename(replacement, &jar)?;
        // Insertion after replacement uses the retained source, both prefetched
        // textures and deferred reads outside textures/ (which bypass the cache).
        let layer = sprites.try_insert("demo:block/flow")?;
        assert_eq!(sprites.native[layer as usize].1[0][..4], [10, 0, 0, 255]);
        assert_eq!(sprites.animations[0].frame(0), 1);
        assert_eq!(sprites.mips[layer as usize].alpha_cutoff_bias, 0.25);
        for name in [
            "sunrise_cit:demo/optifine/cit/flow",
            "assets/demo/optifine/ctm/flow.png",
        ] {
            let layer = if name.contains(':') {
                sprites.try_insert(name)?
            } else {
                sprites.insert_file(name)?
            };
            assert_eq!(sprites.native[layer as usize].1[0][..4], [10, 0, 0, 255]);
        }
        let (_, frames) = pbr_map(
            before.source(),
            "assets/demo/textures/block/flow.png",
            "_n",
            2,
            None,
        )
        .unwrap();
        assert_eq!(frames[0][..4], [10, 0, 0, 255]);
        let mut sheet = Sprites::new(before.source().clone());
        let tiles = sheet.insert_tiles("demo:block/flow", (1, 1), false)?;
        assert_eq!(sheet.native[tiles[0].unwrap() as usize].1.len(), 2);
        sprites.save_disk_cache();
        let mut cached = Sprites::with_disk_cache(&before);
        cached.prefetch(["demo:block/flow"]);
        assert!(
            cached.ready.is_empty(),
            "the retained snapshot hits the disk cache"
        );
        let layer = cached.try_insert("demo:block/flow")?;
        assert_eq!(cached.native[layer as usize].1[0][0], 10);
        let after = ResourceRoot::new(
            root.join("prepared"),
            root.join("cache"),
            ResourceSource::archive(&jar)?,
        );
        let mut reloaded = Sprites::with_disk_cache(&after);
        let layer = reloaded.try_insert("demo:block/flow")?;
        assert_eq!(reloaded.native[layer as usize].1[0][0], 20);
        assert!(!root.join("assets").exists());
        assert!(!root.join("prepared").exists());
        drop((sprites, sheet, cached, reloaded, before, after));
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn native_frames_at_atlas_size_are_shared_without_resampling() {
        let pixels = vec![
            [10, 20, 30, 255].repeat((SPRITE_SIZE * SPRITE_SIZE) as usize),
            [40, 50, 60, 255].repeat((SPRITE_SIZE * SPRITE_SIZE) as usize),
        ];
        let native = Arc::new(pixels.clone());
        let smaller = Arc::new(vec![vec![12; 8 * 8 * 4], vec![34; 8 * 8 * 4]]);
        let metadata = r#"{"frametime":2}"#;
        let mut sprites = Sprites {
            native: vec![
                (SPRITE_SIZE, Arc::clone(&native), 1),
                (8, Arc::clone(&smaller), 0),
            ],
            pbr: vec![[None, None], [None, None]],
            animations: vec![
                test_frames(0, pixels.clone(), metadata),
                test_frames(1, smaller.as_ref().clone(), metadata),
            ],
            ..Default::default()
        };
        sprites.finish();
        assert!(Arc::ptr_eq(&sprites.animations[0].frames, &native));
        assert!(!Arc::ptr_eq(&sprites.animations[1].frames, &smaller));
        assert_eq!(sprites.animations[0].pixels(1), pixels[1].as_slice());
        let resized = resample(&smaller[0], 8, SPRITE_SIZE);
        assert_eq!(sprites.animations[1].pixels(0), resized.as_slice());
        assert_eq!(&sprites.pixels[..pixels[1].len()], pixels[1].as_slice());
    }
    #[test]
    fn the_texture_budget_picks_the_largest_side_that_fits() {
        // 1300 layers: vanilla's count, about.
        assert_eq!(side_within(512, 1300, 256 << 20), 128);
        assert_eq!(side_within(512, 1300, 64 << 20), 64);
        assert_eq!(side_within(512, 1300, 128 << 20), 128);
        assert_eq!(side_within(512, 1300, 1536 << 20), 512);
        assert_eq!(side_within(64, 1300, 1536 << 20), 64);
        // Never under vanilla's size, whatever the budget.
        assert_eq!(side_within(512, 1300, 1), SPRITE_SIZE);
    }
    #[test]
    fn labpbr_maps_follow_the_texture_budget() {
        // 1300 layers of 1024 px maps: 64 px at the default budget, 256 px at 1536 MiB.
        assert_eq!(
            side_within(1024, 1300, map_bytes(DEFAULT_TEXTURE_BUDGET)),
            64
        );
        assert_eq!(side_within(1024, 1300, map_bytes(1536 << 20)), 256);
    }
    #[test]
    fn a_labpbr_map_selects_whole_texels_and_missing_layers_get_defaults() {
        // Selecting a texel preserves encoded channels, including categorical values.
        let frame = [
            [200, 100, 0, 255],
            [0, 0, 0, 0],
            [200, 100, 0, 255],
            [0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(shrink_map(frame, 2, 1), (1, vec![200, 100, 0, 255]));
        let mut sprites = Sprites {
            native: vec![
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
            ],
            pbr: vec![
                [Some((16, vec![[9, 8, 7, 6].repeat(16 * 16)])), None],
                [None, None],
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.pbr_size, 16);
        assert_eq!(sprites.normals[..4], [9, 8, 7, 6]);
        assert_eq!(sprites.normals[16 * 16 * 4..][..4], FLAT_NORMAL);
        assert_eq!(sprites.specular[..4], NO_SPECULAR);
        assert_eq!(sprites.specular.len(), 2 * 16 * 16 * 4);
    }
    #[test]
    fn companion_grid_and_static_map_follow_colour_frame_selection() {
        let root =
            std::env::temp_dir().join(format!("sunrise-companion-grid-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let write = |name: &str, width, height, pixels: &[u8]| {
            let mut encoder =
                png::Encoder::new(File::create(root.join(name)).unwrap(), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        write("grid.png", 4, 4, &[200, 200, 200, 255].repeat(16));
        let maps: Vec<u8> = (0..16)
            .flat_map(|i| {
                let cell = (i / 8) * 2 + (i % 4) / 2;
                [10 + cell as u8, 230, 65, 255]
            })
            .collect();
        write("grid_s.png", 4, 4, &maps);
        write("grid_n.png", 2, 2, &[128, 128, 255, 255].repeat(4));
        let metadata =
            r#"{"width":2,"height":2,"frametime":2,"interpolate":true,"frames":[3,1,2,0]}"#;
        let animation = AnimationMetadata::from_json(metadata).unwrap();
        let (side, frames) = pbr_map(
            &ResourceSource::directory(root.clone()),
            "grid.png",
            "_s",
            4,
            Some(&animation),
        )
        .unwrap();
        assert_eq!(side, 2);
        assert_eq!(
            frames.iter().map(|f| f[0]).collect::<Vec<_>>(),
            [10, 11, 12, 13]
        );
        let (_, normals) = pbr_map(
            &ResourceSource::directory(root.clone()),
            "grid.png",
            "_n",
            4,
            Some(&animation),
        )
        .unwrap();
        assert_eq!(normals.len(), 1);
        let mut sprite = test_frames(0, vec![vec![0; 16]; 4], metadata);
        sprite.materials = [Some(Arc::new(normals)), Some(Arc::new(frames))];
        for (tick, expected) in [(0, 13), (1, 13), (2, 11), (4, 12), (6, 10)] {
            assert_eq!(sprite.material_pixels(1, tick).unwrap()[0], expected);
            assert_eq!(&sprite.material_pixels(0, tick).unwrap()[..4], &FLAT_NORMAL);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn textures_without_labpbr_maps_keep_none() {
        let mut sprites = Sprites {
            native: vec![(SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0)],
            pbr: vec![[None, None]],
            ..Default::default()
        };
        sprites.finish();
        assert!(sprites.normals.is_empty() && sprites.specular.is_empty());
    }
    #[test]
    fn mipmap_option_keeps_real_16px_reductions() {
        for levels in 0..=4 {
            assert_eq!(sprite_mipmap_limit(levels, 32, 16, 6), levels + 1);
            assert_eq!(sprite_mipmap_limit(levels, 16, 16, 5), levels);
        }
    }
    #[test]
    fn files_outside_textures_load_by_path() {
        let root = std::env::temp_dir().join(format!("sunrise-sprite-file-{}", std::process::id()));
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        fs::create_dir_all(root.join(tile).parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(root.join(tile)).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 128]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert_file(tile).unwrap();
        assert_eq!(sprites.insert_file(tile).unwrap(), layer);
        assert!(sprites.translucent[layer as usize]);
        // A missing tile is an error, not the missing texture.
        assert!(
            sprites
                .insert_file("assets/minecraft/optifine/ctm/glass/1.png")
                .is_err()
        );
        assert_eq!(sprites.count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn tile_sheets_cut_into_a_layer_per_tile() {
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-tiles-{}", std::process::id()));
        let file = root.join("assets/minecraft/textures/block/sheet.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // 3 columns of 2 x 2 tiles over 4 rows of pixels: the pixel at (x, y) has red x, green
        // y; the middle column is transparent, and the top-right tile half see-through.
        let (w, h) = (6u32, 4u32);
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let alpha = match x / 2 {
                    1 => 0,
                    2 if y < 2 => 128,
                    _ => 255,
                };
                [x as u8, y as u8, 0, alpha]
            })
            .collect();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        // Red and green of each pixel in a layer's frame.
        let texels = |sprites: &Sprites, layer: u32, frame: usize| -> Vec<(u8, u8)> {
            sprites.native[layer as usize].1[frame]
                .chunks(4)
                .map(|p| (p[0], p[1]))
                .collect()
        };
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let name = "minecraft:block/sheet";
        let own = sprites.insert(name).unwrap();
        // Still, as a 3 x 2 grid: the empty middle column draws nothing.
        let tiles = sprites.insert_tiles(name, (3, 2), false).unwrap();
        // The sheet's own layer now shows the top-left tile, at a tile's size.
        assert_eq!(texels(&sprites, own, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(
            tiles.iter().map(Option::is_some).collect::<Vec<_>>(),
            [true, false, true, true, false, true]
        );
        let bottom_left = tiles[3].unwrap();
        assert_eq!(
            texels(&sprites, bottom_left, 0),
            [(0, 2), (1, 2), (0, 3), (1, 3)]
        );
        assert!(sprites.translucent[tiles[2].unwrap() as usize]);
        assert!(sprites.opaque[bottom_left as usize]);
        // Animated, as one row of tiles: two frames, the grid's top and then its bottom half;
        // per tile, each cell is the column of a tile's frames, which here is the same.
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        for per_tile in [false, true] {
            let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
            let tiles = sprites.insert_tiles(name, (3, 1), per_tile).unwrap();
            let first = tiles[0].unwrap();
            assert_eq!(texels(&sprites, first, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
            assert_eq!(texels(&sprites, first, 1), [(0, 2), (1, 2), (0, 3), (1, 3)]);
            assert_eq!(sprites.animations.len(), 2, "two non-empty tiles");
        }
        // Not a grid of square tiles.
        assert!(sprites.insert_tiles(name, (4, 1), false).is_err());
        assert!(sprites.insert_tiles(name, (3, 3), false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn custom_item_texture_ids_load_with_their_animation() {
        let root = std::env::temp_dir().join(format!("sunrise-sprite-cit-{}", std::process::id()));
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), 16, 48);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..16 * 48).flat_map(|_| [200, 10, 10, 255]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites
            .insert("sunrise_cit:minecraft/optifine/cit/a")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The missing texture would not animate: this one is the file, in three frames.
        assert_eq!(sprites.animations.len(), 1);
        assert_eq!(sprites.animations[0].frame_count(), 3);
    }
    #[test]
    fn custom_item_textures_skip_the_disk_cache() {
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-cit-disk-{}", std::process::id()));
        // A realistic root: the cache's fingerprint walks the `textures` trees.
        let vanilla = root.join("assets/minecraft/textures/item/stick.png");
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        let write = |path: &Path, red: u8| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|_| [red, 0, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write(&vanilla, 1);
        write(&file, 10);
        let id = "sunrise_cit:minecraft/optifine/cit/a";
        let red = |sprites: &Sprites, layer: u32| sprites.native[layer as usize].1[0][0];
        let mut first = Sprites::with_disk_cache(&crate::resource_source::ResourceRoot::directory(
            root.clone(),
        ));
        let layer = first.insert(id).unwrap();
        assert_eq!(red(&first, layer), 10);
        first.save_disk_cache();
        // The pack's file changes; `textures/` does not, so a cached copy would still match.
        write(&file, 20);
        let mut second = Sprites::with_disk_cache(
            &crate::resource_source::ResourceRoot::directory(root.clone()),
        );
        let layer = second.insert(id).unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(red(&second, layer), 20);
    }
    #[test]
    fn custom_item_textures_mip_as_items() {
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-cit-mip-{}", std::process::id()));
        let write = |rel: &str| {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            // Any pixels: whether a texture mips as an item is part of the shared-layer key.
            let pixels: Vec<u8> = (0..256)
                .flat_map(|i| [9, 9, 9, if i % 2 == 0 { 255 } else { 0 }])
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("assets/a/textures/block/x.png");
        write("assets/a/textures/item/x.png");
        write("assets/minecraft/optifine/cit/x.png");
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let block = sprites.insert("a:block/x").unwrap();
        let item = sprites.insert("a:item/x").unwrap();
        let cit = sprites
            .insert("sunrise_cit:minecraft/optifine/cit/x")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The same pixels share a layer only with the same mipmap settings: the CIT texture's
        // are the item's.
        assert_ne!(block, item);
        assert_eq!(cit, item);
    }
    #[test]
    fn prefetched_textures_take_the_layers_inserting_alone_gives() {
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-prefetch-{}", std::process::id()));
        let png = |path: &str, side: u32, seed: u8| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(file).unwrap(), side, side);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..side * side)
                .flat_map(|i| {
                    [
                        seed,
                        i as u8,
                        (i >> 8) as u8,
                        if i % 3 == 0 { 0 } else { 255 },
                    ]
                })
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        for i in 0..40u8 {
            png(
                &format!("assets/minecraft/textures/block/b{i}.png"),
                16 << (i % 3),
                i,
            );
        }
        // Two textures alike share a layer; a tile outside `textures/`, a broken file and a
        // missing one take the paths they take without prefetching.
        png("assets/minecraft/textures/block/same.png", 16, 3);
        png("assets/minecraft/optifine/ctm/glass/0.png", 32, 77);
        fs::write(
            root.join("assets/minecraft/textures/block/broken.png"),
            b"no png",
        )
        .unwrap();
        let names: Vec<String> = (0..40)
            .map(|i| format!("minecraft:block/b{}", (i * 7) % 40))
            .chain(["minecraft:block/same", "minecraft:block/broken"].map(String::from))
            .chain(["minecraft:block/missing", "minecraft:block/b3"].map(String::from))
            .collect();
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        let load = |prefetch: bool| {
            let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
            if prefetch {
                sprites.prefetch(names.iter().map(String::as_str).chain([tile]));
            }
            let mut layers: Vec<u32> = names
                .iter()
                .map(|name| sprites.insert(name).unwrap())
                .collect();
            layers.push(sprites.insert_file(tile).unwrap());
            sprites.finish();
            (
                layers,
                sprites.layers,
                sprites.pixels,
                sprites.translucent,
                sprites.size,
            )
        };
        let alone = load(false);
        assert_eq!(load(true), alone);
        // The textures alike and the broken one's missing texture: fewer layers than names.
        assert!(alone.1.values().max().unwrap() + 1 < names.len() as u32);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn identical_still_textures_share_a_layer() {
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-share-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, red: u8| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|i| [red, i as u8, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("one.png", 10);
        write("copy.png", 10);
        write("other.png", 20);
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let one = sprites.insert("a:block/one").unwrap();
        let copy = sprites.insert("a:block/copy").unwrap();
        let other = sprites.insert("a:block/other").unwrap();
        // The same pixels as an item sprite mip differently: a layer of its own.
        fs::create_dir_all(root.join("assets/a/textures/item")).unwrap();
        fs::copy(
            directory.join("one.png"),
            root.join("assets/a/textures/item/one.png"),
        )
        .unwrap();
        let item = sprites.insert("a:item/one").unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!((one, copy, other, item), (0, 0, 1, 2));
        assert_eq!((sprites.count(), sprites.layers.len()), (3, 4));
        assert_eq!((sprites.opaque.len(), sprites.mips.len()), (3, 3));
        sprites.finish();
        assert_eq!(sprites.pixels.len(), 3 * 16 * 16 * 4);
    }
    #[test]
    fn a_texture_a_connected_rule_matches_keeps_a_layer_of_its_own() {
        // A resource pack's transparent placeholders: a model's layer and a tile another
        // connected-texture rule chains onto (`matchTiles=./19.png`) have the same pixels. The
        // rule finds its faces by layer, so the one it names can't share the others'.
        let root =
            std::env::temp_dir().join(format!("sunrise-sprite-apart-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0; 16 * 16 * 4])
                .unwrap();
        };
        for name in ["placeholder.png", "named.png", "again.png"] {
            write(name);
        }
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites.keep_apart(["a:block/named".to_owned()]);
        let placeholder = sprites.try_insert("a:block/placeholder").unwrap();
        let named = sprites.try_insert("a:block/named").unwrap();
        let again = sprites.try_insert("a:block/again").unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The named one alone; the others still share.
        assert_ne!(named, placeholder);
        assert_eq!(again, placeholder);
        assert_eq!(sprites.count(), 2);
    }
    #[test]
    fn a_32px_only_atlas_does_not_add_an_extra_mipmap() {
        let mut sprites = Sprites {
            native: vec![
                (32, Arc::new(vec![vec![255; 32 * 32 * 4]]), 0),
                (32, Arc::new(vec![vec![128; 32 * 32 * 4]]), 0),
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 32));
        for level in 0..=4 {
            assert_eq!(
                sprite_mipmap_limit(level, sprites.size, sprites.minimum_size, 6),
                level
            );
        }
        sprites
            .native
            .push((16, Arc::new(vec![vec![255; 16 * 16 * 4]]), 0));
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 16));
        assert_eq!(
            sprite_mipmap_limit(0, sprites.size, sprites.minimum_size, 6),
            1
        );
    }

    #[test]
    fn hud_animation_resampling_uses_the_actual_frame_size() {
        for size in [16, 32, 64] {
            let animation = test_animation(size, 0);
            let resized = animation.resized_pixels(1, 16, 16);
            let last = (15 * size / 16) as u8;
            assert_eq!(&resized[15 * 4..16 * 4], &[last, 0, 99, 255]);
            assert_eq!(&resized[255 * 4..256 * 4], &[last, last, 99, 255]);
        }
    }

    #[test]
    fn native_fire_sprites_register_full_animation_sequences() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/fire_0.png")
            .exists()
        {
            return;
        }
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        for name in ["fire_0", "fire_1", "soul_fire_0", "soul_fire_1"] {
            let layer = sprites.insert(&format!("minecraft:block/{name}")).unwrap();
            let animation = sprites
                .animations
                .iter()
                .find(|a| a.layer == layer)
                .expect("fire must be animated");
            assert_ne!(
                animation.pixels(animation.frame(0)),
                animation.pixels(animation.frame(1))
            );
            assert_ne!(
                animation.resized_pixels(animation.frame(0), 16, 16),
                animation.resized_pixels(animation.frame(1), 16, 16)
            );
            assert_eq!(
                animation.pixels(animation.frame(0)).len(),
                (SPRITE_SIZE * SPRITE_SIZE * 4) as usize
            );
        }
    }

    #[test]
    fn native_32_pixel_animation_frames_keep_every_texel() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-sprite-detail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("assets/test/textures");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("flow.png");
        let mut pixels = Vec::new();
        for y in 0..64u8 {
            for x in 0..32u8 {
                pixels.extend([x, y, 0, 255]);
            }
        }
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 32, 64);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(path.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        sprites.insert("test:flow").unwrap();
        sprites.finish();
        let size = sprites.size;
        let initial = sprites.pixels.clone();
        let animation = sprites.animations[0].clone();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            size, 32,
            "half a flowing-water frame must retain 16 texels, not 8"
        );
        assert_eq!(initial, pixels[..32 * 32 * 4]);
        assert_eq!(animation.pixels(animation.frame(1)), &pixels[32 * 32 * 4..]);
    }

    #[test]
    fn animation_boundaries_and_repeated_frames_do_not_depend_on_render_rate() {
        let animation = SpriteAnimation {
            layer: 0,
            materials: [None, None],
            frames: Arc::new(vec![vec![], vec![], vec![]]),
            clip: Arc::new(AnimationMetadata::from_json(r#"{"frames":[{"index":2,"time":8},{"index":1,"time":3},{"index":1,"time":2},0]}"#).unwrap().clip(3).unwrap().unwrap()),
        };
        for (tick, frame) in [
            (0, 2),
            (7, 2),
            (8, 1),
            (10, 1),
            (11, 1),
            (12, 1),
            (13, 0),
            (14, 2),
            (29, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(animation.frame(tick), frame);
        }
    }

    #[test]
    fn interpolated_animations_blend_towards_the_next_frame() {
        let animation = test_clip(16, 0, r#"{"frametime":4,"interpolate":true}"#);
        let blend = |frame, next, millis| FrameBlend {
            frame,
            next,
            millis,
        };
        for (tick, expected) in [
            (0, blend(0, 0, 0)),
            (1, blend(0, 1, 250)),
            (3, blend(0, 1, 750)),
            (4, blend(1, 1, 0)),
            (5, blend(1, 0, 250)),
            (8, blend(0, 0, 0)),
        ] {
            assert_eq!(animation.blend(tick), expected, "tick {tick}");
        }
        let stepped = test_clip(16, 0, r#"{"frametime":4}"#);
        assert_eq!(stepped.blend(1), blend(0, 0, 0));
        assert_eq!(stepped.blend(5), blend(1, 1, 0));
    }
    /// A scratch resource folder with a 2 x 2 sheet whose pixels are red, green, blue and
    /// white (row by row, 16 texels each), and the blocks atlas `blocks_atlas` over it.
    fn atlas_root(tag: &str, blocks_atlas: &str, items_atlas: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("sunrise-atlas-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/block");
        fs::create_dir_all(&textures).unwrap();
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255; 4],
        ];
        let pixels: Vec<u8> = (0..32 * 32)
            .flat_map(|i| colors[(i / 32 / 16) * 2 + (i % 32) / 16])
            .collect();
        let mut encoder =
            png::Encoder::new(File::create(textures.join("sheet.png")).unwrap(), 32, 32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let atlases = root.join("assets/minecraft/atlases");
        fs::create_dir_all(&atlases).unwrap();
        fs::write(atlases.join("blocks.json"), blocks_atlas).unwrap();
        if let Some(items) = items_atlas {
            fs::write(atlases.join("items.json"), items).unwrap();
        }
        root
    }
    fn quadrant(sprite: &str, x: u8, y: u8) -> String {
        format!(
            r#"{{"sources":[{{"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,"divisor_y":2,"regions":[{{"sprite":"{sprite}","x":{x},"y":{y},"width":1,"height":1}}]}}]}}"#
        )
    }
    #[test]
    fn an_unstitched_sprite_has_its_pixels_not_the_missing_texture() {
        let root = atlas_root("unstitch", &quadrant("demo:block/green", 1, 0), None);
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/green").unwrap();
        let frame = &sprites.native[layer as usize].1[0];
        assert_eq!(sprites.native[layer as usize].0, 16);
        assert!(frame.chunks(4).all(|p| p == [0, 255, 0, 255]));
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_later_unstitch_replaces_a_texture_file_of_the_same_id() {
        let blocks = r#"{"sources":[{"type":"minecraft:directory","source":"block","prefix":"block/"},
            {"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,"divisor_y":2,
             "regions":[{"sprite":"demo:block/dup","x":1,"y":1,"width":1,"height":1}]}]}"#;
        let root = atlas_root("replace", blocks, None);
        let textures = root.join("assets/demo/textures/block");
        fs::copy(textures.join("sheet.png"), textures.join("dup.png")).unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/dup").unwrap();
        assert_eq!(sprites.native[layer as usize].0, 16);
        assert!(
            sprites.native[layer as usize].1[0]
                .chunks(4)
                .all(|p| p == [255; 4])
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn the_items_atlas_names_a_sprite_before_the_blocks_atlas() {
        let root = atlas_root(
            "items-first",
            &quadrant("demo:block/shared", 0, 0),
            Some(&quadrant("demo:block/shared", 0, 1)),
        );
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/shared").unwrap();
        assert!(
            sprites.native[layer as usize].1[0]
                .chunks(4)
                .all(|p| p == [0, 0, 255, 255])
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_changed_atlas_definition_does_not_serve_the_cached_sprite() {
        let root = atlas_root("cache", &quadrant("demo:block/cut", 0, 0), None);
        let load = |root: &std::path::Path| {
            let mut sprites = Sprites::with_disk_cache(&ResourceRoot::directory(root.to_owned()));
            let layer = sprites.insert("demo:block/cut").unwrap();
            sprites.save_disk_cache();
            sprites.native[layer as usize].1[0][..4].to_vec()
        };
        assert_eq!(load(&root), [255, 0, 0, 255]);
        assert_eq!(load(&root), [255, 0, 0, 255]);
        // A longer definition, so the change shows in the file's size whatever the clock.
        fs::write(
            root.join("assets/minecraft/atlases/blocks.json"),
            format!("{} ", quadrant("demo:block/cut", 1, 1)),
        )
        .unwrap();
        assert_eq!(load(&root), [255; 4]);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_sprite_too_wide_for_its_square_frames_is_the_missing_texture() {
        // A 32768 x 1 sheet cut whole: its square frame would be 32768 x 32768 texels.
        let root = std::env::temp_dir().join(format!("sunrise-wide-cut-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let textures = root.join("assets/demo/textures/block");
        let atlases = root.join("assets/minecraft/atlases");
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(&atlases).unwrap();
        let mut encoder =
            png::Encoder::new(File::create(textures.join("wide.png")).unwrap(), 32768, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![200; 32768 * 4])
            .unwrap();
        fs::write(
            atlases.join("blocks.json"),
            r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/wide",
                "regions":[{"sprite":"demo:block/strip","x":0,"y":0,"width":1,"height":1}]}]}"#,
        )
        .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/strip").unwrap();
        // The missing texture: 16 x 16, black in its top left quarter.
        assert_eq!(sprites.native[layer as usize].0, SPRITE_SIZE);
        assert_eq!(sprites.native[layer as usize].1[0][..4], [0, 0, 0, 255]);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_paletted_sprite_reads_palettes_directly_below_textures() {
        let root = std::env::temp_dir().join(format!("sunrise-palette-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, width: u32, height: u32, pixels: &[u8]| {
            let file = root.join("assets/demo/textures").join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(file).unwrap(), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        // Two columns of key colours, recoloured to the target palette's two colours.
        write("block/base.png", 2, 1, &[10, 20, 30, 255, 40, 50, 60, 255]);
        write("trim/key.png", 2, 1, &[10, 20, 30, 255, 40, 50, 60, 255]);
        write("trim/red.png", 2, 1, &[200, 0, 0, 255, 0, 200, 0, 255]);
        let atlases = root.join("assets/minecraft/atlases");
        fs::create_dir_all(&atlases).unwrap();
        fs::write(
            atlases.join("blocks.json"),
            r#"{"sources":[{"type":"minecraft:paletted_permutations","textures":["demo:block/base"],
                "palette_key":"demo:trim/key","permutations":{"red":"demo:trim/red"}}]}"#,
        )
        .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/base_red").unwrap();
        assert_eq!(sprites.native[layer as usize].0, 2);
        assert_eq!(
            sprites.native[layer as usize].1[0],
            [
                200, 0, 0, 255, 0, 200, 0, 255, 200, 0, 0, 255, 0, 200, 0, 255
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_failed_items_cut_leaves_the_blocks_sprite() {
        // The items atlas cuts a region outside the sheet; the blocks atlas has a good cut.
        let outside = r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,
            "regions":[{"sprite":"demo:block/both","x":2,"y":0,"width":1,"height":1}]}]}"#;
        let root = atlas_root(
            "fallthrough",
            &quadrant("demo:block/both", 1, 1),
            Some(outside),
        );
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/both").unwrap();
        assert!(
            sprites.native[layer as usize].1[0]
                .chunks(4)
                .all(|p| p == [255; 4])
        );
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_texture_no_atlas_lists_is_the_missing_texture() {
        // A filter removed the file's id; the file is still there, but no atlas holds it.
        let atlas = r#"{"sources":[{"type":"minecraft:directory","source":"block","prefix":"block/"},
            {"type":"minecraft:filter","pattern":{"path":"block/sheet"}}]}"#;
        let root = atlas_root("filtered", atlas, None);
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/sheet").unwrap();
        assert_eq!(sprites.native[layer as usize].0, SPRITE_SIZE);
        assert_eq!(sprites.native[layer as usize].1[0][..4], [0, 0, 0, 255]);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_renamed_sprite_has_the_winning_resources_pixels_and_metadata() {
        // `single` renames the sheet's file; its `.mcmeta` animates it in two frames of 32 x 16.
        let atlas = r#"{"sources":[{"type":"minecraft:single","resource":"demo:block/sheet","sprite":"demo:block/alias"}]}"#;
        let root = atlas_root("rename", atlas, None);
        fs::write(
            root.join("assets/demo/textures/block/sheet.png.mcmeta"),
            r#"{"animation":{"width":32,"height":16}}"#,
        )
        .unwrap();
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert("demo:block/alias").unwrap();
        assert_eq!(sprites.native[layer as usize].1.len(), 2);
        assert_eq!(sprites.native[layer as usize].1[0][..4], [255, 0, 0, 255]);
        assert_eq!(sprites.native[layer as usize].1[1][..4], [0, 0, 255, 255]);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_file_read_on_its_own_never_brings_back_an_unlisted_model_sprite() {
        // The atlas filters the sheet out: no model material of that id exists.
        let atlas = r#"{"sources":[{"type":"minecraft:directory","source":"block","prefix":"block/"},
            {"type":"minecraft:filter","pattern":{"path":"block/sheet"}}]}"#;
        let root = atlas_root("raw-read", atlas, None);
        let load = |raw: bool| {
            let mut sprites = Sprites::with_disk_cache(&ResourceRoot::directory(root.clone()));
            let layer = if raw {
                sprites.insert_limited("demo:block/sheet", 1 << 20).unwrap()
            } else {
                sprites.insert("demo:block/sheet").unwrap()
            };
            sprites.save_disk_cache();
            (
                sprites.native[layer as usize].0,
                sprites.layers.contains_key("demo:block/sheet"),
            )
        };
        // Read on its own it is the file: 32 texels a side, and no model layer of that name.
        assert_eq!(load(true), (32, false));
        // The warm cache doesn't turn that into the model sprite, which is the missing texture.
        assert_eq!(load(false), (SPRITE_SIZE, true));
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn the_fluid_lookup_takes_the_blocks_atlas_sprite_not_the_items_atlas() {
        let root = atlas_root(
            "blocks-only",
            &quadrant("demo:block/both", 0, 0),
            Some(&quadrant("demo:block/both", 1, 1)),
        );
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = sprites.insert_in_blocks("demo:block/both").unwrap();
        assert!(
            sprites.native[layer as usize].1[0]
                .chunks(4)
                .all(|p| p == [255, 0, 0, 255])
        );
        let mut model = Sprites::new(ResourceSource::directory(root.clone()));
        let layer = model.insert("demo:block/both").unwrap();
        assert!(
            model.native[layer as usize].1[0]
                .chunks(4)
                .all(|p| p == [255; 4])
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A scratch tree with `sheet.png` (red, green, blue, white quadrants) and the two atlases'
    /// definitions, each cutting the given `(sprite, x, y)` quadrants of it.
    fn cut_atlases(
        tag: &str,
        items: &[(&str, u8, u8)],
        blocks: &[(&str, u8, u8)],
    ) -> std::path::PathBuf {
        let definition = |cuts: &[(&str, u8, u8)]| {
            let regions: Vec<String> = cuts
                .iter()
                .map(|(sprite, x, y)| {
                    format!(r#"{{"sprite":"{sprite}","x":{x},"y":{y},"width":1,"height":1}}"#)
                })
                .collect();
            format!(
                r#"{{"sources":[{{"type":"minecraft:unstitch","resource":"demo:block/sheet",
                "divisor_x":2,"divisor_y":2,"regions":[{}]}}]}}"#,
                regions.join(",")
            )
        };
        atlas_root(tag, &definition(blocks), Some(&definition(items)))
    }
    fn first_texel(sprites: &Sprites, layer: u32) -> [u8; 4] {
        sprites.native[layer as usize].1[0][..4].try_into().unwrap()
    }
    #[test]
    fn a_model_and_a_fluid_naming_one_id_each_get_their_own_atlas_sprite() {
        // `both` is white in the items atlas and red in the blocks atlas; `items_only` is blue.
        let root = cut_atlases(
            "model-fluid",
            &[("demo:block/both", 1, 1), ("demo:block/items_only", 0, 1)],
            &[("demo:block/both", 0, 0)],
        );
        for model_first in [true, false] {
            let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
            let (model, fluid) = if model_first {
                let model = sprites.insert("demo:block/both").unwrap();
                (model, sprites.insert_in_blocks("demo:block/both").unwrap())
            } else {
                let fluid = sprites.insert_in_blocks("demo:block/both").unwrap();
                (sprites.insert("demo:block/both").unwrap(), fluid)
            };
            assert_eq!(
                first_texel(&sprites, model),
                [255; 4],
                "model first: {model_first}"
            );
            assert_eq!(
                first_texel(&sprites, fluid),
                [255, 0, 0, 255],
                "model first: {model_first}"
            );
            // The name mods and the fluid renderer look up is the blocks atlas's sprite.
            assert_eq!(sprites.layers["demo:block/both"], fluid);
            // An id only the items atlas lists: the blocks lookup misses, the model finds it.
            let miss = sprites.insert_in_blocks("demo:block/items_only").unwrap();
            assert_eq!(first_texel(&sprites, miss), [0, 0, 0, 255]);
            let found = sprites.insert("demo:block/items_only").unwrap();
            assert_eq!(first_texel(&sprites, found), [0, 0, 255, 255]);
        }
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn a_broken_file_read_on_its_own_leaves_the_model_sprite_in_place() {
        let root = cut_atlases("raw-broken", &[], &[("demo:block/cut", 0, 0)]);
        let mut sprites = Sprites::new(ResourceSource::directory(root.clone()));
        let model = sprites.insert("demo:block/cut").unwrap();
        // A readable file that is no PNG, at the same id.
        fs::write(
            root.join("assets/demo/textures/block/cut.png"),
            b"not a png",
        )
        .unwrap();
        let raw = sprites.insert_limited("demo:block/cut", 1 << 20).unwrap();
        assert_ne!(raw, model);
        assert_eq!(
            first_texel(&sprites, raw),
            [0, 0, 0, 255],
            "the missing texture"
        );
        assert_eq!(sprites.files["demo:block/cut"], raw);
        assert_eq!(sprites.layers["demo:block/cut"], model);
        assert_eq!(first_texel(&sprites, model), [255, 0, 0, 255]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A 16 x 16 sprite of one colour, as a mod's source makes it.
    fn flat(sprite: &str, rgb: [u8; 3]) -> mod_api::SpriteSourceOutput {
        mod_api::SpriteSourceOutput::Pixels {
            sprite: mod_api::Identifier::parse(sprite).unwrap(),
            width: 16,
            height: 16,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat(256),
        }
    }
    const OWN: &str = r#"{"sources":[{"type":"srcsprites:own"}]}"#;

    /// What a mod's sources make in the blocks atlas is published in the block arrays though no
    /// model names it, whether it made pixels or named a texture; an id both atlases list keeps
    /// two sprites, the blocks atlas's found by name and the items atlas's as the model material;
    /// and what the items atlas alone lists is no blocks sprite.
    #[test]
    fn a_mods_sprites_are_published_by_atlas_whether_or_not_a_model_names_them() {
        let root = atlas_root("mod-published", OWN, Some(OWN));
        let source = ResourceSource::directory(root.clone());
        let mods =
            crate::atlas_sources::test_mods::Mods::new(&["srcsprites:own"], |atlas, _, _, _| {
                match atlas.path() {
                    "blocks" => vec![
                        flat("srcsprites:pixels", [1, 2, 3]),
                        flat("srcsprites:same", [10, 0, 0]),
                        mod_api::SpriteSourceOutput::Texture {
                            sprite: mod_api::Identifier::parse("srcsprites:alias").unwrap(),
                            texture: mod_api::Identifier::parse("demo:block/sheet").unwrap(),
                        },
                    ],
                    _ => vec![
                        flat("srcsprites:same", [20, 0, 0]),
                        flat("srcsprites:items_only", [30, 0, 0]),
                    ],
                }
            });
        crate::atlas_sources::shared(&source, &mods);
        let mut sprites = Sprites::new(source);
        sprites.insert_mod_sprites().unwrap();
        let blocks = |sprites: &Sprites, name: &str| first_texel(sprites, sprites.layers[name]);
        assert_eq!(blocks(&sprites, "srcsprites:pixels"), [1, 2, 3, 255]);
        assert_eq!(
            blocks(&sprites, "srcsprites:alias"),
            [255, 0, 0, 255],
            "the sheet's own first texel: a texture no model names"
        );
        assert_eq!(blocks(&sprites, "srcsprites:same"), [10, 0, 0, 255]);
        assert!(!sprites.layers.contains_key("srcsprites:items_only"));
        // A model naming the id gets the items atlas's sprite; the blocks atlas's stays.
        let model = sprites.insert("srcsprites:same").unwrap();
        assert_eq!(first_texel(&sprites, model), [20, 0, 0, 255]);
        assert_eq!(blocks(&sprites, "srcsprites:same"), [10, 0, 0, 255]);
        let items_only = sprites.insert("srcsprites:items_only").unwrap();
        assert_eq!(first_texel(&sprites, items_only), [30, 0, 0, 255]);
        assert!(!sprites.layers.contains_key("srcsprites:items_only"));
        fs::remove_dir_all(&root).unwrap();
    }

    /// A sprite a mod's source removes is the missing texture where a model names it.
    #[test]
    fn a_sprite_a_mods_source_removes_is_missing() {
        let both = r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/sheet","divisor_x":2,
            "divisor_y":2,"regions":[{"sprite":"demo:block/cut","x":0,"y":0,"width":1,"height":1},
            {"sprite":"demo:block/kept","x":1,"y":0,"width":1,"height":1}]},
            {"type":"srcsprites:own"}]}"#;
        let root = atlas_root("mod-removed", both, None);
        let source = ResourceSource::directory(root.clone());
        let mods = crate::atlas_sources::test_mods::Mods::new(&["srcsprites:own"], |_, _, _, _| {
            vec![mod_api::SpriteSourceOutput::Remove {
                sprite: mod_api::Identifier::parse("demo:block/cut").unwrap(),
            }]
        });
        crate::atlas_sources::shared(&source, &mods);
        let mut sprites = Sprites::new(source);
        let cut = sprites.insert("demo:block/cut").unwrap();
        assert_eq!(
            first_texel(&sprites, cut),
            [0, 0, 0, 255],
            "the missing texture"
        );
        let kept = sprites.insert("demo:block/kept").unwrap();
        assert_eq!(first_texel(&sprites, kept), [0, 255, 0, 255]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// Code answers a definition differently with no file changed: a warm cache serves what the
    /// listing of that load gave, never an older answer, for pixels changed, a sprite removed and
    /// an alias whose texture is missing (the failed origin leaves the next atlas's sprite).
    #[test]
    fn a_warm_cache_never_serves_what_the_mods_answered_in_an_earlier_load() {
        let root = atlas_root("mod-warm", OWN, Some(OWN));
        let load = |outputs: Vec<mod_api::SpriteSourceOutput>| {
            let resources = ResourceRoot::directory(root.clone());
            let outputs = Arc::new(outputs);
            let mods = crate::atlas_sources::test_mods::Mods::new(
                &["srcsprites:own"],
                move |atlas, _, _, _| {
                    if atlas.path() == "items" {
                        Vec::new()
                    } else {
                        outputs.to_vec()
                    }
                },
            );
            crate::atlas_sources::shared(resources.source(), &mods);
            let mut sprites = Sprites::with_disk_cache(&resources);
            let layer = sprites.insert("srcsprites:a").unwrap();
            sprites.save_disk_cache();
            first_texel(&sprites, layer)
        };
        let alias = |texture: &str| mod_api::SpriteSourceOutput::Texture {
            sprite: mod_api::Identifier::parse("srcsprites:a").unwrap(),
            texture: mod_api::Identifier::parse(texture).unwrap(),
        };
        assert_eq!(load(vec![flat("srcsprites:a", [1, 2, 3])]), [1, 2, 3, 255]);
        assert_eq!(load(vec![flat("srcsprites:a", [1, 2, 3])]), [1, 2, 3, 255]);
        assert_eq!(
            load(vec![flat("srcsprites:a", [9, 9, 9])]),
            [9, 9, 9, 255],
            "other pixels, the same files"
        );
        assert_eq!(load(vec![alias("demo:block/sheet")]), [255, 0, 0, 255]);
        assert_eq!(
            load(vec![alias("demo:block/not_there")]),
            [0, 0, 0, 255],
            "an alias to nothing is the missing texture, not the sheet's cached pixels"
        );
        assert_eq!(
            load(Vec::new()),
            [0, 0, 0, 255],
            "a sprite no source lists any more is missing"
        );
        assert_eq!(load(vec![flat("srcsprites:a", [1, 2, 3])]), [1, 2, 3, 255]);
        assert_eq!(
            load(vec![
                flat("srcsprites:a", [1, 2, 3]),
                mod_api::SpriteSourceOutput::Remove {
                    sprite: mod_api::Identifier::parse("srcsprites:a").unwrap(),
                },
            ]),
            [0, 0, 0, 255],
            "listed in this load and removed in it: gone, not the cached pixels"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// The items atlas lists the id with a cut that does not finish, so the blocks atlas's sprite
    /// of the mod's answer is the model's material; a warm cache follows that answer when it
    /// changes with no file changed.
    #[test]
    fn a_failed_items_origin_leaves_the_mods_blocks_sprite_in_a_warm_cache() {
        let outside = r#"{"sources":[{"type":"minecraft:unstitch","resource":"demo:block/sheet",
            "divisor_x":2,"regions":[{"sprite":"srcsprites:a","x":2,"y":0,"width":1,"height":1}]}]}"#;
        let root = atlas_root("mod-failed-items", OWN, Some(outside));
        let load = |rgb: [u8; 3]| {
            let resources = ResourceRoot::directory(root.clone());
            let mods = crate::atlas_sources::test_mods::Mods::new(
                &["srcsprites:own"],
                move |_, _, _, _| vec![flat("srcsprites:a", rgb)],
            );
            crate::atlas_sources::shared(resources.source(), &mods);
            let mut sprites = Sprites::with_disk_cache(&resources);
            let layer = sprites.insert("srcsprites:a").unwrap();
            sprites.save_disk_cache();
            first_texel(&sprites, layer)
        };
        assert_eq!(load([1, 2, 3]), [1, 2, 3, 255]);
        assert_eq!(load([1, 2, 3]), [1, 2, 3, 255]);
        assert_eq!(load([4, 5, 6]), [4, 5, 6, 255]);
        fs::remove_dir_all(&root).unwrap();
    }
}
