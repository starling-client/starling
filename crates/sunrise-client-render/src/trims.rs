//! Armour trims on worn armour: `EquipmentLayerRenderer`'s trim layer. A
//! trim's texture is `textures/trims/entity/<layer>/<pattern>.png` recoloured
//! by `PalettedTextureManager`: every pixel whose colour is in the texture's
//! base palette (its `palette` metadata section, `trim_base`) takes the colour
//! at the same index of the material's palette (`textures/palettes/trim/*`),
//! unless the equipment asset's `trim_overrides` pick another texture or
//! palette (iron armour with an iron trim uses `trim/iron_darker`).
use crate::resource_source::ResourceSource;
use serde_json::Value;
use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;
use sunrise_assets::palette::apply_palette;
use sunrise_core::{identifier::Identifier, nbt::Tag};
use sunrise_protocol::registry::Dynamic;

/// Runtime trim slots and the completed ordered requests for their current atlas.
/// A frame with more requests than slots keeps the same final residents as the
/// sequential LRU pass, without decoding and evicting them again every frame.
#[derive(Default)]
pub struct TrimSlots {
    slots: Vec<(String, usize)>,
    atlas_revision: u64,
    prepared: Option<Vec<TrimTexture>>,
}

impl TrimSlots {
    /// The renderer was rebuilt: runtime pixels must be uploaded again.
    pub fn forget(&mut self) {
        self.atlas_revision = 0;
        self.prepared = None;
    }

    pub fn prepare(
        &mut self,
        wanted: Vec<TrimTexture>,
        root: &ResourceSource,
        atlas: &mut crate::hud::HudAtlas,
        patches: &mut Vec<crate::font::Patch>,
    ) {
        if self.atlas_revision != atlas.revision {
            for (name, _) in self.slots.drain(..) {
                atlas.sprites.remove(&name);
            }
            self.atlas_revision = atlas.revision;
            self.prepared = None;
        }
        let before = self.slots.len();
        self.slots
            .retain(|(name, _)| atlas.sprites.contains_key(name));
        if before != self.slots.len() {
            self.prepared = None;
        }
        if self.prepared.as_ref() == Some(&wanted) {
            return;
        }
        let mut complete = true;
        for t in &wanted {
            let name = t.sprite();
            if let Some(i) = self.slots.iter().position(|e| e.0 == name) {
                let e = self.slots.remove(i);
                self.slots.push(e);
                continue;
            }
            let Some((w, h, rgba)) = t.generate(root) else {
                complete = false;
                continue;
            };
            let slot =
                match (0..crate::hud::TRIM_SLOTS).find(|i| !self.slots.iter().any(|e| e.1 == *i)) {
                    Some(free) => free,
                    None => {
                        let (old, slot) = self.slots.remove(0);
                        atlas.sprites.remove(&old);
                        slot
                    }
                };
            let Some(area) = atlas.sprites.get(&format!("sunrise/trim{slot}")).copied() else {
                complete = false;
                continue;
            };
            if w > area.w || h > area.h {
                complete = false;
                continue;
            }
            let (aw, ah) = (atlas.width as f32, atlas.height as f32);
            let (x, y) = ((area.u0 * aw).round(), (area.v0 * ah).round());
            patches.push(crate::font::Patch {
                x: x as u32,
                y: y as u32,
                width: w,
                height: h,
                rgba,
            });
            atlas.sprites.insert(
                name.clone(),
                crate::hud::Rect {
                    u0: area.u0,
                    v0: area.v0,
                    u1: (x + w as f32) / aw,
                    v1: (y + h as f32) / ah,
                    w,
                    h,
                },
            );
            self.slots.push((name, slot));
        }
        // Failed or not-yet-available textures retain their per-frame retries.
        self.prepared = complete.then_some(wanted);
    }
}

/// An RGBA image: width, height, pixels.
pub type Image = (u32, u32, Vec<u8>);

/// `minecraft:` added to an identifier without a namespace.
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// `assets/<namespace>/<folder>/<path><suffix>` of an identifier.
fn asset_path(folder: &str, id: &str, suffix: &str) -> String {
    let id = qualified(id);
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", &id));
    format!("assets/{namespace}/{folder}/{path}{suffix}")
}

