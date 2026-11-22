//! The game's shared read-only tables: `Resources` (blocks, items, recipes, mob types, biomes),
//! `UiResources` and `PackResources`.
use super::{api_values::TryIntoApi, *};
use std::path::Path;

pub(super) fn load_animate_tables(
    root: &Path,
    registry: &sunrise_world::registry::BlockRegistry,
    draws: bool,
    biomes: &Biomes,
) -> Result<(
    sunrise_client::animate_tick::AnimateTables,
    sunrise_client::fog::FogTables,
)> {
    let _step = startup::step("rest.animate_fog");
    let animate = if draws {
        sunrise_client::animate_tick::AnimateTables::load(root, registry)?
    } else {
        Default::default()
    };
    Ok((animate, sunrise_client::fog::FogTables::load(root, biomes)))
}

impl Resources {
    pub(super) fn rest(&self) -> &WorldRest {
        self.rest
            .get()
            .expect("world resources joined before entry")
    }
    /// Whether the world's resources (items, mobs...) have loaded: menus show before they do.
    pub(super) fn rest_ready(&self) -> bool {
        self.rest.get().is_some()
    }
    pub(super) fn facts(&self) -> &BlockFacts {
        &self.core.facts
    }
    pub(super) fn item_defaults(&self) -> &sunrise_world::item_defaults::ItemDefaults {
        self.core.item_data.defaults()
    }
    pub(super) fn items(&self) -> &Items {
        &self.rest().items
    }
    pub(super) fn shared_items(&self) -> Arc<Items> {
        self.rest().items.clone()
    }
    pub(super) fn recipes(&self) -> &Recipes {
        &self.rest().recipes
    }
    pub(super) fn transformers(&self) -> &sunrise_client::block_transform::Transformers {
        &self.rest().transformers
    }
    pub(super) fn biomes(&self) -> &Arc<Biomes> {
        &self.core.biomes
    }
    pub(super) fn environment(&self) -> &Environment {
        &self.core.environment
    }
    /// The environment of a dimension type id; a type with no table of its own gets the overworld's.
    pub(super) fn environment_of(&self, dimension_type: &str) -> &Environment {
        match dimension_type {
            "minecraft:the_nether" => self.nether_environment(),
            "minecraft:the_end" => self.end_environment(),
            _ => self.environment(),
        }
    }
    pub(super) fn mob_types(&self) -> &HashMap<String, Arc<MobType>> {
        &self.rest().mob_types
    }
    pub(super) fn mob_data(&self) -> &sunrise_client::mobs::MobData {
        &self.rest().mob_data
    }
    pub(super) fn providers(&self) -> &Providers {
        &self.rest().providers
    }
    pub(super) fn block_entity_render(&self) -> &BlockEntityRenderer {
        &self.rest().block_entity_render
    }
    pub(super) fn animate(&self) -> &sunrise_client::animate_tick::AnimateTables {
        &self.rest().animate
    }
    pub(super) fn fog(&self) -> &sunrise_client::fog::FogTables {
        &self.rest().fog
    }
    pub(super) fn music(&self) -> &sunrise_client::music::MusicTables {
        &self.rest().music
    }
    pub(super) fn ambient(&self) -> &sunrise_client::ambience::AmbientTables {
        &self.rest().ambient
    }
    pub(super) fn nether_environment(&self) -> &Environment {
        &self.rest().nether_environment
    }
    pub(super) fn end_environment(&self) -> &Environment {
        &self.rest().end_environment
    }
    pub(super) fn behaviours(&self) -> &Behaviours {
        &self.rest().behaviours
    }
    pub(super) fn collision(&self) -> &BlockCollisionRegistry<'static> {
        &self.core.collision
    }
    pub(super) fn suffocation(&self) -> &sunrise_world::suffocation::SuffocationRegistry<'static> {
        &self.core.suffocation
    }
    pub(super) fn motion(&self) -> &MotionResources {
        &self.core.motion
    }
    /// The variant registries finalizeSpawn selects from
    /// (`data/minecraft/<registry>`), shared with world generation.
    pub fn spawn_registries(&self) -> &sunrise_client::mobs::finalize::SpawnRegistries {
        &self.core.spawn_registries
    }
    pub(super) fn threads(&self) -> usize {
        self.core.threads
    }
    pub(super) fn player_arm(&self, slim: bool, left: bool) -> &Option<sunrise_client::mobs::Part> {
        let arms = if slim {
            &self.rest().player_arm_slim
        } else {
            &self.rest().player_arm
        };
        &arms[usize::from(left)]
    }
    pub(super) fn player_type(&self) -> &Option<Arc<sunrise_client::mobs::MobType>> {
        &self.rest().player_type
    }
    pub(super) fn spin_attack(&self) -> &Option<sunrise_client::mobs::Part> {
        &self.rest().spin_attack
    }
    pub(super) fn stations(&self) -> &sunrise_client::stations::StationData {
        &self.rest().stations
    }
}

