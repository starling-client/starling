//! Shader packs: the one picked on the Shader Packs page, with its profile
//! (`Graphics::shader_choice`; `HORIZON_SHADER_PACK` for development), runs `shaderpacks/<name>`
//! (a folder or a zip beside `options.txt`) over the world; Horizon's own packs go to
//! `game/modding/native_shaders.rs`. The programs of the current dimension's folder are loaded when the
//! dimension or the choice changes (spec 1.2).

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::{PackInputs, PlayerStatus};
use horizon_client::graphics::{AUTO_PROFILE, light_shader_pack};
use horizon_client::mod_list::ModList;
use horizon_client::shader_options::ShaderOptions;
use horizon_client::text::Text;
use horizon_shader_api::Api;
use horizon_shaders::{BlockIds, Macros, Pack, Properties, dimension_folder_named, load_post};

use super::native_shaders::{fingerprint, outcome, refused, remember};
use super::{Dim, Game};
use mod_render::render::EntitySpan;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// The values the player chose for pack `name`'s options (`shaderpacks/<name>.txt`).
fn chosen_shader_options(name: &str) -> BTreeMap<String, String> {
    let file = Game::shader_packs_dir().join(horizon_shaders::chosen_file(name));
    std::fs::read_to_string(file)
        .map(|text| horizon_shaders::parse_chosen(&text))
        .unwrap_or_default()
}

/// Pack `name`'s options, screens and names in `language`, with the values the player chose. An
/// option's default is what `profile` runs it at, so the page shows what runs.
pub(super) fn read_shader_options(
    name: &str,
    profile: &str,
    language: &str,
    macros: &Macros,
) -> Option<ShaderOptions> {
    // Native options come from the manifest; the shared screen opens either kind.
    if name.starts_with("horizon:") {
        return None;
    }
    let pack = match Pack::open(&Game::shader_packs_dir().join(name)) {
        Ok(pack) => pack,
        Err(e) => {
            log::warn!("shader pack {name}: {e}");
            return None;
        }
    };
    let mut options = horizon_shaders::options(&pack);
    let lang = horizon_shaders::Lang::load(&pack, language);
    let chosen = chosen_shader_options(name);
    let pack = pack.with_chosen(chosen.clone());
    let text = pack.read_text("shaders.properties").unwrap_or_default();
    let properties = Properties::parse(&text, &horizon_shaders::properties_macros(&pack, macros));
    let screens = horizon_shaders::Screens::parse(&properties, &options);
    let running = picked_values(&properties, &options, name, profile, &BTreeMap::new());
    for option in options.values_mut() {
        if let Some(value) = running.declared(&option.name, &option.default) {
            option.default = value;
        }
    }
    Some(ShaderOptions::new(
        name.to_owned(),
        options,
        screens,
        lang,
        chosen,
    ))
}

/// Saves the values chosen on the Shader Options page; none chosen removes the file.
pub(super) fn write_shader_options(options: &ShaderOptions) {
    let file = Game::shader_packs_dir().join(horizon_shaders::chosen_file(&options.pack));
    let saved = if options.chosen.is_empty() {
        std::fs::remove_file(&file).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(&file, horizon_shaders::chosen_text(&options.chosen))
    };
    if let Err(e) = saved {
        log::warn!("shader pack {}: {}: {e}", options.pack, file.display());
    }
}

/// The pack that runs over the world ("": none): `HORIZON_SHADER_PACK`, else the pick
/// (`active_pack`).
pub(super) fn dev_pack() -> Option<&'static str> {
    static DEV_PACK: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DEV_PACK
        .get_or_init(|| std::env::var("HORIZON_SHADER_PACK").ok())
        .as_deref()
}

pub(super) fn picked_pack(settings: &horizon_client::settings::Settings) -> &str {
    active_pack(
        dev_pack().unwrap_or(&settings.horizon.shader_pack),
        &settings.mods,
    )
}

/// What a renderer readies for the pick, unless it was refused, is missing or its owner is off.
#[derive(Clone, Copy)]
pub(crate) struct PickedPack {
    /// It draws flat where the device can (`pack_draws_flat`): the sections are laid out so.
    pub draws_flat: bool,
    /// It is a shader API 0.2 pack under `HORIZON_HDR_FRAME=float`: the frame is float.
    pub linear: bool,
    /// Under `HORIZON_HDR_FRAME=float`, with no pack available or a 0.2 one: the frame is the
    /// layered float one where HDR output is on (docs/design/hdr-linear-light.md, The HUD and
    /// menus; the renderer decides, `FrameRequest::hdr_float`).
    pub hdr_float: bool,
}
impl PickedPack {
    /// What it asks of the renderer's frame.
    pub(crate) fn frame(self) -> horizon_client::gfx::FrameRequest {
        horizon_client::gfx::FrameRequest {
            linear_pack: self.linear,
            hdr_float: self.hdr_float,
        }
    }
}

pub(super) fn picked_pack_shape(settings: &horizon_client::settings::Settings) -> PickedPack {
    let picked = picked_pack(settings);
    let dir = Game::shader_packs_dir();
    let path = if dev_pack().is_some() {
        dir.join(picked)
    } else {
        horizon_client::shader_pack_list::resolve(&dir, picked).1
    };
    let active = !picked.is_empty()
        && (!super::native_shaders::linear_frame_switch()
            || (path.exists() && outcome(&path) != Some(None)))
        && path
            .file_name()
            .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty());
    let linear = || {
        super::native_shaders::linear_frame_switch()
            && horizon_shader_api::Pack::read_manifest(&path)
                .is_ok_and(|m| m.api == horizon_shader_api::Api::V0_2)
    };
    let linear = active && linear();
    PickedPack {
        draws_flat: active && pack_draws_flat(&path),
        linear,
        hdr_float: (!active || linear) && super::native_shaders::linear_frame_switch(),
    }
}

/// Whether the pack at `path` has passes that draw the terrain flat where the device can,
/// reading translucent quads beside their section's solid ones: a `shaders/` pack, taken to
/// draw terrain, or one of Horizon's own with a `[shadow]` pass.
fn pack_draws_flat(path: &std::path::Path) -> bool {
    !horizon_shader_api::package::is_pack(path)
        || horizon_shader_api::Pack::read_manifest(path).is_ok_and(|m| m.shadow.is_some())
}

// A native mod can bundle a pack as `native-<mod-id>`; its Mods toggle controls it too.
pub(super) fn active_pack<'a>(picked: &'a str, mods: &ModList) -> &'a str {
    if picked.strip_prefix("native-").is_some_and(|id| {
        mods.installed
            .iter()
            .any(|m| m.id == id && m.native.is_some() && mods.disabled.contains(id))
    }) {
        ""
    } else {
        picked
    }
}