/// `EquipmentClientInfo.TrimOverride`: when the trim matches (`when`'s
/// material and pattern, each optional), use `texture` instead of the
/// pattern's asset id and `palette` (none when absent) as the palette.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrimOverride {
    pub material: Option<String>,
    pub pattern: Option<String>,
    pub texture: Option<String>,
    pub palette: Option<String>,
}
impl TrimOverride {
    /// An equipment asset's `trim_overrides` list.
    pub fn list(equipment: &Value) -> Vec<Self> {
        let Some(list) = equipment.get("trim_overrides").and_then(Value::as_array) else {
            return Vec::new();
        };
        let id = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(qualified);
        list.iter()
            .filter_map(|o| {
                let when = o.get("when")?;
                Some(Self {
                    material: id(when, "material"),
                    pattern: id(when, "pattern"),
                    texture: id(o, "texture"),
                    palette: id(o, "palette"),
                })
            })
            .collect()
    }
    /// `TrimPredicate.matches`.
    fn matches(&self, material: &str, pattern: &str) -> bool {
        self.material.as_deref().is_none_or(|m| m == material)
            && self.pattern.as_deref().is_none_or(|p| p == pattern)
    }
}

/// One synced registry keyed by entry name, as `MappedRegistry`'s id map is: the asset or
/// palette id the server sent, none for an entry of a known pack, whose data is the client's own
/// files.
type SyncedEntries = HashMap<String, Option<String>>;

/// The `trim_pattern` (asset id) and `trim_material` (palette id) registries.
#[derive(Clone, Debug, Default)]
pub struct TrimRegistry {
    /// The resources' entries.
    patterns: HashMap<String, String>,
    materials: HashMap<String, String>,
    /// The connected server's registries (patterns, materials): when set, only their entries
    /// resolve, as `ClientPacketListener`'s registry access holds only what the server sent.
    synced: Option<(SyncedEntries, SyncedEntries)>,
}

/// `id` in `local`, or in `synced` when the connection has one.
fn lookup<'a>(
    local: &'a HashMap<String, String>,
    synced: Option<&'a SyncedEntries>,
    id: &str,
) -> Option<&'a str> {
    match synced {
        None => local.get(id),
        Some(entries) => match entries.get(id)? {
            Some(sent) => Some(sent),
            None => local.get(id),
        },
    }
    .map(String::as_str)
}

/// Decode one synced trim entry (`TrimPattern.DIRECT_CODEC`, `TrimMaterial.DIRECT_CODEC`,
/// `NetworkRegistryLoadTask.loadFromNetwork`). A known-pack entry has no data and resolves
/// from the client's own files; supplied data must decode cleanly and never falls back.
fn decode_synced_entry(
    registry: &str,
    key: &str,
    has_decal: bool,
    name: &str,
    data: Option<&Tag>,
) -> Result<(String, Option<String>), String> {
    let id = Identifier::parse(name)
        .map_err(|error| format!("{registry}/{name}: {error}"))?
        .as_str()
        .to_owned();
    let Some(data) = data else {
        return Ok((id, None));
    };
    if !matches!(data, Tag::Compound(_)) {
        return Err(format!("{registry}/{id}: not a compound"));
    }
    let target = match data.get(key) {
        Some(Tag::String(raw)) => {
            let raw = raw
                .to_string()
                .map_err(|_| format!("{registry}/{id}: {key}: invalid UTF-8"))?;
            Identifier::parse(&raw)
                .map_err(|error| format!("{registry}/{id}: {key}: {error}"))?
                .as_str()
                .to_owned()
        }
        Some(_) => return Err(format!("{registry}/{id}: {key}: a string is required")),
        None => return Err(format!("{registry}/{id}: {key}: an identifier is required")),
    };
    match data.get("description") {
        Some(Tag::String(_) | Tag::Compound(_) | Tag::List(_)) => {}
        Some(_) => {
            return Err(format!(
                "{registry}/{id}: description: a text component is required"
            ));
        }
        None => return Err(format!("{registry}/{id}: description is required")),
    }
    if has_decal
        && let Some(decal) = data.get("decal")
        && !matches!(decal, Tag::Byte(_))
    {
        return Err(format!("{registry}/{id}: decal: a boolean is required"));
    }
    Ok((id, Some(target)))
}