/// Everything the title and menus draw: the GUI atlas with its font, the
/// title panorama and the splash pool. Kept apart from the world bundle so the
/// title can appear before the block sprites, models and gameplay tables are ready.
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<sunrise_client::gfx::panorama::Faces>,
    /// The title screen's splashes (`sunrise_client::splashes::pool`).
    pub splashes: Arc<str>,
    /// Starling: what it was read from, the packs over the game's own, whose sprites' nine-slice
    /// metadata and translations the title takes with it (`StartupMenu::set_root`). The loader
    /// names only the game's own folder, and a 20 x 20 pack button nine-sliced as vanilla's
    /// 200 x 20 one stretched its ends tenfold. None where it was read from no source.
    pub source: Option<sunrise_assets::resource_source::ResourceSource>,
    /// The `Text` of each plain string mods draw (`DrawContext::text`).
    pub plain_texts: PlainTexts,
}

/// The `Text` of each plain string a mod draws by `&str` (`DrawContext::text`), kept so drawing
/// the same string again builds no new `Text`. It holds at most [`PlainTexts::MAX`] strings and
/// starts over past that, so text that changes every frame can't grow it.
#[derive(Default)]
pub struct PlainTexts(
    std::cell::RefCell<std::collections::HashMap<Box<str>, sunrise_client::text::Text>>,
);

impl PlainTexts {
    const MAX: usize = 256;
    /// `draw` with the `Text` of `text`, made the first time it is asked for.
    pub(in crate::game) fn with<R>(
        &self,
        text: &str,
        draw: impl FnOnce(&sunrise_client::text::Text) -> R,
    ) -> R {
        let mut texts = self.0.borrow_mut();
        if !texts.contains_key(text) {
            if texts.len() >= Self::MAX {
                texts.clear();
            }
            texts.insert(text.into(), sunrise_client::text::Text::literal(text));
        }
        draw(&texts[text])
    }
}

impl Game<'_> {
    /// A title screen over the current resources and their splash pool.
    pub(crate) fn new_title(&self, saves: PathBuf) -> Title {
        Title::new(saves, &self.options.resources, &self.gui.splashes)
    }
}

/// Whether the HUD atlas packs the pool of dynamic texture slots: whenever a client mod is
/// loaded, since any of them can call `ClientHost::create_dynamic_texture`, with
/// no mixin and no opt-in (#4437). A game with no mod packs no slot and its atlas does not grow.
/// Read where the atlas is built: at the first load and on a resource reload.
pub(in crate::game) fn dynamic_texture_pool() -> bool {
    !mod_api::__private::mods().is_empty()
}

impl UiResources {
    /// The resolved resource generation, including the base layer and pack filters, over the
    /// game's own resources at `vanilla` (which tell the packs' core shaders from the game's).
    /// Its splash pool is empty until `with_splashes`.
    pub fn load(
        root: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &Path,
    ) -> Result<Self> {
        // The title screen waits for both: the panorama's six faces decode
        // beside the atlas.
        std::thread::scope(|scope| {
            let panorama = named_scope(scope, "panorama", || {
                startup::time("ui.panorama", || {
                    sunrise_client::gfx::panorama::Faces::load(root, root)
                })
            });
            let pool = dynamic_texture_pool();
            let game = sunrise_assets::resource_source::ResourceSource::directory(vanilla.into());
            let atlas = startup::time("ui.atlas", || {
                HudAtlas::build_over_with_dynamic_textures(root, root, &game, pool)
            });
            let panorama = panorama
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            Ok(Self {
                atlas: atlas?,
                panorama: Arc::new(panorama),
                source: Some(root.clone()),
                splashes: Arc::from(""),
                plain_texts: PlainTexts::default(),
            })
        })
    }
    /// The title's splash pool from `stack` (`sunrise_client::splashes::pool`).
    pub fn with_splashes(
        mut self,
        stack: &sunrise_assets::resource_source::ResourceSource,
        vanilla: &std::path::Path,
        skip_if_modified: bool,
    ) -> Self {
        self.splashes = sunrise_client::splashes::pool(stack, vanilla, skip_if_modified).into();
        self
    }
    /// What a server's game holds, since it draws nothing: the font, which books
    /// and chat measure with, and a one-pixel panorama.
    pub fn server(root: &sunrise_assets::resource_source::ResourceSource) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::fonts(root)?,
            panorama: Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])),
            source: Some(root.clone()),
            splashes: Arc::from(""),
            plain_texts: PlainTexts::default(),
        })
    }
}