/// A selected pack may read resource-pack material maps, whether native or GLSL.
pub(super) fn picks_material_pack(settings: &horizon_client::settings::Settings) -> bool {
    let dev = std::env::var("HORIZON_SHADER_PACK").ok();
    let picked = active_pack(
        dev.as_deref().unwrap_or(&settings.horizon.shader_pack),
        &settings.mods,
    );
    if picked.is_empty() {
        return false;
    }
    let dir = Game::shader_packs_dir();
    let path = if dev.is_some() {
        dir.join(picked)
    } else {
        horizon_client::shader_pack_list::resolve(&dir, picked).1
    };
    path.file_name()
        .is_none_or(|f| !active_pack(&f.to_string_lossy(), &settings.mods).is_empty())
}

/// The ids a pack gives entities and block entities (4.1, 5.3), for the spans of the entity
/// geometry.
#[cfg_attr(test, derive(Default))]
pub(super) struct PackIds {
    entities: HashMap<String, i32>,
    blocks: BlockIds,
    /// `blockEntityId` by block state, filled as states are drawn.
    block_states: RefCell<HashMap<u32, i32>>,
    /// The id of a name the file lacks: 65535, or 0 without the file.
    missing_entity: i32,
    missing_block: i32,
    /// `item.properties` (4.1) and the id of a held item the file lacks, or of an empty hand:
    /// −1, or 0 without the file (5.3).
    items: HashMap<String, i32>,
    missing_item: i32,
    /// `currentRenderedItemId` of each block item (`StateFacts::item` of its block's default
    /// state): that state's `block.properties` id, or 0.
    block_items: HashMap<String, i32>,
    /// Whether the pack has `item.properties` (`currentRenderedItemId` of an item it lacks).
    item_file: bool,
    /// `oldHandLight` (3.2): the first held item and light are those of the brighter hand.
    old_hand_light: bool,
    /// `vignette`: `false` hides the game's vignette over the pack's image.
    vignette: bool,
    /// `oldLighting`: `true` keeps the game's face shading in the block colours.
    old_lighting: bool,
    /// `separateAo`: `true` passes AO factor in vertex alpha and keeps pure tint in RGB.
    pub(super) separate_ao: bool,
}

/// A pack the renderer is making on its own thread (`Renderer::load_shader_pack`): its ids,
/// which entity spans use once it draws.
pub(super) struct Making {
    ids: PackIds,
}

/// What the Auto profile does to a pack on this device. Packs written for a PC spend most of a
/// phone's frame on their pixels, water and shadow map, so it takes one of the pack's own light
/// profiles (Complementary on a Snapdragon 8 Gen 2 at distance 10: 46 ms of GPU on LOW, 28 ms on
/// VERYLOW), limits the shadow and the resolution, and has terrain draw its depth first. Which
/// profile is lightest is read from what the profiles set (`Values::profiles_by_cost`), not from
/// their names, so any pack that has profiles works.
struct AutoProfile {
    /// Place among the pack's profiles, lightest first. A strong phone skips the very lightest
    /// when the pack has three or more (a profile that turns everything off looks like none).
    rank: usize,
    shadow_resolution: f64,
    shadow_distance: f64,
}

impl AutoProfile {
    fn here() -> Self {
        use horizon_client::system_stats::Tier;
        match Tier::here().unwrap_or(Tier::High) {
            Tier::High => Self {
                rank: 1,
                shadow_resolution: 1024.0,
                shadow_distance: 64.0,
            },
            Tier::Mid => Self {
                rank: 0,
                shadow_resolution: 1024.0,
                shadow_distance: 64.0,
            },
            Tier::Low => Self {
                rank: 0,
                shadow_resolution: 512.0,
                shadow_distance: 64.0,
            },
        }
    }
}