impl TrimRegistry {
    /// Take the registries a server sent as the ones that resolve.
    /// A malformed entry refuses the whole load (`RegistryDataLoader.load`), leaving the
    /// previous registries unchanged.
    pub fn set_synced(&mut self, registries: &Dynamic) -> Result<(), String> {
        let read = |registry: &str, key: &str, has_decal: bool| -> Result<SyncedEntries, String> {
            registries
                .registries
                .get(registry)
                .into_iter()
                .flatten()
                .map(|(name, data)| {
                    decode_synced_entry(registry, key, has_decal, name, data.as_ref())
                })
                .collect()
        };
        let patterns = read("minecraft:trim_pattern", "asset_id", true)?;
        let materials = read("minecraft:trim_material", "palette_id", false)?;
        self.synced = Some((patterns, materials));
        Ok(())
    }
    /// A resource reload's entries, keeping this connection's synced registries.
    pub fn reloaded(&mut self, mut fresh: Self) {
        fresh.synced = self.synced.take();
        *self = fresh;
    }
    fn pattern(&self, id: &str) -> Option<&str> {
        lookup(&self.patterns, self.synced.as_ref().map(|s| &s.0), id)
    }
    fn material(&self, id: &str) -> Option<&str> {
        lookup(&self.materials, self.synced.as_ref().map(|s| &s.1), id)
    }
    /// Every `data/<namespace>/trim_pattern` and `trim_material` entry.
    pub fn load(root: &ResourceSource) -> Self {
        let mut out = Self::default();
        for (folder, key, map) in [
            ("trim_pattern", "asset_id", &mut out.patterns),
            ("trim_material", "palette_id", &mut out.materials),
        ] {
            for file in root.list_under("data", &[folder]) {
                let Some((namespace, name)) = file
                    .strip_prefix("data/")
                    .and_then(|name| name.split_once(&format!("/{folder}/")))
                else {
                    continue;
                };
                let Some(name) = name.strip_suffix(".json") else {
                    continue;
                };
                let Some(value) = root
                    .read(&file)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                else {
                    continue;
                };
                if let Some(value) = value.get(key).and_then(Value::as_str) {
                    map.insert(format!("{namespace}:{name}"), qualified(value));
                }
            }
        }
        out
    }
    #[cfg(test)]
    fn of(patterns: &[(&str, &str)], materials: &[(&str, &str)]) -> Self {
        let map = |l: &[(&str, &str)]| {
            l.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        Self {
            patterns: map(patterns),
            materials: map(materials),
            synced: None,
        }
    }
}

/// The texture and palette of one trim layer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrimTexture {
    /// `<namespace>:trims/entity/<layer>/<path>`, under `textures/`.
    pub texture: String,
    /// The target palette under `textures/palettes/`; none draws the
    /// texture's own colours.
    pub palette: Option<String>,
}
impl TrimTexture {
    /// `TrimTextureKey.getOrPrepareTexture`: the trim (`material`,
    /// `pattern` registry ids) on the `layer` (`humanoid`,
    /// `humanoid_leggings`...) of an equipment asset with `overrides`.
    pub fn resolve(
        registry: &TrimRegistry,
        overrides: &[TrimOverride],
        material: &str,
        pattern: &str,
        layer: &str,
    ) -> Option<Self> {
        let (material, pattern) = (qualified(material), qualified(pattern));
        let mut texture = registry.pattern(&pattern)?.to_owned();
        let mut palette = Some(registry.material(&material)?.to_owned());
        if let Some(o) = overrides.iter().find(|o| o.matches(&material, &pattern)) {
            if let Some(t) = &o.texture {
                texture.clone_from(t);
            }
            palette.clone_from(&o.palette);
        }
        let (namespace, path) = texture.split_once(':')?;
        Some(Self {
            texture: format!("{namespace}:trims/entity/{layer}/{path}"),
            palette,
        })
    }
    /// The atlas sprite name the generated texture is stored under.
    pub fn sprite(&self) -> String {
        format!(
            "sunrise/trim/{}/{}",
            self.texture,
            self.palette.as_deref().unwrap_or("-")
        )
    }
    /// `PalettedTextureManager.prepareSlot`: the texture with its base
    /// palette's colours swapped for the target palette's (`PaletteMapping`;
    /// unchanged when a palette is missing or the sizes differ). None when
    /// the texture itself is missing.
    pub fn generate(&self, root: &ResourceSource) -> Option<Image> {
        let path = asset_path("textures", &self.texture, ".png");
        let (w, h, pixels) = crate::hud::decode_bytes(&root.read(&path).ok()?).ok()?;
        let Some(palette) = &self.palette else {
            return Some((w, h, pixels));
        };
        let base = root
            .read(&format!("{path}.mcmeta"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|m| {
                m.get("palette")?
                    .get("base_palette")?
                    .as_str()
                    .map(str::to_owned)
            });
        let load = |id: &str| {
            root.read(&asset_path("textures/palettes", id, ".png"))
                .ok()
                .and_then(|bytes| crate::hud::decode_bytes(&bytes).ok())
                .map(|(_, _, p)| argb(&p))
        };
        let mut mapped = argb(&pixels);
        if let (Some(key), Some(target)) = (base.as_deref().and_then(load), load(palette)) {
            // Different sizes: PaletteMapping.NONE, the texture as it is.
            let _ = apply_palette(&mut mapped, &key, &target);
        }
        let out = mapped
            .iter()
            .flat_map(|p| {
                let [a, r, g, b] = p.to_be_bytes();
                [r, g, b, a]
            })
            .collect();
        Some((w, h, out))
    }
}

fn argb(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}

/// The largest trim texture of the resources (the size of the atlas's trim
/// slots), at least vanilla's 64x32.
pub fn slot_size(root: &ResourceSource) -> (u32, u32) {
    let mut size = (64, 32);
    for file in root.list_under("assets", &["textures/trims/entity"]) {
        if file.ends_with(".png")
            && let Some((w, h)) = png_size(root, &file)
        {
            size = (size.0.max(w.min(512)), size.1.max(h.min(512)));
        }
    }
    size
}

/// The width and height in a PNG's IHDR chunk.
fn png_size(source: &ResourceSource, path: &str) -> Option<(u32, u32)> {
    let head: [u8; 24] = source.read_head(path, 24).ok()?.try_into().ok()?;
    let be = |i: usize| u32::from_be_bytes(head[i..i + 4].try_into().unwrap());
    (&head[12..16] == b"IHDR").then(|| (be(16), be(20)))
}

#[cfg(test)]
mod slots_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> TrimRegistry {
        TrimRegistry::of(
            &[("minecraft:coast", "minecraft:coast")],
            &[
                ("minecraft:iron", "minecraft:trim/iron"),
                ("minecraft:gold", "minecraft:trim/gold"),
            ],
        )
    }