impl PackResources {
    /// Everything a pack overlay at `root` provides, over `assets` and `render`
    /// loaded from it.
    pub(super) fn load(
        root: &sunrise_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
        item_data: &sunrise_world::item_defaults::ItemData,
    ) -> Result<Self> {
        let visibility = models::load_visibility(root.prepared(), assets.registry)?;
        api_values::resolve_block_definitions(assets.registry)?;
        mods::extend_poi_types(assets.registry);
        mods::register_cauldron_fluid_contents(assets.registry);
        let tables = Arc::new(
            MeshTables::new(
                &assets,
                &render,
                facts,
                visibility,
                api_values::mod_tint_values(assets.registry)?,
            )?
            .with_polytone_colormaps(root.source(), assets.registry),
        );
        let item_renderer = sunrise_client::item_render::ItemRenderer::load_with_library(
            root,
            render.model_library.clone(),
            item_data,
        )?;
        Self::assemble(root, assets, render, tables, item_renderer, biomes, light)
    }
    /// The workers' context over `biomes`: this pack's own when `biomes` is
    /// the table it was built with, else the same parts with that table.
    pub(super) fn context_with(&self, biomes: Arc<Biomes>) -> Arc<WorkerContext> {
        let old = &self.context;
        if Arc::ptr_eq(&old.biomes, &biomes) {
            return old.clone();
        }
        Arc::new(WorkerContext {
            assets: old.assets.clone(),
            render: old.render.clone(),
            tables: old.tables.clone(),
            biomes,
            light: old.light.clone(),
            hidden_states: old.hidden_states.clone(),
            part_tables: old.part_tables.clone(),
        })
    }
    /// The pack tables over their parts, which `load_resources` builds on
    /// separate workers. A cloud texture vanilla refuses fails it, as it fails vanilla's reload
    /// (`sunrise_client::clouds::load`).
    pub(super) fn assemble(
        root: &sunrise_assets::resource_source::ResourceRoot,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        tables: Arc<MeshTables>,
        item_renderer: sunrise_client::item_render::ItemRenderer,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
    ) -> Result<Self> {
        let water_overlay_state = assets
            .registry
            .default_state("minecraft:water")
            .ok_or("block registry is missing the default minecraft:water state")?
            .try_into_api()
            .map_err(|e| format!("cannot bind the default minecraft:water state: {e:?}"))?;
        // These pack-local resources do not depend on one another. Decode the
        // images and language map together so a custom-sky pack cannot put all
        // of the pack assembly on the critical path behind its own files.
        let (names, clouds, forcefield, custom_sky, environments, polytone_particles) =
            std::thread::scope(|scope| {
                let names = named_scope(scope, "pack.names", || Names::load(root.source()));
                let clouds = named_scope(scope, "pack.clouds", || {
                    sunrise_client::clouds::load(root.source())
                });
                let forcefield =
                    named_scope(scope, "pack.forcefield", || load_forcefield(root.source()));
                let custom_sky = named_scope(scope, "pack.custom_sky", || {
                    sunrise_client::custom_sky::load(root.source())
                });
                let environments =
                    named_scope(scope, "pack.environments", || pack_environments(root));
                let particles = named_scope(scope, "pack.particles", || {
                    pack_particles(root, assets.registry)
                });
                (
                    names.join().unwrap(),
                    clouds.join().unwrap(),
                    forcefield.join().unwrap(),
                    custom_sky.join().unwrap(),
                    environments.join().unwrap(),
                    particles.join().unwrap(),
                )
            });
        let clouds = clouds.map_err(|e| e.to_string())?;
        let polytone_post = sunrise_client::polytone_post::PolytonePost::read(
            root.source(),
            sunrise_client::packs::polytone_configs(&sunrise_client::packs::Source::Opened(
                root.source().clone(),
            )),
        )
        .map(Arc::new);
        // Each item's own default name, which the hover name reads as `getDescriptionId` does.
        let names = names.with_item_names(&item_renderer.models.prototypes, assets.registry);
        Ok(Self {
            layer_names: Arc::new(sunrise_client::block_sprites::texture_names(
                &render.sprite_layers,
            )),
            context: Arc::new(WorkerContext {
                assets: assets.clone(),
                render: render.clone(),
                tables,
                biomes,
                light,
                hidden_states: Arc::from([]),
                part_tables: Default::default(),
            }),
            block_registries: native_block_states::block_registries(assets.registry),
            assets,
            water_overlay_state,
            render,
            item_renderer: Arc::new(item_renderer),
            source: root.source().clone(),
            data_source: root.data().clone(),
            data_changes: root.data_changes().clone(),
            additional_data: Arc::from([]),
            mod_layers: root.mod_layers().clone(),
            names: Arc::new(names),
            clouds: Arc::new(clouds),
            forcefield: Arc::new(forcefield),
            custom_sky,
            environments,
            polytone_particles,
            polytone_post,
            polytone_colors: Arc::new(read_colors(root)),
        })
    }
}