/// The pack's option values: the chosen `profile` (`LOW`), then what the player chose on the
/// Shader Options page, which Auto's limits leave alone, then `HORIZON_SHADER_OPTIONS` lists
/// values over it (`shadowDistance=64.0 SHADOW_QUALITY=0 !BLOOM`).
fn picked_values(
    properties: &Properties,
    options: &BTreeMap<String, horizon_shaders::PackOption>,
    name: &str,
    profile: &str,
    chosen: &BTreeMap<String, String>,
) -> horizon_shaders::Values {
    let mut values = horizon_shaders::Values::default();
    if profile == AUTO_PROFILE {
        let auto = AutoProfile::here();
        let ranked = horizon_shaders::Values::profiles_by_cost(properties, options);
        let at = if ranked.len() > 2 { auto.rank } else { 0 };
        match ranked.get(at) {
            Some((found, cost)) => {
                log::info!(
                    "shader pack {name}: Auto picks profile `{found}` (cost {cost:.1}) of {}",
                    ranked
                        .iter()
                        .map(|(n, c)| format!("{n} {c:.1}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                values = horizon_shaders::Values::profile(properties, found).unwrap_or_default();
            }
            None => log::info!("shader pack {name}: Auto finds no profile to start from"),
        }
        for (option, limit) in [
            ("shadowMapResolution", auto.shadow_resolution),
            ("shadowDistance", auto.shadow_distance),
        ] {
            if !chosen.contains_key(option) {
                values.cap(option, limit);
            }
        }
    } else if !profile.is_empty() {
        match horizon_shaders::Values::profile(properties, profile) {
            Some(p) => values = p,
            None => log::warn!("shader pack {name}: no profile `{profile}`"),
        }
    }
    for (option, value) in chosen {
        values.set(option, value);
    }
    if let Ok(list) = std::env::var("HORIZON_SHADER_OPTIONS") {
        values.parse_list(&list);
    }
    if !values.is_empty() {
        log::info!("shader pack {name}: option values {values:?}");
    }
    values
}
impl Game<'_> {
    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(super) fn shader_pack_inputs(
        &mut self,
        camera: glam::DVec3,
        partial: f32,
        sky: &horizon_client::environment::Sky,
    ) -> PackInputs {
        // A `shaders/` pack lights the faces itself unless `oldLighting=true`: sections mesh
        // without the game's face shading (the pack loaded or dropped below shows next frame).
        let pack_lighting = self.pack_ids.as_ref().is_some_and(|ids| !ids.old_lighting);
        if self.streaming.shader_pack_lighting != pack_lighting {
            self.streaming.shader_pack_lighting = pack_lighting;
            self.all_sections_changed();
        }
        // Any `shaders/` pack (its `oldLighting` too) draws fluid tops split as Iris does.
        let shader_pack = self.pack_ids.is_some();
        if self.streaming.shader_pack != shader_pack {
            self.streaming.shader_pack = shader_pack;
            self.all_sections_changed();
        }
        let separate_ao = self.pack_ids.as_ref().is_some_and(|ids| ids.separate_ao);
        if self.streaming.separate_ao != separate_ao {
            self.streaming.separate_ao = separate_ao;
            self.all_sections_changed();
        }
        let selected = picked_pack(&self.settings).to_owned();
        let path = self.native_shaders.resolve_path(
            &Self::shader_packs_dir(),
            &selected,
            dev_pack().is_some(),
        );
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let picked = if selected.is_empty() || active_pack(&file, &self.settings.mods).is_empty() {
            String::new()
        } else {
            path.to_string_lossy().into_owned()
        };
        // A new choice, once the options close (each load takes seconds): what ran for the old
        // one goes, of either kind.
        let choice = self
            .settings
            .horizon
            .shader_choice()
            .filter(|(name, _)| !active_pack(name, &self.settings.mods).is_empty());
        if (choice != self.shader_choice || self.native_shaders.picked != picked)
            && self.screen != super::Screen::Options
        {
            self.shader_choice = choice;
            self.native_shaders.select(&picked);
            self.set_translucent_layers(Vec::new());
            self.pack_ids = None;
            self.pack_making = None;
            let scale = self.world_scale();
            let horizon = &self.settings.horizon;
            let (antialias, taa, upscale) = (horizon.fxaa, horizon.taa, horizon.upscaling.ratio());
            let sharpness = horizon.upscaling_sharpness as f32 / 100.0;
            if let Some(renderer) = &mut self.renderer {
                renderer.drop_shader_packs();
                renderer.set_world_post(scale, antialias, taa, upscale, sharpness);
            }
        }
        // A pack whose pipelines were building runs once they are built, or is turned off.
        if let Some((dimension, name, started, report)) =
            self.renderer.as_mut().and_then(|r| r.poll_shader_pack())
        {
            match report {
                Ok(Some((programs, notes))) => {
                    remember(&path, fingerprint(&path), Some(Api::V0_1));
                    for note in notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    log::info!(
                        "shader pack {name}: {} programs ({}) in {} ms",
                        programs.len(),
                        programs.join(" "),
                        started.elapsed().as_millis()
                    );
                    // None could be made (the notes above say why): the world would stay as it
                    // is with the pack still "on".
                    if let Some(making) = self.pack_making.take() {
                        if programs.is_empty() {
                            self.shader_pack_failed(
                                &name,
                                dimension,
                                "none of its programs loaded",
                            );
                        } else {
                            self.pack_ids = Some(making.ids);
                        }
                    }
                }
                Ok(None) => remember(&path, fingerprint(&path), None),
                // Stopped because it isn't wanted any more: not the pack's failure.
                Err(refused) if refused.is_cancelled() => {}
                Err(refused) => {
                    for note in refused.notes {
                        log::warn!("shader pack {name}: {note}");
                    }
                    self.shader_pack_failed(&name, dimension, &refused.reason);
                }
            }
        }
        if picked.is_empty() {
            return PackInputs::default();
        }
        if !horizon_client::sprites::labpbr_maps() {
            horizon_client::sprites::set_labpbr_maps(true);
            self.reload_packs();
            return PackInputs::default();
        }
        let Some((name, profile)) = self.shader_choice.clone() else {
            return PackInputs::default();
        };
        let dimension = match self.dim {
            Dim::Overworld | Dim::Extra(_) => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        if (dev_pack().is_none() && selected.starts_with("horizon:"))
            || horizon_shader_api::package::is_pack(&path)
        {
            // One of Horizon's own packs (game/modding/native_shaders.rs).
            self.tick_native_shader_pack(&path);
        } else if let Some(renderer) = &mut self.renderer
            && renderer.shader_pack_dimension() != Some(dimension)
            && !(super::native_shaders::linear_frame_switch() && refused(&path, fingerprint(&path)))
        {
            let name = name.as_str();
            let started = std::time::Instant::now();
            self.pack_ids = None;
            self.pack_making = None;
            // A pack is not the game's own code: one that makes the shader compiler (or our
            // translation of it) panic is turned off instead of crashing every world load (#3525).
            let loaded = horizon_client::crash::contain(|| {
                match Pack::open(&path).map(|p| p.with_chosen(chosen_shader_options(name))) {
                    Ok(mut pack) => {
                        let mut macros = Macros::standard();
                        renderer.define_pack_macros(&mut macros);
                        // Its conditionals test the pack's options too.
                        let (mut properties, notes) =
                            horizon_shaders::read_properties(&pack, &mut macros);
                        for note in notes {
                            log::warn!("shader pack {name}: {note}");
                        }
                        for note in &properties.notes {
                            log::warn!(
                                "shader pack {name}: shaders.properties:{}: {}",
                                note.line,
                                note.message
                            );
                        }
                        let chosen = chosen_shader_options(name);
                        let values = picked_values(
                            &properties,
                            &horizon_shaders::options(&pack),
                            name,
                            &profile,
                            &chosen,
                        );
                        if !values.is_empty() {
                            pack.set_values(values);
                            // A profile can switch on what shaders.properties tests (`image.*`,
                            // `program.*.enabled`, features): read it again with the values in,
                            // or the stage files and the properties disagree about the options.
                            macros = Macros::standard();
                            renderer.define_pack_macros(&mut macros);
                            properties = horizon_shaders::read_properties(&pack, &mut macros).0;
                        }
                        let separate_ao = properties
                            .get("separateAo")
                            .is_some_and(|v| v.trim() == "true");
                        renderer.set_separate_ao(separate_ao);
                        // On a phone whatever the profile: it pays for itself even on a pack's
                        // highest settings, and the player picks a profile for how it looks.
                        renderer.set_pack_prepass(
                            (cfg!(target_os = "android") || profile == AUTO_PROFILE)
                                && !light_shader_pack(name),
                        );
                        // Lava's faces draw with the solid terrain's program.
                        renderer.set_pack_lava_layers(
                            ["lava_still", "lava_flow"]
                                .iter()
                                .filter_map(|f| {
                                    let name = format!("minecraft:block/{f}");
                                    self.pack.render.sprite_layers.get(&name).map(|&l| l as u16)
                                })
                                .collect(),
                        );
                        let folder = dimension_folder_named(&pack, self.dim.id(), dimension);
                        let inputs = renderer.pack_inputs();
                        let biomes: Vec<String> = world_biomes!(self)
                            .list
                            .iter()
                            .map(|b| b.name.clone())
                            .collect();
                        let resources = self.pack.source.clone();
                        let text = pack.read_text("block.properties").unwrap_or_default();
                        let ids = BlockIds::parse(&Properties::parse(&text, &macros));
                        for note in &ids.notes {
                            log::warn!("shader pack {name}: block.properties: {note}");
                        }
                        let text = pack.read_text("entity.properties");
                        let entities = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "entity",
                        );
                        let text = pack.read_text("item.properties");
                        let items = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "item",
                        );
                        let block_items =
                            block_item_ids(self.res.facts(), self.pack.assets.registry, &ids);
                        let pack_ids = PackIds {
                            block_items,
                            item_file: text.is_some(),
                            items: items.plain(),
                            missing_item: if text.is_some() { -1 } else { 0 },
                            // OQ-21: on unless the pack turns it off.
                            old_hand_light: properties
                                .get("oldHandLight")
                                .is_none_or(|v| v.trim() != "false"),
                            // On unless the pack turns it off (`vignette=false`).
                            vignette: properties
                                .get("vignette")
                                .is_none_or(|v| v.trim() != "false"),
                            // Off unless the pack turns it on (Iris shades no faces by default).
                            old_lighting: properties
                                .get("oldLighting")
                                .is_some_and(|v| v.trim() == "true"),
                            separate_ao,
                            entities: entities.plain(),
                            missing_entity: if text.is_some() { 65535 } else { 0 },
                            missing_block: if ids.is_empty() { 0 } else { 65535 },
                            blocks: ids.clone(),
                            block_states: RefCell::default(),
                        };
                        let started_ids = std::time::Instant::now();
                        let table = terrain_state_ids(self.pack.assets.registry, &ids);
                        log::info!(
                            "shader pack {name}: mc_Entity for {} of {} block states in {} ms",
                            table.chunks(2).filter(|p| p[0] != -1).count(),
                            table.len() / 2,
                            started_ids.elapsed().as_millis()
                        );
                        // The programs are translated and built on the renderer's thread for
                        // packs; the world draws without the pack until they are.
                        let program_folder = folder.clone();
                        let pack_name = name.to_owned();
                        renderer.load_shader_pack(
                            dimension,
                            move || {
                                let mut post = load_post(
                                    &pack,
                                    &program_folder,
                                    &properties,
                                    &macros,
                                    &inputs,
                                );
                                // Textures that name one of the game's: from the resource packs
                                // over the game's own resources.
                                let read = |namespace: &str, path: &str| {
                                    resources.read(&format!("assets/{namespace}/{path}")).ok()
                                };
                                if !post.skipped.is_empty() {
                                    let fell: Vec<&str> =
                                        post.skipped.iter().map(|s| s.name.as_str()).collect();
                                    let uses = |p: &Option<horizon_shaders::PostProgram>| {
                                        p.as_ref().map_or("none", |p| p.name.as_str()).to_owned()
                                    };
                                    log::warn!(
                                        "shader pack {pack_name}: {} programs did not load ({}); \
                                         terrain draws with {}, water with {}, entities with {}, \
                                         a composite pass that fell is skipped",
                                        fell.len(),
                                        fell.join(" "),
                                        uses(&post.terrain),
                                        uses(&post.water),
                                        uses(&post.entities),
                                    );
                                }
                                for note in
                                    horizon_shaders::resolve_resources(&mut post.textures, &read)
                                {
                                    log::warn!("shader pack {pack_name}: {note}");
                                }
                                let names: Vec<&str> = biomes.iter().map(String::as_str).collect();
                                post.custom.set_biomes(&names);
                                post
                            },
                            table,
                            name.to_owned(),
                            started,
                        );
                        self.pack_making = Some(Making { ids: pack_ids });
                    }
                    Err(e) => {
                        remember(&path, fingerprint(&path), None);
                        log::warn!("shader pack {name}: {e}");
                        // Without a pack this can't fail.
                        let _ = renderer.set_shader_pack(dimension, None, &[]);
                    }
                }
                Ok(())
            });
            // A pack that can't run is turned off like one that panics.
            if let Err(error) = loaded.flatten() {
                self.shader_pack_failed(name, dimension, &error);
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.world.light(at.x, at.y, at.z);
        let biome = self
            .world
            .chunks
            .get(&(at.x >> 4, at.z >> 4))
            .map(|c| c.biome(at.x, at.y, at.z));
        let (held_items, held_light) = self.held_for_pack();
        let status = self.pack_player_status();
        let (end_flash, end_flash_direction) = self.end_flash_for_pack(partial);
        let info = biome.map(|b| world_biomes!(self).get(b));
        let dimension = self.remote.as_ref().map(|r| &r.dimension);
        PackInputs {
            // `Level.getDefaultClockTime`: 0 in a dimension type with no default clock (the
            // Nether's has none), as Iris 1.11.7 reads `worldTime` there (probe pack).
            day_time: if dimension.is_some_and(|d| d.default_clock.is_none()) {
                0
            } else {
                self.world.day_time
            },
            rain: lerp(self.world.weather.rain_level),
            thunder: lerp(self.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(self.potions.effects.get("blindness").is_some())),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.ui.hide_gui,
            brightness: self.settings.gamma,
            biome: info.map_or(-1, |b| {
                let names = world_biomes!(self).list.iter().map(|b| b.name.as_str());
                horizon_shaders::biome_pack_id(&b.name, names)
            }),
            biome_known: biome.is_some(),
            biome_category: info.map_or(0, |b| horizon_shaders::biome_category(&b.name)),
            biome_precipitation: info.map_or(0, |b| {
                horizon_shaders::biome_precipitation(b.precipitation, b.temperature)
            }),
            temperature: info.map_or(0.0, |b| b.temperature),
            rainfall: info.map_or(0.0, |b| b.downfall),
            held_items,
            held_light,
            right_handed: !self.settings.main_hand_left,
            on_ground: self.player.as_ref().is_some_and(|p| p.ground.on_ground),
            min_y: self.world.shape.min_y,
            height: self.world.shape.height,
            logical_height: self.world.shape.logical_height,
            has_skylight: self.world.shape.sky_light,
            has_ceiling: self.world.shape.has_ceiling,
            elytra_flying: self.player.as_ref().is_some_and(|p| p.fall_flying),
            cloud_height: horizon_client::gfx::pack_uniforms::cloud_height(sky),
            // Vanilla's drift in float, times its 0.030000001F blocks a tick.
            cloud_time: horizon_client::clouds::drift(
                self.world.game_time,
                f64::from(partial),
                self.pack.clouds.0,
            ) as f32
                * 0.030_000_001,
            mod_params: self.native_shader_params_now(partial),
            status,
            end_flash,
            end_flash_direction,
            ambient_light: dimension.map_or(0.0, |d| d.ambient_light),
            mood: self.ambience.moodiness,
            still_ticks: self.clock.still().then(|| self.visual_ticks()),
            frame_timing: None,
        }
    }

    /// The local player's state for Iris's status uniforms, read as the HUD reads it.
    fn pack_player_status(&mut self) -> PlayerStatus {
        let spectator = self.spectator();
        let survival = !self.creative && !spectator;
        let invisible = self.own_invisible();
        let riding = self.riding.is_some();
        let Some(p) = &mut self.player else {
            return PlayerStatus::default();
        };
        PlayerStatus {
            survival,
            health: p.health.health(),
            max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
            food: p.health.food.food as f32,
            air: p.air as f32,
            armor: p
                .attribute("minecraft:armor")
                .or_else(|_| p.attribute("armor"))
                .unwrap_or(0.0) as f32,
            spectator,
            riding,
            feet_in_water: p.fluids.was_touching_water,
            swimming: p.pose == horizon_client::pose::Pose::Swimming,
            burning: p.fire_ticks > 0 && !spectator,
            hurt: p.health.hurt_animation_time > 0,
            invisible,
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
        }
    }

    /// The world's resolution, of the screen's: the player's, and no more than Auto's while it
    /// runs a pack that isn't a light one.
    pub(super) fn world_scale(&self) -> f32 {
        let chosen = self.settings.horizon.resolution_scale as f32 / 100.0;
        match &self.shader_choice {
            Some((name, profile))
                if profile == AUTO_PROFILE
                    && !light_shader_pack(name)
                    && !horizon_shader_api::package::is_pack(
                        &Self::shader_packs_dir().join(name),
                    ) =>
            {
                chosen.min(horizon_client::graphics::auto_shader_scale())
            }
            _ => chosen,
        }
    }

    /// The Shader Options page opened: the picked pack's options, screens and names.
    pub(super) fn open_shader_options(&mut self) {
        let Some((name, profile)) = self.settings.horizon.shader_choice() else {
            return;
        };
        let mut macros = Macros::standard();
        if let Some(renderer) = &self.renderer {
            renderer.define_pack_macros(&mut macros);
        }
        self.settings.shader_options =
            read_shader_options(&name, &profile, &self.settings.language, &macros);
    }

    /// The Shader Options page closed with changes: they are saved, and the pack loads again.
    pub(super) fn save_shader_options(&mut self) {
        let Some(options) = self.settings.shader_options.take() else {
            return;
        };
        write_shader_options(&options);
        self.pack_ids = None;
        self.pack_making = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.drop_shader_packs();
        }
    }

    /// Turns off the pack `name` after loading it panicked or failed: nothing runs in its place,
    /// it is no longer the picked pack (so the next start does not load it again), and the chat
    /// says so.
    fn shader_pack_failed(&mut self, name: &str, dimension: i32, error: &str) {
        let path = Self::shader_packs_dir().join(name);
        remember(&path, fingerprint(&path), None);
        log::error!("shader pack {name}: turned off: {error}");
        self.pack_ids = None;
        self.pack_making = None;
        if let Some(renderer) = &mut self.renderer {
            // Marks this dimension as loaded, without a pack, so it is not tried again (without
            // a pack this can't fail).
            let _ = renderer.set_shader_pack(dimension, None, &[]);
        }
        if self.settings.horizon.shader_pack == name {
            self.settings.horizon.shader_pack.clear();
            self.settings.save(&Self::options_path());
        }
        self.system_message(Text::colored(
            format!("Shader pack {name} could not be loaded and was turned off"),
            0xFF5555,
        ));
    }

    /// The device was lost with a pack picked: most likely its frames were too long for the
    /// driver, so the rebuilt renderer starts without it and it stays off at the next start.
    pub(super) fn shader_pack_lost_device(&mut self) {
        if self.settings.horizon.shader_pack.is_empty() {
            return;
        }
        let name = std::mem::take(&mut self.settings.horizon.shader_pack);
        log::error!("shader pack {name}: turned off after the graphics device was lost");
        self.pack_ids = None;
        self.pack_making = None;
        self.settings.save(&Self::options_path());
        self.system_message(Text::colored(
            format!("Shader pack {name} was too heavy for this device and was turned off"),
            0xFF5555,
        ));
    }

    /// Whether the game's vignette draws: the option, unless the loaded pack turns it off.
    pub(super) fn vignette_shown(&self) -> bool {
        self.settings.vignette && self.pack_ids.as_ref().is_none_or(|ids| ids.vignette)
    }

    /// `currentRenderedItemId` of a held `item`, [`rendered_item_id`]'s; 0 without a pack.
    pub(super) fn rendered_item_id(&self, item: &str) -> i32 {
        self.pack_ids.as_ref().map_or(0, |ids| {
            rendered_item_id(
                ids.block_items.get(item).copied(),
                ids.items.get(item).copied(),
                ids.item_file,
            )
        })
    }

    /// `heldItemId(2)` and `heldBlockLightValue(2)`: main hand, then off hand (5.2, 5.3).
    fn held_for_pack(&mut self) -> ([i32; 2], [i32; 2]) {
        if self.pack_ids.is_none() {
            return ([0; 2], [0; 2]);
        }
        let lum = self.luminance();
        let Some(ids) = &self.pack_ids else {
            return ([0; 2], [0; 2]);
        };
        let hands = [self.selected, super::shield::OFFHAND_SLOT].map(|slot| {
            let item = self.stack_in(slot).map(|s| &*s.item);
            let id = item.and_then(|i| ids.items.get(i).copied());
            (
                id.unwrap_or(ids.missing_item),
                item.map_or(0, |i| i32::from(lum.of(i))),
            )
        });
        let first = if ids.old_hand_light && hands[1].1 > hands[0].1 {
            hands[1]
        } else {
            hands[0]
        };
        ([first.0, hands[1].0], [first.1, hands[1].1])
    }
}

impl Game<'_> {
    /// Starts a span of the entity geometry at vertex `start` for entity type `name`
    /// (`minecraft:zombie`; "" for one the pack cannot name). Nothing without a pack.
    pub(super) fn mark_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, false);
    }

    /// Starts a span for entity geometry that blends (name tags, shadows).
    pub(super) fn mark_translucent_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, true);
    }

    fn mark_entity_part(&self, start: usize, name: &str, translucent: bool) {
        let id = match &self.pack_ids {
            Some(ids) => ids
                .entities
                .get(name)
                .copied()
                .unwrap_or(ids.missing_entity),
            None if native_spans() => name_id(name),
            None => return,
        };
        self.push_span(start, "ENTITIES", id);
        if let Some(last) = self.entity_spans.borrow_mut().last_mut() {
            last.translucent = translucent;
            if let Some(gpu) = self.skin_frame.borrow_mut().spans.last_mut() {
                gpu.translucent = translucent;
            }
        }
    }

    /// Starts a span for the block entity of block state `state`.
    pub(super) fn mark_block_entity(&self, start: usize, state: u32) {
        let Some(ids) = &self.pack_ids else {
            if native_spans() {
                self.push_span(start, "BLOCK_ENTITIES", state as i32);
            }
            return;
        };
        let id = *ids
            .block_states
            .borrow_mut()
            .entry(state)
            .or_insert_with(|| {
                let registry = self.pack.assets.registry;
                registry
                    .state(horizon_world::registry::BlockStateId(state))
                    .and_then(|s| {
                        ids.blocks
                            .id(&s.name, |k| s.properties.get(k).map(String::as_str))
                    })
                    .unwrap_or(ids.missing_block)
            });
        self.push_span(start, "BLOCK_ENTITIES", id);
        let emission = self
            .res
            .facts()
            .get(horizon_world::registry::BlockStateId(state))
            .emission;
        if let Some(last) = self.entity_spans.borrow_mut().last_mut() {
            last.emissive = emission > 0;
            if let Some(gpu) = self.skin_frame.borrow_mut().spans.last_mut() {
                gpu.emissive = emission > 0;
            }
        }
    }

    /// Starts a span for mod model copies (`WorldDrawContext::draw_model_instances`) under a
    /// pack: block entities, as `BlockEntityDrawContext::model` draws the same model, with the
    /// id of a block the pack doesn't list, since no block owns them. Nothing without a pack.
    pub(super) fn mark_model_copies(&self, start: usize) {
        if let Some(ids) = &self.pack_ids {
            self.push_span(start, "BLOCK_ENTITIES", ids.missing_block);
        }
    }

    /// Starts the span of the local player's model; `hidden` in first person, where only a
    /// pack's shadow pass draws it.
    pub(super) fn mark_player(&self, start: usize, hidden: bool) {
        self.mark_entity(start, "minecraft:player");
        if (self.pack_ids.is_some() || native_spans())
            && let Some(last) = self.entity_spans.borrow_mut().last_mut()
        {
            last.player = true;
            last.hidden = hidden;
            if let Some(gpu) = self.skin_frame.borrow_mut().spans.last_mut() {
                gpu.player = true;
                gpu.hidden = hidden;
            }
        }
    }

    /// Marks vertices `start..end` (one entity) as hidden from the camera, for an entity
    /// Horizon's culling would drop but a pack's shadow pass still draws (`EntitySpan::hide`).
    pub(super) fn hide_spans_from(&self, start: usize, end: usize) -> bool {
        EntitySpan::hide(&mut self.entity_spans.borrow_mut(), start, end)
    }

    /// Starts a span of geometry that is not an entity (`PARTICLES`, `DESTROY`...).
    pub(super) fn mark_stage(&self, start: usize, stage: &str) {
        if self.pack_ids.is_some() || native_spans() {
            self.push_span(start, stage, 0);
        }
    }

    fn push_span(&self, start: usize, stage: &str, id: i32) {
        let mut spans = self.entity_spans.borrow_mut();
        let span = EntitySpan {
            start: start as u32,
            stage: horizon_shaders::render_stage(stage),
            id,
            ..Default::default()
        };
        if self.pack_ids.is_some() && self.skinning() {
            let mut skin = self.skin_frame.borrow_mut();
            let gpu = EntitySpan {
                start: skin.vertices,
                ..span
            };
            match skin.spans.last_mut() {
                Some(last) if last.start == gpu.start => *last = gpu,
                _ => skin.spans.push(gpu),
            }
        }
        // A span that drew nothing gives way to the next.
        match spans.last_mut() {
            Some(last) if last.start == span.start => *last = span,
            _ => spans.push(span),
        }
    }
}

