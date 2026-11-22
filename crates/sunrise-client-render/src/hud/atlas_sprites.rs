use super::*;

/// How the GUI atlas publishes one of its ids.
pub(super) enum GuiPublished {
    /// The sprite is packed under this name, with the nine-slice scaling its file gave it.
    Packed(String, Option<crate::ui::NineSlice>),
    /// The list removed it or its origin didn't finish.
    Missing,
}
/// The GUI atlas's answer for one id ([`HudAtlas::gui_lookup`]).
#[derive(Clone, Copy, Debug)]
pub enum GuiSprite {
    /// The sprite the id names, with the nine-slice scaling of the file it is (none for a
    /// generated sprite, a live texture or any sprite without that metadata: it stretches).
    Present {
        rect: Rect,
        scaling: Option<crate::ui::NineSlice>,
    },
    /// The id is listed, and its sprite was removed or its origin didn't finish.
    Missing,
}
/// The nine-slice scaling a sprite file's `.png.mcmeta` gives (none: it stretches).
fn nine_slice_of(root: &ResourceSource, file: &str) -> Option<crate::ui::NineSlice> {
    let text = root.read_to_string(&format!("{file}.mcmeta")).ok()?;
    crate::packs::first_json(text.as_bytes())
        .ok()
        .as_ref()
        .and_then(crate::ui::parse_nine_slice)
}
/// What [`atlas_sprites`] found.
#[derive(Default)]
pub(super) struct AtlasSprites {
    /// The sprites to pack, by packed name.
    pub(super) images: Vec<PackedImage>,
    /// The packed names that animate.
    pub(super) animated: Vec<String>,
    /// The packed names to drop.
    pub(super) dropped: HashSet<String>,
    /// The render flags of the sprites added.
    pub(super) flags: HashMap<String, TextureFlags>,
    /// The resource file each GUI sprite added from a file takes its animation from.
    pub(super) gui_files: HashMap<String, String>,
    /// The GUI atlas's own answer for the names a model or a removal makes ambiguous: the packed
    /// name its pixels are under, or none for a sprite its list removed or failed.
    pub(super) gui: Vec<(String, GuiPublished)>,
    /// The packed names no GUI lookup of an unlisted id answers with: the model atlases' sprites
    /// and the walked files a GUI entry stands on.
    pub(super) non_gui: HashSet<String>,
}
/// What the atlas definitions (`atlases/*.json`) change in the texture files already packed:
/// the sprites to add (generated ones such as the armour trim palettes and `unstitch` regions,
/// and files listed under another id), the animated ones' names, and the packed names to drop.
///
/// A model's material is one finished sprite, the one the item masks resolve: the items atlas's
/// first, then the blocks atlas's. The walked file of its name stands only when that is the
/// winning origin; a rename, a cut or a palette replaces it, and a walked block or item texture
/// no atlas keeps is dropped, as the model baker would find it missing.
///
/// The GUI atlas's final list decides its sprites the same way, under the names the GUI is looked
/// up by: a sprite `minecraft:container/slot` is packed as `gui/sprites/container/slot` where the
/// walk packed it so (or by its plain id where that is walked), a mod's `mymod:panel` under its
/// plain id, and a walked `gui/sprites/*` file no entry keeps is dropped. Where a model's sprite
/// has the same packed name the GUI's sprite is packed apart (`GUI_KEY`) and found through
/// `HudAtlas::gui`, which also answers for a GUI sprite its list removed or failed.
pub(super) fn atlas_sprites(root: &ResourceSource, packed: &[PackedImage]) -> AtlasSprites {
    use crate::atlas_sources::{self, Materials, SpriteOrigin};
    let have: HashSet<&str> = packed.iter().map(|i| i.0.as_str()).collect();
    let materials = Materials::new(root);
    let listing = materials.listing();
    let ids: Vec<&str> = materials.ids().into_iter().collect();
    // Each id's winner, all at once: `None` where no origin finishes, `Some(None)` where the
    // file already walked stands.
    let winners: HashMap<&str, Option<Option<atlas_sources::Material>>> = ids
        .iter()
        .copied()
        .zip(crate::par::par_map(&ids, |id| {
            let name = id.strip_prefix("minecraft:").unwrap_or(id);
            materials.resolve(id, |origin| match origin {
                // The walked file stands if its frames tile it, as a finished sprite's do.
                SpriteOrigin::File(texture)
                    if texture == id && have.contains(name) && walked_frames_fit(root, texture) =>
                {
                    Some(None)
                }
                // A file under a walked folder is packed under its own name or was too large
                // for the atlas: either way it is not added again.
                SpriteOrigin::File(texture)
                    if texture.strip_prefix("minecraft:") == Some(name)
                        && walked_minecraft_texture(name) =>
                {
                    None
                }
                origin => atlas_sources::finish_origin(root, origin).map(Some),
            })
        }))
        .collect();
    let model_names: HashSet<&str> = winners
        .keys()
        .map(|id| id.strip_prefix("minecraft:").unwrap_or(id))
        .collect();
    let mut dropped = HashSet::new();
    let mut out = Vec::new();
    let mut animated = Vec::new();
    let mut flags = HashMap::new();
    let mut gui_files = HashMap::new();
    let mut gui_table: Vec<(String, GuiPublished)> = Vec::new();
    let mut non_gui: HashSet<String> = model_names.iter().map(|name| (*name).to_owned()).collect();
    let gui = listing.sprites("minecraft:gui");
    let mut finished: Vec<(String, atlas_sources::Material, bool)> = Vec::new();
    if !materials.is_bare() {
        for (name, ..) in packed {
            let path = name.split_once(':').map_or(name.as_str(), |(_, p)| p);
            let winner = winners
                .get(atlas_sources::qualified(name).as_str())
                .and_then(|w| w.as_ref());
            let stands = matches!(winner, Some(None));
            let replaced = matches!(winner, Some(Some(_)));
            if replaced || (!stands && (path.starts_with("block/") || path.starts_with("item/"))) {
                dropped.insert(name.clone());
            }
        }
        finished.extend(winners.into_iter().filter_map(|(id, winner)| {
            Some((
                id.strip_prefix("minecraft:").unwrap_or(id).to_owned(),
                winner??,
                true,
            ))
        }));
    }
    // The GUI atlas owns every id its final list names, and every id a filter or `Remove` took
    // out of it, by that id: its sprite is the walked file the entry itself is, else packed under
    // a name of its own (`#gui:<id>`: `#` is in no identifier, so no model's or mod's name can be
    // it). A walked file is dropped only for the id whose own key it is, and not while another
    // entry stands on it; a file of the same name another owner packed (a registered atlas's
    // projection, a live texture's) is no GUI id's to drop.
    if listing.defines("minecraft:gui") {
        let mut claimed: HashSet<String> = HashSet::new();
        let mut standing: HashSet<String> = HashSet::new();
        let mut dropping: Vec<String> = Vec::new();
        let removed = listing.removed("minecraft:gui").map(|id| (id, None));
        for (id, origin) in gui
            .iter()
            .map(|(id, origin)| (id, Some(origin)))
            .chain(removed)
        {
            let plain = id.strip_prefix("minecraft:").unwrap_or(id);
            let key = gui_sprite::atlas_key_of_gui_sprite(id);
            let own = (have.contains(key.as_str()) && !model_names.contains(key.as_str()))
                .then(|| key.clone());
            claimed.extend(own.clone());
            let stands = match origin {
                Some(SpriteOrigin::File(texture)) => {
                    let name = texture.strip_prefix("minecraft:").unwrap_or(texture);
                    ([key.as_str(), plain].contains(&name)
                        && have.contains(name)
                        && !model_names.contains(name)
                        && walked_frames_fit(root, texture))
                    .then(|| name.to_owned())
                }
                _ => None,
            };
            if let Some(name) = stands {
                claimed.insert(name.clone());
                standing.insert(name.clone());
                let scaling =
                    nine_slice_of(root, &atlas_sources::texture_resource(&name, "textures"));
                gui_table.push((plain.to_owned(), GuiPublished::Packed(name, scaling)));
                continue;
            }
            dropping.extend(own);
            let material = origin.and_then(|origin| atlas_sources::finish_origin(root, origin));
            let Some(material) = material else {
                gui_table.push((plain.to_owned(), GuiPublished::Missing));
                continue;
            };
            let packed_as = format!("{GUI_KEY}{id}");
            if let Some(file) = &material.resource {
                gui_files.insert(packed_as.clone(), file.clone());
            }
            let scaling = material
                .resource
                .as_deref()
                .and_then(|file| nine_slice_of(root, file));
            gui_table.push((
                plain.to_owned(),
                GuiPublished::Packed(packed_as.clone(), scaling),
            ));
            finished.push((packed_as, material, false));
        }
        dropped.extend(dropping.into_iter().filter(|name| !standing.contains(name)));
        non_gui.extend(standing);
        // A walked GUI sprite the list names nowhere is no sprite of the GUI atlas.
        for (name, ..) in packed {
            let path = name.split_once(':').map_or(name.as_str(), |(_, p)| p);
            if path.starts_with("gui/sprites/")
                && !claimed.contains(name.as_str())
                && !model_names.contains(name.as_str())
            {
                dropped.insert(name.clone());
            }
        }
    }
    {
        for (name, material, model) in finished {
            let path = name.split_once(':').map_or(name.as_str(), |(_, p)| p);
            let image = match &material.resource {
                Some(file) => {
                    // Animated strips keep their first frame and animate, like the packed
                    // files, under any folder: vanilla animates every sprite of these atlases
                    // (a server pack's `wynn/...` as well as `item/...`).
                    let mcmeta = root.read_to_string(&format!("{file}.mcmeta")).ok();
                    let animation = material.animation.as_ref();
                    let image = if animation.is_some()
                        || (model && (path.starts_with("block/") || path.starts_with("item/")))
                    {
                        match first_frame(material.image, animation) {
                            Ok(image) => image,
                            Err(error) => {
                                log::warn!("skipped GUI texture {file}: {error}");
                                continue;
                            }
                        }
                    } else {
                        material.image
                    };
                    if animation.is_some() {
                        animated.push(name.clone());
                    }
                    // The winner's own render flags (blur, clamp), as the walk reads them.
                    if let Some(texture_flags) =
                        mcmeta.as_deref().and_then(TextureFlags::from_mcmeta)
                    {
                        flags.insert(name.clone(), texture_flags);
                    }
                    image
                }
                None => material.image,
            };
            out.push((name, image.0, image.1, image.2));
        }
        out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    }
    // The other atlases' generated sprites (GUI sprites, particles...), by the first origin
    // that loads, in the atlases' order; they load at once (a content pack's atlases list
    // tens of thousands of files).
    let mut names: Vec<(String, Vec<SpriteOrigin>)> = Vec::new();
    let mut slots = HashMap::new();
    for (atlas, sprites) in listing.atlases() {
        if matches!(
            atlas.as_str(),
            "minecraft:blocks" | "minecraft:items" | "minecraft:gui"
        ) {
            continue;
        }
        for (id, origin) in sprites {
            let name = id.strip_prefix("minecraft:").unwrap_or(id).to_owned();
            // A model's sprite is the model atlases' winner above and a GUI sprite the GUI
            // atlas's: not another atlas's output.
            if matches!(origin, SpriteOrigin::File(_))
                || have.contains(name.as_str())
                || model_names.contains(name.as_str())
                || gui.contains_key(id)
            {
                continue;
            }
            let slot = *slots.entry(name.clone()).or_insert_with(|| {
                names.push((name, Vec::new()));
                names.len() - 1
            });
            names[slot].1.push(origin.clone());
        }
    }
    let load = |texture: &str| {
        decode_resource(root, &atlas_sources::texture_resource(texture, "textures")).ok()
    };
    out.extend(
        crate::par::par_map(&names, |(name, origins)| {
            origins
                .iter()
                .find_map(|origin| atlas_sources::generate(origin, &load))
                .map(|(w, h, pixels)| (name.clone(), w, h, pixels))
        })
        .into_iter()
        .flatten(),
    );
    AtlasSprites {
        images: out,
        animated,
        dropped,
        flags,
        gui_files,
        gui: gui_table,
        non_gui,
    }
}