/// The overworld's, Nether's and End's attribute stacks with the Polytone biome and dimension
/// modifiers of the packs merged at `root` (#11486); `None` when no pack has any. The overlay
/// root links the vanilla data the stacks start from.
fn pack_environments(
    root: &sunrise_assets::resource_source::ResourceRoot,
) -> Option<Arc<[Environment; 3]>> {
    let modifiers = sunrise_client::environment::PackModifiers::read(root.source());
    if modifiers.is_empty() {
        return None;
    }
    let modifiers = modifiers.with_configs(sunrise_client::packs::polytone_configs(
        &sunrise_client::packs::Source::Opened(root.source().clone()),
    ));
    let load = |d: &str| Environment::load_dimension_with(root.data(), d, &modifiers);
    match (load("overworld"), load("the_nether"), load("the_end")) {
        (Ok(o), Ok(n), Ok(e)) => Some(Arc::new([o, n, e])),
        (o, n, e) => {
            for error in [o.err(), n.err(), e.err()].into_iter().flatten() {
                log::warn!("pack environment: {error}");
            }
            None
        }
    }
}

/// The custom particles and block emitters of the packs merged at `root` (#11486); `None` when
/// no pack has any.
fn pack_particles(
    root: &sunrise_assets::resource_source::ResourceRoot,
    blocks: &'static sunrise_world::registry::BlockRegistry,
) -> Option<Arc<sunrise_client::particles::polytone::Registry>> {
    let configs = sunrise_client::packs::polytone_configs(&sunrise_client::packs::Source::Opened(
        root.source().clone(),
    ));
    let registry =
        sunrise_client::particles::polytone::Registry::read(root.source(), configs, blocks);
    (!registry.is_empty()).then(|| Arc::new(registry))
}

/// The packs' custom colours; their dye and sheep colours go to the draw paths' table.
fn read_colors(
    root: &sunrise_assets::resource_source::ResourceRoot,
) -> sunrise_client::pack_palette::PackColors {
    let colors = sunrise_client::pack_palette::read(root.source());
    sunrise_client::pack_palette::set(&colors);
    sunrise_client::brewing::set_pack_colors(colors.effects.clone());
    colors
}

#[cfg(test)]
mod plain_text_tests {
    use super::PlainTexts;

    #[test]
    fn a_string_drawn_again_reuses_its_text_and_the_store_stays_bounded() {
        let texts = PlainTexts::default();
        let first = texts.with("Jump", |t| t as *const _);
        assert_eq!(texts.with("Jump", |t| t as *const _), first);
        assert_eq!(texts.with("Jump", |t| t.plain()), "Jump");
        for i in 0..PlainTexts::MAX * 2 {
            texts.with(&i.to_string(), |_| ());
        }
        assert!(texts.0.borrow().len() <= PlainTexts::MAX);
    }
}