/// `currentRenderedItemId` of an item (Iris 1.11.7, observed with probe packs): a block item
/// (`block`: its block's `block.properties` id, 0 when the file lacks it) reads that, whatever
/// `item.properties` says of it; any other item its `item.properties` id (`item`), and one the
/// file lacks 65535, or 0 when the pack has no `item.properties` (`item_file`).
fn rendered_item_id(block: Option<i32>, item: Option<i32>, item_file: bool) -> i32 {
    block.or(item).unwrap_or(if item_file { 65535 } else { 0 })
}

/// The block items' `currentRenderedItemId`: for each block's default state, the item it gives
/// (`StateFacts::item`) takes the state's id in `ids`, or 0. A block named as its item wins over
/// another block giving that item.
fn block_item_ids(
    facts: &horizon_client::blocks::BlockFacts,
    registry: &horizon_world::registry::BlockRegistry,
    ids: &BlockIds,
) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for (i, state) in facts.states.iter().enumerate() {
        let id = horizon_world::registry::BlockStateId(i as u32);
        if &*state.item == "minecraft:air" || registry.default_state(&state.name) != Some(id) {
            continue;
        }
        let value = registry
            .state(id)
            .and_then(|s| ids.id(&s.name, |k| s.properties.get(k).map(String::as_str)))
            .unwrap_or(0);
        if state.item == state.name {
            out.insert(state.item.to_string(), value);
        } else {
            out.entry(state.item.to_string()).or_insert(value);
        }
    }
    out
}