    #[test]
    fn material_palette_and_layer_folder() {
        let t = TrimTexture::resolve(
            &registry(),
            &[],
            "minecraft:gold",
            "minecraft:coast",
            "humanoid_leggings",
        )
        .unwrap();
        assert_eq!(t.texture, "minecraft:trims/entity/humanoid_leggings/coast");
        assert_eq!(t.palette.as_deref(), Some("minecraft:trim/gold"));
        // Unknown registry entries draw nothing.
        assert!(
            TrimTexture::resolve(
                &registry(),
                &[],
                "minecraft:wood",
                "minecraft:coast",
                "humanoid"
            )
            .is_none()
        );
    }

    #[test]
    fn equipment_overrides_pick_the_darker_palette() {
        let iron = serde_json::json!({"trim_overrides": [
            {"palette": "minecraft:trim/iron_darker", "when": {"material": "minecraft:iron"}},
        ]});
        let overrides = TrimOverride::list(&iron);
        let resolve =
            |m| TrimTexture::resolve(&registry(), &overrides, m, "coast", "humanoid").unwrap();
        assert_eq!(
            resolve("iron").palette.as_deref(),
            Some("minecraft:trim/iron_darker")
        );
        assert_eq!(
            resolve("gold").palette.as_deref(),
            Some("minecraft:trim/gold")
        );
        // An override without a palette leaves the texture's own colours.
        let plain = TrimOverride::list(&serde_json::json!({"trim_overrides": [
            {"texture": "minecraft:other", "when": {"pattern": "minecraft:coast"}},
        ]}));
        let t = TrimTexture::resolve(&registry(), &plain, "gold", "coast", "humanoid").unwrap();
        assert_eq!(t.texture, "minecraft:trims/entity/humanoid/other");
        assert_eq!(t.palette, None);
    }