/// Spans for a native mod (`native_render::RenderNeeds::entity_spans`), without a pack.
fn native_spans() -> bool {
    horizon_client::gfx::native_render::needs().entity_spans
}

/// A span's id for entity type `name` without a pack: stable, so a mod can match runs from one
/// frame to the next.
fn name_id(name: &str) -> i32 {
    // FNV-1a.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash as i32
}

/// Model/fluid material ids by the original state, independent of sprite sharing and
/// multipart model selection. Waterlogged model faces retain their model id; emitted
/// fluid faces use the fluid's state (the source state itself for liquid blocks).
fn terrain_state_ids(
    registry: &horizon_world::registry::BlockRegistry,
    ids: &BlockIds,
) -> Vec<i32> {
    use horizon_world::{
        fluid::{FluidKind, FluidState},
        registry::BlockStateId,
    };
    let material = |state: &horizon_world::chunk::BlockState| {
        ids.id(&state.name, |k| state.properties.get(k).map(String::as_str))
            .unwrap_or(-1)
    };
    (0..registry.len())
        .flat_map(|i| {
            let Some(state) = registry.state(BlockStateId(i as u32)) else {
                return [-1, -1];
            };
            let fluid = FluidState::from_block(state).unwrap_or(FluidState::EMPTY);
            let name = match fluid.kind() {
                FluidKind::Water | FluidKind::FlowingWater => "minecraft:water",
                FluidKind::Lava | FluidKind::FlowingLava => "minecraft:lava",
                FluidKind::Empty => return [material(state), -1],
                FluidKind::Mod(_) | FluidKind::ModFlowing(_) => {
                    return [material(state), material(state)];
                }
            };
            let fluid = if state.name == name {
                Some(state)
            } else {
                registry
                    .default_state(name)
                    .and_then(|id| registry.state(id))
            };
            [material(state), fluid.map_or(-1, material)]
        })
        .collect()
}

#[cfg(test)]
mod terrain_source_tests {
    use super::*;

    #[test]
    fn terrain_source_cpu_state_filters_fluid_owners_and_pack_reload() {
        let rows = serde_json::json!({"defaults":{"minecraft:air":0,"fixture:signal":1,"minecraft:water":3,"minecraft:lava":5},"states":[
            ["",true,false,"","minecraft:air"],
            ["",false,false,"",{"id":"fixture:signal","properties":{"lit":"false","waterlogged":"false"}}],
            ["",false,true,"",{"id":"fixture:signal","properties":{"lit":"true","waterlogged":"true"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"0"}}],
            ["",false,true,"",{"id":"minecraft:water","properties":{"level":"1"}}],
            ["",false,true,"",{"id":"minecraft:lava","properties":{"level":"0"}}]
        ]});
        let registry =
            horizon_world::registry::BlockRegistry::from_facts_json(rows.to_string().as_bytes())
                .unwrap();
        let parse = |text| BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let ids = parse(
            "block.31=fixture:signal:lit=false\nblock.47=fixture:signal:lit=true\nblock.59=minecraft:water:level=0\nblock.61=minecraft:water:level=1\nblock.73=minecraft:lava\n",
        );
        assert_eq!(
            terrain_state_ids(&registry, &ids),
            [-1, -1, 31, -1, 47, 59, 59, 59, 61, 61, 73, 73]
        );
        let changed = parse("block.211=fixture:signal:lit=true\nblock.223=minecraft:water\n");
        assert_eq!(
            terrain_state_ids(&registry, &changed),
            [-1, -1, -1, -1, 211, 223, 223, 223, 223, 223, -1, -1]
        );
        assert_eq!(terrain_state_ids(&registry, &BlockIds::default()), [-1; 12]);
    }
}