    #[test]
    fn vanilla_trims_are_recoloured() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3");
        if !root.join("assets/minecraft/textures/trims").is_dir() {
            return;
        }
        let registry = TrimRegistry::load(&ResourceSource::directory(root.clone()));
        let t = TrimTexture::resolve(
            &registry,
            &[],
            "minecraft:gold",
            "minecraft:coast",
            "humanoid",
        )
        .unwrap();
        let (w, h, gold) = t
            .generate(&ResourceSource::directory(root.clone()))
            .unwrap();
        assert_eq!((w, h), (64, 32));
        let plain = TrimTexture {
            palette: None,
            ..t.clone()
        };
        let (_, _, grey) = plain
            .generate(&ResourceSource::directory(root.clone()))
            .unwrap();
        // Every opaque pixel of the grey pattern takes a gold palette colour.
        let gold_palette = argb(
            &crate::hud::decode(&root.join("assets/minecraft/textures/palettes/trim/gold.png"))
                .unwrap()
                .2,
        );
        let mut opaque = 0;
        for (g, c) in grey.chunks(4).zip(gold.chunks(4)) {
            if g[3] != 0 {
                opaque += 1;
                let c = u32::from_be_bytes([0xFF, c[0], c[1], c[2]]);
                assert!(gold_palette.contains(&c), "{c:08x}");
            }
        }
        assert!(opaque > 0);
        assert_eq!(
            slot_size(&ResourceSource::directory(root.clone())),
            (64, 32)
        );
    }

    /// A server's pack trims render with the asset and palette ids it synced, and only the
    /// synced entries resolve; a known pack's entry (no data sent) reads the client's own files.
    #[test]
    fn a_synced_pack_trim_resolves_from_its_asset_ids() {
        let entry = |name: &str, field: &str, id: &str| {
            (
                name.to_owned(),
                Some(Tag::compound([
                    (field, Tag::String(id.into())),
                    ("description", Tag::String(name.into())),
                ])),
            )
        };
        let synced = Dynamic {
            registries: [
                (
                    "minecraft:trim_pattern".to_owned(),
                    vec![
                        ("minecraft:coast".to_owned(), None),
                        entry("test:zigzag", "asset_id", "test:zig"),
                    ],
                ),
                (
                    "minecraft:trim_material".to_owned(),
                    vec![entry("test:jade", "palette_id", "test:trim/jade")],
                ),
            ]
            .into(),
            ..Default::default()
        };
        let mut registry = registry();
        registry.set_synced(&synced).unwrap();
        let t =
            TrimTexture::resolve(&registry, &[], "test:jade", "test:zigzag", "humanoid").unwrap();
        assert_eq!(t.texture, "test:trims/entity/humanoid/zig");
        assert_eq!(t.palette.as_deref(), Some("test:trim/jade"));
        // Not in the server's registry: nothing, though the client's files hold it.
        assert!(
            TrimTexture::resolve(&registry, &[], "minecraft:gold", "test:zigzag", "humanoid")
                .is_none()
        );
        // A known pack's pattern has no data in the packet: the client's own entry.
        let known =
            TrimTexture::resolve(&registry, &[], "test:jade", "minecraft:coast", "humanoid")
                .unwrap();
        assert_eq!(known.texture, "minecraft:trims/entity/humanoid/coast");
    }

    /// A malformed synced trim entry refuses the whole load instead of falling back to the
    /// client's own files (`RegistryDataLoader.load`).
    #[test]
    fn malformed_synced_trim_refuses_load_without_fallback() {
        let cases = [
            (
                "minecraft:trim_material",
                "minecraft:gold",
                Tag::compound([("description", Tag::String("Gold".into()))]),
            ),
            (
                "minecraft:trim_material",
                "minecraft:gold",
                Tag::compound([
                    ("palette_id", Tag::Int(1)),
                    ("description", Tag::String("Gold".into())),
                ]),
            ),
            (
                "minecraft:trim_material",
                "minecraft:gold",
                Tag::compound([
                    ("palette_id", Tag::String("..:bad".into())),
                    ("description", Tag::String("Gold".into())),
                ]),
            ),
            (
                "minecraft:trim_material",
                "minecraft:gold",
                Tag::compound([("palette_id", Tag::String("minecraft:trim/gold".into()))]),
            ),
            ("minecraft:trim_material", "minecraft:gold", Tag::Int(0)),
            (
                "minecraft:trim_pattern",
                "minecraft:coast",
                Tag::compound([("description", Tag::String("Coast".into()))]),
            ),
            (
                "minecraft:trim_pattern",
                "minecraft:coast",
                Tag::compound([
                    ("asset_id", Tag::String("minecraft:coast".into())),
                    ("description", Tag::String("Coast".into())),
                    ("decal", Tag::String("yes".into())),
                ]),
            ),
            (
                "minecraft:trim_pattern",
                "..:bad",
                Tag::compound([
                    ("asset_id", Tag::String("minecraft:coast".into())),
                    ("description", Tag::String("Coast".into())),
                ]),
            ),
        ];
        let mut registry = registry();
        for (kind, name, data) in cases {
            let bad = Dynamic {
                registries: [(kind.to_owned(), vec![(name.to_owned(), Some(data))])].into(),
                ..Default::default()
            };
            assert!(
                registry.set_synced(&bad).is_err(),
                "expected refusal for {kind}/{name}"
            );
            assert!(
                registry.synced.is_none(),
                "failed load must leave synced state unchanged"
            );
        }
    }
}