/// Votes from distinct blocks, so adding states cannot change a shared sprite's material.
#[derive(Default)]
struct BlockIdVotes<'a>(BTreeMap<(u32, i32), (bool, std::collections::BTreeSet<&'a str>)>);

impl<'a> BlockIdVotes<'a> {
    fn add(&mut self, layer: u32, id: i32, name: &'a str, sprite: Option<&str>) {
        // Containers share the still sprite. Its fluid owns it just as a block
        // named exactly like its sprite does; a lower container id must not win.
        let owner = sprite
            .and_then(|s| s.strip_prefix("block/"))
            .map(|s| match s {
                "lava_still" | "lava_flow" => "lava",
                "water_still" | "water_flow" => "water",
                _ => s,
            });
        let named = owner == Some(name.strip_prefix("minecraft:").unwrap_or(name));
        let v = self.0.entry((layer, id)).or_default();
        v.0 |= named;
        v.1.insert(name);
    }

    fn write(self, table: &mut [i32]) {
        let mut best = BTreeMap::new();
        for ((layer, id), (named, blocks)) in self.0 {
            let candidate = (named, blocks.len(), -id);
            let b = best.entry(layer).or_insert(candidate);
            if candidate > *b {
                *b = candidate;
            }
        }
        for (layer, (_, _, id)) in best {
            if let Some(slot) = table.get_mut(layer as usize * 2) {
                *slot = -id;
            }
        }
    }
}

#[cfg(test)]
mod block_id_tests {
    use super::BlockIdVotes;

    #[test]
    fn fluid_sprites_keep_their_fluid_material_when_a_container_uses_them() {
        let mut votes = BlockIdVotes::default();
        // Our runtime probe: cauldron 17, lava 29. The lower container id used to
        // win the still sprite's tie, while the flow sprite already read 29.
        for (layer, sprite) in [(0, "lava_still"), (1, "lava_flow")] {
            votes.add(
                layer,
                29,
                "minecraft:lava",
                Some(&format!("block/{sprite}")),
            );
        }
        votes.add(0, 17, "minecraft:lava_cauldron", Some("block/lava_still"));
        // Water's still sprite is also shared by containers and waterlogged models.
        votes.add(2, 31, "minecraft:water", Some("block/water_still"));
        votes.add(2, 17, "minecraft:water_cauldron", Some("block/water_still"));
        votes.add(
            2,
            17,
            "minecraft:other_container",
            Some("block/water_still"),
        );
        let mut table = [-1; 8];
        votes.write(&mut table);
        assert_eq!(table, [29, -1, 29, -1, 31, -1, -1, -1]);
    }

    #[test]
    fn shared_model_sprites_still_use_distinct_blocks_and_the_named_owner() {
        let mut votes = BlockIdVotes::default();
        for _ in 0..20 {
            votes.add(0, 3, "minecraft:many_states", Some("block/shared"));
        }
        votes.add(0, 9, "minecraft:one", Some("block/shared"));
        votes.add(0, 9, "minecraft:two", Some("block/shared"));
        votes.add(1, 20, "minecraft:oak_leaves", Some("block/oak_leaves"));
        votes.add(1, 3, "minecraft:one", Some("block/oak_leaves"));
        votes.add(1, 3, "minecraft:two", Some("block/oak_leaves"));
        let mut table = [-1; 4];
        votes.write(&mut table);
        assert_eq!(table, [9, -1, 20, -1]);
    }
}

/// `mc_Entity` (x, y) of each block sprite layer, as (id, fluid) pairs: terrain vertices carry
/// their sprite layer, not their block. A layer takes the id of the block named like its sprite
/// (`block/oak_leaves` for `oak_leaves`, and still/flow sprites for their fluid), else the id
/// most blocks drawing it share (ties: the lowest); -1 when no block drawing it has one.
/// y is 1 for the fluid sprites, 0 otherwise (`fluid_pairs`).
pub(super) fn block_id_table(pack: &super::PackResources, ids: &BlockIds) -> Vec<i32> {
    let layers = pack
        .render
        .sprite_layers
        .values()
        .max()
        .map_or(0, |l| l + 1) as usize;
    let sprite = |name: &str| pack.render.sprite_layers.get(name).copied();
    let fluids = ["water_still", "water_flow", "lava_still", "lava_flow"]
        .map(|f| sprite(&format!("minecraft:block/{f}")));
    let mut table = fluid_pairs(layers, fluids.into_iter().flatten());
    if ids.is_empty() {
        return table;
    }
    let registry = pack.assets.registry;
    let catalog = &pack.render.model_catalog;
    // (layer, id) -> (a block named like the sprite, the blocks).
    let mut votes = BlockIdVotes::default();
    for i in 0..registry.len() {
        let state_id = horizon_world::registry::BlockStateId(i as u32);
        let Some(state) = registry.state(state_id) else {
            continue;
        };
        let Some(id) = ids.id(&state.name, |k| state.properties.get(k).map(String::as_str)) else {
            continue;
        };
        let block = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
        let mut vote = |layer: u32| {
            votes.add(
                layer,
                id,
                &state.name,
                pack.layer_names.get(&layer).map(String::as_str),
            );
        };
        // Fluids draw from their own sprites.
        let fluid = match block {
            "water" => ["water_still", "water_flow"],
            "lava" => ["lava_still", "lava_flow"],
            _ => ["", ""],
        };
        for f in fluid.iter().filter(|f| !f.is_empty()) {
            if let Some(l) = sprite(&format!("minecraft:block/{f}")) {
                vote(l);
            }
        }
        let origin = horizon_core::position::BlockPos::new(0, 0, 0);
        for (_, model) in catalog.collect(state_id, origin).unwrap_or_default() {
            for face in &model.faces {
                vote(face.layer);
            }
        }
    }
    votes.write(&mut table);
    table
}

/// `layers` (id, fluid) pairs before the ids are known: id -1, and `mc_Entity.y` 1 for the
/// `fluids` layers, 0 for the others. Iris 1.11.7, with probe packs of ours (#10452), gave
/// terrain y 1 on water and lava, and 0 on every other block, whether `block.properties`
/// maps it or not.
fn fluid_pairs(layers: usize, fluids: impl IntoIterator<Item = u32>) -> Vec<i32> {
    let mut table: Vec<i32> = std::iter::repeat_n([-1, 0], layers).flatten().collect();
    for l in fluids {
        if let Some(slot) = table.get_mut(l as usize * 2 + 1) {
            *slot = 1;
        }
    }
    table
}

#[cfg(test)]
mod fluid_pair_tests {
    use super::fluid_pairs;

    /// Iris 1.11.7's `mc_Entity.y` in terrain (probe packs of ours, #10452): 1 on water and
    /// lava, 0 on every other block; the ids start at -1 (no id) until `block.properties` gives
    /// them.
    #[test]
    fn terrain_reads_y_1_on_fluids_and_0_elsewhere() {
        assert_eq!(fluid_pairs(4, [1, 3]), [-1, 0, -1, 1, -1, 0, -1, 1]);
        assert_eq!(fluid_pairs(2, []), [-1, 0, -1, 0]);
    }
}

#[cfg(test)]
mod rendered_item_tests {
    use super::rendered_item_id;

    /// Iris 1.11.7's values (probe packs of ours): a block item reads its block's id, and 0 when
    /// `block.properties` lacks it even if `item.properties` names it (oak planks as item 15);
    /// other items read their `item.properties` id, or 65535 when the file lacks them, and 0
    /// when the pack has no such file.
    #[test]
    fn a_held_item_reads_its_block_id_or_item_id() {
        // Stone: block 21 and item 12.
        assert_eq!(rendered_item_id(Some(21), Some(12), true), 21);
        assert_eq!(rendered_item_id(Some(0), Some(15), true), 0);
        assert_eq!(rendered_item_id(Some(0), None, true), 0);
        assert_eq!(rendered_item_id(Some(0), None, false), 0);
        assert_eq!(rendered_item_id(None, Some(11), true), 11);
        assert_eq!(rendered_item_id(None, None, true), 65535);
        assert_eq!(rendered_item_id(None, None, false), 0);
    }
}

#[cfg(test)]
mod native_pack_tests {
    use super::{active_pack, pack_draws_flat, picked_pack_shape};
    use horizon_client::mod_list::{Installed, ModList, NativePage};

    #[test]
    fn unknown_pack_is_optimistic_but_refusal_owner_and_presence_override_it() {
        use crate::game::native_shaders::{fingerprint, remember};
        use horizon_client::settings::Settings;
        use horizon_shader_api::Api;
        let mut settings = Settings::default();
        settings.horizon.hdr = horizon_client_render::graphics::HdrMode::On;
        settings.mods.installed.push(Installed {
            id: "frame_owner".into(),
            title: "Frame Owner".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let root =
            std::env::temp_dir().join(format!("horizon-frame-policy-{}", std::process::id()));
        let path = root.join("native-frame_owner");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(horizon_shader_api::MANIFEST),
            "schema = 1\nid = \"fixture\"\nname = \"Fixture\"\nversion = \"1\"\napi = \"0.2\"\n[[post]]\nfile = \"p.wgsl\"\n").unwrap();
        settings.horizon.shader_pack = path.to_string_lossy().into_owned();
        let switch = crate::game::native_shaders::linear_frame_switch();
        // A 0.2 pack with HDR on asks for the layered frame, as no pack does.
        let shape = picked_pack_shape(&settings);
        assert_eq!(shape.linear, switch);
        assert_eq!(shape.hdr_float, switch);
        for api in [Api::V0_1, Api::V0_2] {
            let manifest = path.join(horizon_shader_api::MANIFEST);
            let text = std::fs::read_to_string(&manifest).unwrap();
            std::fs::write(
                &manifest,
                text.replace("api = \"0.2\"", &format!("api = \"{api}\""))
                    .replace("api = \"0.1\"", &format!("api = \"{api}\"")),
            )
            .unwrap();
            remember(&path, fingerprint(&path), Some(api));
            let shape = picked_pack_shape(&settings);
            assert_eq!(shape.linear, switch && api == Api::V0_2);
            assert_eq!(shape.hdr_float, switch && api == Api::V0_2);
        }
        settings.mods.disabled.insert("frame_owner".into());
        let shape = picked_pack_shape(&settings);
        assert!(!shape.linear);
        assert_eq!(shape.hdr_float, switch);
        settings.mods.disabled.clear();
        assert_eq!(picked_pack_shape(&settings).linear, switch);
        remember(&path, fingerprint(&path), None);
        assert_eq!(picked_pack_shape(&settings).hdr_float, switch);
        remember(&path, fingerprint(&path), Some(Api::V0_2));
        std::fs::remove_dir_all(root).unwrap();
        let missing = picked_pack_shape(&settings);
        assert!(!missing.linear);
        assert_eq!(missing.hdr_float, switch);
    }

    #[test]
    fn shaders_packs_and_packs_with_a_shadow_pass_draw_flat() {
        let root = std::env::temp_dir().join(format!("horizon-flat-packs-{}", std::process::id()));
        // A folder without a manifest is a `shaders/` pack; with one, a pack of Horizon's own.
        let pack = |name: &str, manifest: Option<&str>| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(extra) = manifest {
                std::fs::write(dir.join("material.wgsl"), "").unwrap();
                let text = format!(
                    "schema = 1\nid = \"{name}\"\nname = \"{name}\"\nversion = \"1\"\napi = \"{}\"\n\
                     [material]\nfile = \"material.wgsl\"\n{extra}",
                    horizon_shader_api::API
                );
                std::fs::write(dir.join(horizon_shader_api::MANIFEST), text).unwrap();
            }
            dir
        };
        assert!(pack_draws_flat(&pack("shaders_pack", None)));
        assert!(!pack_draws_flat(&pack("unshadowed", Some(""))));
        assert!(pack_draws_flat(&pack("shadowed", Some("[shadow]\n"))));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn native_shader_follows_its_mod_without_affecting_other_packs() {
        let mut mods = ModList::default();
        mods.installed.push(Installed {
            id: "horizon_better_visuals".into(),
            title: "Better Visuals".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let owned = "native-horizon_better_visuals";
        assert_eq!(active_pack(owned, &mods), owned);
        mods.disabled.insert("horizon_better_visuals".into());
        assert_eq!(active_pack(owned, &mods), "");
        let root =
            std::env::temp_dir().join(format!("horizon-bundled-shader-{}", std::process::id()));
        let pack = root.join(owned);
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join(horizon_shader_api::MANIFEST),
            "schema = 1\nid = \"bundled\"\nname = \"Bundled\"\nversion = \"1\"\napi = \"0.1\"\n[[post]]\nfile = \"p.wgsl\"\n").unwrap();
        let (key, path) = horizon_client::shader_pack_list::resolve(&root, owned);
        assert_eq!(key, "horizon:bundled");
        assert_eq!(path, pack);
        assert_eq!(
            active_pack(&path.file_name().unwrap().to_string_lossy(), &mods),
            ""
        );
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(active_pack("golden_hour", &mods), "golden_hour");
        mods.installed[0].native = None;
        assert_eq!(active_pack(owned, &mods), owned);
    }
}
