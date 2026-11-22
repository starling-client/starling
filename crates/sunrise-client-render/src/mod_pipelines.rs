//! Render pipelines mods register (`Hooks::register_render_pipelines`, NeoForge's
//! `RegisterRenderPipelinesEvent`): the registrations, kept once at start, and what each
//! resource load makes of them. A load reads every registered pipeline's fragment file
//! (`assets/<namespace>/shaders/pipeline/<path>.wgsl`), puts Sunrise's prelude before it and its
//! entry points after it, and checks the whole shader, so the renderer (`gfx::mod_pipelines`) is
//! only ever handed WGSL that compiles. A required pipeline the load cannot build fails the load;
//! an optional one is logged and left out. A frame reads none of this by name.
//! The shader's composition and checks live in `sunrise_shader_api::mod_pipeline`, shared with
//! `cargo sunrise-mod check`.
use mod_api::{
    Identifier, PipelineBlend, PipelineTexture, PipelineTextureError, RenderPipeline,
    RenderPipelineError, RenderPipelineRef, WorldDepth,
};
use std::{collections::BTreeMap, sync::OnceLock};
use sunrise_shader_api::mod_pipeline::{FOLDER, compose_with};

static REGISTERED: OnceLock<Vec<RenderPipeline>> = OnceLock::new();

/// Canonical vanilla 26.3 required render pipeline paths under the `minecraft` namespace
/// (`RenderPipelines.PIPELINES_BY_LOCATION`, sorted for binary search).
const BUILTIN_REQUIRED_PIPELINES: &[&str] = &[
    "pipeline/animate_sprite_blit",
    "pipeline/animate_sprite_interpolate",
    "pipeline/armor_cutout_no_cull",
    "pipeline/armor_cutout_no_cull_glint",
    "pipeline/armor_decal_cutout_no_cull",
    "pipeline/armor_translucent",
    "pipeline/banner_pattern",
    "pipeline/beacon_beam_opaque",
    "pipeline/beacon_beam_translucent",
    "pipeline/blit_depth",
    "pipeline/blit_depth_bounds",
    "pipeline/blit_depth_during_depth_bounds",
    "pipeline/block_screen_effect",
    "pipeline/breeze_wind",
    "pipeline/celestial",
    "pipeline/clouds",
    "pipeline/crosshair",
    "pipeline/crumbling",
    "pipeline/cutout_block",
    "pipeline/cutout_terrain",
    "pipeline/cutout_terrain_multidraw",
    "pipeline/debug_filled_box",
    "pipeline/debug_points",
    "pipeline/debug_quads",
    "pipeline/debug_triangle_fan",
    "pipeline/dragon_rays",
    "pipeline/end_crystal_beam",
    "pipeline/end_gateway",
    "pipeline/end_portal",
    "pipeline/end_sky",
    "pipeline/energy_swirl",
    "pipeline/entity_cutout",
    "pipeline/entity_cutout_cull",
    "pipeline/entity_cutout_dissolve",
    "pipeline/entity_cutout_z_offset",
    "pipeline/entity_outline_blit",
    "pipeline/entity_shadow",
    "pipeline/entity_solid",
    "pipeline/entity_solid_glint",
    "pipeline/entity_solid_offset_forward",
    "pipeline/entity_translucent",
    "pipeline/entity_translucent_cull",
    "pipeline/entity_translucent_emissive",
    "pipeline/eyes",
    "pipeline/fire_screen_effect",
    "pipeline/flat_clouds",
    "pipeline/glint",
    "pipeline/gui",
    "pipeline/gui_invert",
    "pipeline/gui_nausea_overlay",
    "pipeline/gui_opaque_textured_background",
    "pipeline/gui_text",
    "pipeline/gui_text_grayscale",
    "pipeline/gui_text_highlight",
    "pipeline/gui_textured",
    "pipeline/gui_textured_premultiplied_alpha",
    "pipeline/integrate_depth",
    "pipeline/item_cutout",
    "pipeline/item_translucent",
    "pipeline/item_translucent_glint",
    "pipeline/leash",
    "pipeline/lightmap",
    "pipeline/lightning",
    "pipeline/lines",
    "pipeline/lines_depth_bias",
    "pipeline/lines_translucent",
    "pipeline/lines_translucent_no_depth_write",
    "pipeline/mojang_logo",
    "pipeline/oit_accumulate_beacon_beam",
    "pipeline/oit_accumulate_breeze_wind",
    "pipeline/oit_accumulate_clouds",
    "pipeline/oit_accumulate_crumbling",
    "pipeline/oit_accumulate_debug_filled_box",
    "pipeline/oit_accumulate_debug_points",
    "pipeline/oit_accumulate_debug_quads",
    "pipeline/oit_accumulate_debug_triangle_fan",
    "pipeline/oit_accumulate_dragon_rays",
    "pipeline/oit_accumulate_energy_swirl",
    "pipeline/oit_accumulate_entity",
    "pipeline/oit_accumulate_entity_cull",
    "pipeline/oit_accumulate_entity_emissive",
    "pipeline/oit_accumulate_entity_shadow",
    "pipeline/oit_accumulate_eyes",
    "pipeline/oit_accumulate_flat_clouds",
    "pipeline/oit_accumulate_item",
    "pipeline/oit_accumulate_item_glint",
    "pipeline/oit_accumulate_item_glint_special",
    "pipeline/oit_accumulate_lightning",
    "pipeline/oit_accumulate_lines_translucent",
    "pipeline/oit_accumulate_particle",
    "pipeline/oit_accumulate_terrain",
    "pipeline/oit_accumulate_terrain_multidraw",
    "pipeline/oit_accumulate_text",
    "pipeline/oit_accumulate_text_grayscale",
    "pipeline/oit_accumulate_text_grayscale_polygon_offset",
    "pipeline/oit_accumulate_text_polygon_offset",
    "pipeline/oit_accumulate_translucent_block",
    "pipeline/oit_accumulate_weather",
    "pipeline/oit_accumulate_world_border",
    "pipeline/oit_composite",
    "pipeline/oit_depth_bounds_beacon_beam",
    "pipeline/oit_depth_bounds_breeze_wind",
    "pipeline/oit_depth_bounds_clouds",
    "pipeline/oit_depth_bounds_crumbling",
    "pipeline/oit_depth_bounds_cull",
    "pipeline/oit_depth_bounds_debug_filled_box",
    "pipeline/oit_depth_bounds_debug_points",
    "pipeline/oit_depth_bounds_debug_quads",
    "pipeline/oit_depth_bounds_debug_triangle_fan",
    "pipeline/oit_depth_bounds_dragon_rays",
    "pipeline/oit_depth_bounds_energy_swirl",
    "pipeline/oit_depth_bounds_entity",
    "pipeline/oit_depth_bounds_entity_cull",
    "pipeline/oit_depth_bounds_entity_emissive",
    "pipeline/oit_depth_bounds_entity_shadow",
    "pipeline/oit_depth_bounds_eyes",
    "pipeline/oit_depth_bounds_flat_clouds",
    "pipeline/oit_depth_bounds_item",
    "pipeline/oit_depth_bounds_item_glint",
    "pipeline/oit_depth_bounds_item_glint_special",
    "pipeline/oit_depth_bounds_lightning",
    "pipeline/oit_depth_bounds_lines_translucent",
    "pipeline/oit_depth_bounds_particle",
    "pipeline/oit_depth_bounds_terrain",
    "pipeline/oit_depth_bounds_terrain_multidraw",
    "pipeline/oit_depth_bounds_text",
    "pipeline/oit_depth_bounds_text_grayscale",
    "pipeline/oit_depth_bounds_text_grayscale_polygon_offset",
    "pipeline/oit_depth_bounds_text_polygon_offset",
    "pipeline/oit_depth_bounds_translucent_block",
    "pipeline/oit_depth_bounds_weather",
    "pipeline/oit_depth_bounds_world_border",
    "pipeline/oit_transmittance_beacon_beam",
    "pipeline/oit_transmittance_breeze_wind",
    "pipeline/oit_transmittance_clouds",
    "pipeline/oit_transmittance_crumbling",
    "pipeline/oit_transmittance_debug_filled_box",
    "pipeline/oit_transmittance_debug_points",
    "pipeline/oit_transmittance_debug_quads",
    "pipeline/oit_transmittance_debug_triangle_fan",
    "pipeline/oit_transmittance_dragon_rays",
    "pipeline/oit_transmittance_energy_swirl",
    "pipeline/oit_transmittance_entity",
    "pipeline/oit_transmittance_entity_cull",
    "pipeline/oit_transmittance_entity_emissive",
    "pipeline/oit_transmittance_entity_shadow",
    "pipeline/oit_transmittance_eyes",
    "pipeline/oit_transmittance_flat_clouds",
    "pipeline/oit_transmittance_item",
    "pipeline/oit_transmittance_item_glint",
    "pipeline/oit_transmittance_item_glint_special",
    "pipeline/oit_transmittance_lightning",
    "pipeline/oit_transmittance_lines_translucent",
    "pipeline/oit_transmittance_particle",
    "pipeline/oit_transmittance_terrain",
    "pipeline/oit_transmittance_terrain_multidraw",
    "pipeline/oit_transmittance_text",
    "pipeline/oit_transmittance_text_grayscale",
    "pipeline/oit_transmittance_text_grayscale_polygon_offset",
    "pipeline/oit_transmittance_text_polygon_offset",
    "pipeline/oit_transmittance_translucent_block",
    "pipeline/oit_transmittance_weather",
    "pipeline/oit_transmittance_world_border",
    "pipeline/oit_water_mask",
    "pipeline/opaque_particle",
    "pipeline/outline_cull",
    "pipeline/outline_no_cull",
    "pipeline/panorama",
    "pipeline/secondary_block_outline",
    "pipeline/sky",
    "pipeline/solid_block",
    "pipeline/solid_terrain",
    "pipeline/solid_terrain_multidraw",
    "pipeline/stars",
    "pipeline/sunrise_sunset",
    "pipeline/text",
    "pipeline/text_grayscale",
    "pipeline/text_grayscale_polygon_offset",
    "pipeline/text_grayscale_see_through",
    "pipeline/text_polygon_offset",
    "pipeline/text_see_through",
    "pipeline/tracy_blit",
    "pipeline/translucent_block",
    "pipeline/translucent_particle",
    "pipeline/translucent_terrain",
    "pipeline/translucent_terrain_multidraw",
    "pipeline/vignette",
    "pipeline/water_mask",
    "pipeline/weather",
    "pipeline/world_border",
];

/// Canonical vanilla 26.3 optional render pipeline paths under the `minecraft` namespace
/// (`RenderPipelines.OPTIONAL_PIPELINES_BY_LOCATION`, sorted for binary search).
const BUILTIN_OPTIONAL_PIPELINES: &[&str] = &["pipeline/wireframe", "pipeline/wireframe_multidraw"];

/// Publish the mods' registrations once at startup, as `register_render_pipelines` left them.
/// Mod pipeline IDs are unique across the table because lookup uses the ID alone. Check each
/// mod ID against the corresponding vanilla required or optional registry, as NeoForge's
/// `RenderPipelines.registerCustomPipelines` does. A collision fails startup without committing
/// the table. A pipeline's place in the list is its slot.
pub fn register(pipelines: Vec<RenderPipeline>) -> Result<(), String> {
    register_into(&REGISTERED, pipelines)
}

fn register_into(
    slot: &OnceLock<Vec<RenderPipeline>>,
    pipelines: Vec<RenderPipeline>,
) -> Result<(), String> {
    let mut seen = BTreeMap::new();
    for pipeline in &pipelines {
        let (builtins, kind) = if pipeline.optional {
            (BUILTIN_OPTIONAL_PIPELINES, "optional")
        } else {
            (BUILTIN_REQUIRED_PIPELINES, "required")
        };
        if pipeline.id.namespace() == "minecraft"
            && builtins.binary_search(&pipeline.id.path()).is_ok()
        {
            return Err(format!(
                "Duplicate {kind} render pipeline registration: {} collides with a built-in vanilla pipeline (registered by {})",
                pipeline.id,
                pipeline.owner()
            ));
        }
        if let Some(first_owner) = seen.insert(&pipeline.id, pipeline.owner()) {
            return Err(format!(
                "Duplicate render pipeline registration: {} (registered by {} and {})",
                pipeline.id,
                first_owner,
                pipeline.owner()
            ));
        }
    }
    if slot.set(pipelines).is_err() {
        log::warn!("Render pipelines already registered; keeping the first list");
    }
    Ok(())
}

/// The registered pipelines; none until registration publishes them.
fn registered() -> &'static [RenderPipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

/// A registered pipeline one resource load built: a shader that compiles and its fixed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The owning mod, not necessarily the pipeline namespace.
    pub owner: String,
    /// The pipeline's id, for the log.
    pub id: Identifier,
    /// The whole shader: prelude, the mod's file, entry points.
    pub shader: String,
    pub blend: PipelineBlend,
    pub depth: WorldDepth,
    /// Whether the load may go on without it ([`RenderPipeline::optional()`]).
    pub optional: bool,
    /// What its shader samples, if anything.
    pub texture: Option<BuiltTexture>,
}

/// A textured pipeline's texture as a load found it ([`PipelineTexture`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltTexture {
    /// A sprite of the 2D atlas, by its name there (`block/stone`, `powah:block/energy_charge`),
    /// looked up when the renderer builds the pipeline.
    Sprite(String),
}

/// The registered pipelines as one resource load built them, by slot. An optional pipeline the
/// load refused has no shader here and resolves to [`RenderPipelineError::Invalid`].
#[derive(Clone, Debug, Default)]
pub struct Pipelines {
    ids: Vec<Identifier>,
    built: Vec<Option<Built>>,
    /// The render assets' generation this set was loaded for; 0 for none.
    generation: u64,
    invalid: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}

impl Pipelines {
    /// Build every registered pipeline from `source`, for the load of `generation`. An optional
    /// pipeline whose file is missing or refused is logged with its reason and the others build;
    /// a required one fails the load, naming every required pipeline refused and why.
    pub fn load(
        source: &crate::resource_source::ResourceSource,
        generation: u64,
    ) -> Result<Self, String> {
        Self::build(registered(), generation, |file| {
            source.read_to_string(file).map_err(|e| e.to_string())
        })
    }

    /// Build every registered pipeline, reading fragment files with `read`. A pipeline's
    /// texture is named here; its sprite is looked up when the renderer builds it.
    pub(crate) fn build(
        registered: &[RenderPipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<String, String>,
    ) -> Result<Self, String> {
        let mut required = Vec::new();
        let built = registered
            .iter()
            .map(|pipeline| {
                let file = fragment_file(&pipeline.fragment);
                // A refusal reads `<file>[:<line>:<column>]: <message>`, the shader's own
                // `Problem`s printed as the pack checker prints them. The checks run contained:
                // a panic in naga on a mod's shader refuses the pipeline, with no crash report.
                let loaded = read(&file)
                    .map_err(|cause| format!("{file}: {cause}"))
                    .and_then(|text| {
                        crate::crash::contain(|| {
                            compose_with(&file, &text, pipeline.texture.is_some())
                        })
                        .map_err(|panic| format!("{file}: checking it panicked: {panic}"))?
                        .map_err(|problems| {
                            problems
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                    })
                    .and_then(|shader| {
                        let texture = match &pipeline.texture {
                            None => None,
                            Some(texture) => Some(load_texture(texture).map_err(|cause| {
                                format!("its texture: {}", texture_refusal(cause))
                            })?),
                        };
                        Ok((shader, texture))
                    });
                match loaded {
                    Ok((shader, texture)) => Some(Built {
                        owner: pipeline.owner().to_owned(),
                        id: pipeline.id.clone(),
                        shader,
                        blend: pipeline.blend,
                        depth: pipeline.depth,
                        optional: pipeline.optional,
                        texture,
                    }),
                    Err(reason) => {
                        let refusal = format!(
                            "render pipeline {} from mod {} refused: {reason}",
                            pipeline.id,
                            pipeline.owner()
                        );
                        if pipeline.optional {
                            log::warn!("Sunrise: {refusal}");
                        } else {
                            required.push(format!("required {refusal}"));
                        }
                        None
                    }
                }
            })
            .collect();
        if !required.is_empty() {
            return Err(required.join("; "));
        }
        Ok(Self {
            ids: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            invalid: Default::default(),
        })
    }

    /// Marks `slot` as one the GPU refused; false when it already was, so it is reported once.
    pub(crate) fn invalidate(&self, slot: usize) -> bool {
        let mut invalid = self.invalid.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = !invalid.contains(&slot);
        if fresh {
            invalid.push(slot);
        }
        fresh
    }

    /// Makes `slot` usable again: a renderer that built it took the refusal of another over.
    pub(crate) fn revalidate(&self, slot: usize) {
        self.invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|&invalid| invalid != slot);
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<Built>] {
        &self.built
    }

    /// The handle of the registered pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let slot = self
            .ids
            .iter()
            .position(|registered| registered == id)
            .ok_or(RenderPipelineError::Unknown)?;
        if self.built[slot].is_none()
            || self
                .invalid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&slot)
        {
            return Err(RenderPipelineError::Invalid);
        }
        Ok(mod_api::__private::built::render_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle draws with, checked once when a batch is submitted.
    pub fn slot(&self, pipeline: RenderPipelineRef) -> Result<u32, RenderPipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(RenderPipelineError::Stale);
        }
        if self
            .invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(fields.slot as usize))
        {
            return Err(RenderPipelineError::Invalid);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(RenderPipelineError::Invalid),
            None => Err(RenderPipelineError::Unknown),
        }
    }
}

/// `mymod:beam/glow` is `assets/mymod/shaders/pipeline/beam/glow.wgsl`.
fn fragment_file(fragment: &Identifier) -> String {
    format!(
        "assets/{}/{FOLDER}/{}.wgsl",
        fragment.namespace(),
        fragment.path()
    )
}

/// `texture` as a load finds it, or its named refusal.
fn load_texture(texture: &PipelineTexture) -> Result<BuiltTexture, PipelineTextureError> {
    match texture {
        PipelineTexture::Sprite(id) => {
            // A file's path or name is not a sprite key.
            let path = id.path();
            if path.starts_with("textures/") || path.ends_with(".png") {
                return Err(PipelineTextureError::NotASpriteKey);
            }
            // Named as the 2D atlas names the blocks and items atlases' sprites.
            Ok(BuiltTexture::Sprite(match id.namespace() {
                "minecraft" => path.to_owned(),
                _ => id.to_string(),
            }))
        }
        // A texture kind a later API adds is one this game has no route for yet.
        _ => Err(PipelineTextureError::NoSuchTextureFile),
    }
}

/// The diagnostics line of a refused texture: its named cause, then what it means.
pub fn texture_refusal(cause: PipelineTextureError) -> String {
    format!("{cause:?}: {cause}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOW: &str = "\
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    return vec4<f32>(input.color.rgb * (1.0 + sin(frame.time)), input.color.a * input.uv.y);
}
";

    fn pipeline(id: &str, fragment: &str) -> RenderPipeline {
        RenderPipeline::new(
            Identifier::parse(id).unwrap(),
            Identifier::parse(fragment).unwrap(),
            PipelineBlend::Additive,
            WorldDepth::ThroughWalls,
        )
    }
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn an_optional_pipeline_that_fails_leaves_the_others_drawing_and_refuses_its_own_use() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:broken", "example:broken").optional(),
            pipeline("example:missing", "example:nested/missing").optional(),
            pipeline("example:driver", "example:glow").optional(),
        ];
        let read = |file: &str| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            "assets/example/shaders/pipeline/broken.wgsl" => Ok("fn nothing() {}".to_owned()),
            "assets/example/shaders/pipeline/nested/missing.wgsl" => Err("not found".to_owned()),
            other => panic!("read {other}"),
        };
        let set = Pipelines::build(&registered, 7, read).unwrap();

        let glow = set.resolve(&id("example:glow")).unwrap();
        assert_eq!(set.slot(glow), Ok(0));
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(
            (built.blend, built.depth),
            (PipelineBlend::Additive, WorldDepth::ThroughWalls)
        );
        assert_eq!(
            set.resolve(&id("example:broken")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:missing")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:never")),
            Err(RenderPipelineError::Unknown)
        );

        // The GPU turns down a pipeline the load built: it is reported once, a handle taken
        // before is refused, and the rest still draw.
        let driver = set.resolve(&id("example:driver")).unwrap();
        assert!(set.invalidate(3));
        assert!(!set.invalidate(3));
        assert_eq!(set.slot(driver), Err(RenderPipelineError::Invalid));
        assert_eq!(
            set.resolve(&id("example:driver")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(set.slot(glow), Ok(0));
    }

    #[test]
    fn a_required_pipeline_that_fails_fails_the_load_by_name() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:beam", "example:beam"),
            pipeline("example:extra", "example:extra").optional(),
        ];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:beam")).unwrap();

        // A pack broke the required beam and the optional extra: the load fails naming the
        // required one only, so nothing of it is published and the previous load still draws.
        let error = Pipelines::build(&registered, 8, |file| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            _ => Ok("garbage".to_owned()),
        })
        .unwrap_err();
        assert!(
            error.starts_with(
                "required render pipeline example:beam from mod  refused: assets/example/shaders/pipeline/beam.wgsl:1:1: "
            ),
            "{error}"
        );
        assert!(!error.contains("example:glow"), "{error}");
        assert!(!error.contains("example:extra"), "{error}");
        assert_eq!(before.slot(handle), Ok(1));
    }

    #[test]
    fn initial_required_failure_has_a_literal_message() {
        let registered = [pipeline("example:glow", "example:glow")];
        let error = Pipelines::build(&registered, 7, |_| Err("not found".to_owned())).unwrap_err();
        assert_eq!(
            error,
            "required render pipeline example:glow from mod  refused: assets/example/shaders/pipeline/glow.wgsl: not found"
        );
    }

    #[test]
    fn a_reload_makes_old_handles_stale_and_can_fix_or_break_a_pipeline() {
        let registered = [pipeline("example:glow", "example:glow").optional()];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned())).unwrap();
        let handle = before.resolve(&id("example:glow")).unwrap();

        // A pack replaced the file with one that doesn't compile: the pipeline is invalid in
        // the new load, and the old handle is stale there, not silently redirected.
        let broken = Pipelines::build(&registered, 8, |_| Ok("garbage".to_owned())).unwrap();
        assert_eq!(broken.slot(handle), Err(RenderPipelineError::Stale));
        assert_eq!(
            broken.resolve(&id("example:glow")),
            Err(RenderPipelineError::Invalid)
        );

        let fixed = Pipelines::build(&registered, 9, |_| Ok(GLOW.to_owned())).unwrap();
        assert_eq!(fixed.slot(handle), Err(RenderPipelineError::Stale));
        let again = fixed.resolve(&id("example:glow")).unwrap();
        assert_eq!(fixed.slot(again), Ok(0));
    }

    /// Sprites are named as the 2D atlas names them; an id naming a file is refused by name,
    /// and a required pipeline's refusal fails the load with the named cause.
    #[test]
    fn a_textured_pipeline_names_its_sprite_and_refuses_a_file_path() {
        use mod_api::PipelineTexture;
        let pipeline = |id: &str| {
            let name = Identifier::parse("test:glow").unwrap();
            RenderPipeline::new(
                name.clone(),
                name,
                PipelineBlend::Additive,
                WorldDepth::Tested,
            )
            .textured(PipelineTexture::Sprite(Identifier::parse(id).unwrap()))
        };
        let shader = |_: &str| Ok(GLOW.to_owned());
        let built = |id| {
            Pipelines::build(&[pipeline(id)], 1, shader)
                .map(|set| set.built()[0].as_ref().unwrap().texture.clone().unwrap())
        };
        assert_eq!(
            built("minecraft:block/stone"),
            Ok(BuiltTexture::Sprite("block/stone".into()))
        );
        assert_eq!(
            built("powah:block/energy_charge"),
            Ok(BuiltTexture::Sprite("powah:block/energy_charge".into()))
        );
        assert_eq!(
            load_texture(&PipelineTexture::Sprite(
                Identifier::parse("test:textures/glow").unwrap()
            )),
            Err(PipelineTextureError::NotASpriteKey)
        );
        assert_eq!(
            built("test:effect/glow.png"),
            Err("required render pipeline test:glow from mod  refused: its texture: NotASpriteKey: the id names a file, not a sprite key".into())
        );
    }

    #[test]
    fn register_checks_builtin_required_and_optional_registries_and_duplicates() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterRenderPipelinesEvent, RegistrationContext};

        assert!(BUILTIN_REQUIRED_PIPELINES.windows(2).all(|w| w[0] < w[1]));
        assert!(BUILTIN_OPTIONAL_PIPELINES.windows(2).all(|w| w[0] < w[1]));

        struct Register(&'static [(&'static str, bool)]);
        #[mod_api::__subscriptions]
        impl Hooks for Register {
            fn register_render_pipelines(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterRenderPipelinesEvent<'_>,
            ) {
                for &(name, optional) in self.0 {
                    let p = pipeline(name, name);
                    event.register(if optional { p.optional() } else { p });
                }
            }
        }

        // Free `minecraft:` location plus cross-registry reuse (optional at a built-in
        // required location, required at a built-in optional location) are all allowed.
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "first",
                    Box::new(Register(&[
                        ("minecraft:glow", false),
                        ("minecraft:pipeline/solid_terrain", true),
                        ("minecraft:pipeline/wireframe", false),
                        ("other:beam", false),
                    ])),
                ),
                ModEntry::new("second", Box::new(Register(&[("second:halo", false)]))),
            ]
            .into(),
            || {
                let mut staged = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged,
                );
                let slot = OnceLock::new();
                register_into(&slot, staged).unwrap();
                let stored = slot.get().unwrap();
                assert_eq!(stored.len(), 5);
                assert_eq!(stored[0].id, id("minecraft:glow"));
                assert_eq!(stored[0].owner(), "first");
                assert_eq!(stored[1].id, id("minecraft:pipeline/solid_terrain"));
                assert!(stored[1].optional);
                assert_eq!(stored[2].id, id("minecraft:pipeline/wireframe"));
                assert!(!stored[2].optional);
                assert_eq!(stored[3].id, id("other:beam"));
                assert_eq!(stored[4].id, id("second:halo"));
                assert_eq!(stored[4].owner(), "second");
            },
        );

        // Built-in required collision fails naming the ID and mod.
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "first",
                Box::new(Register(&[("minecraft:pipeline/solid_terrain", false)])),
            )]
            .into(),
            || {
                let mut staged = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged,
                );
                let slot = OnceLock::new();
                let error = register_into(&slot, staged).unwrap_err();
                assert!(
                    error.contains("minecraft:pipeline/solid_terrain"),
                    "{error}"
                );
                assert!(error.contains("first"), "{error}");
                assert!(slot.get().is_none());
            },
        );

        // Built-in optional collision fails naming the ID and mod.
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "first",
                Box::new(Register(&[("minecraft:pipeline/wireframe", true)])),
            )]
            .into(),
            || {
                let mut staged = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged,
                );
                let slot = OnceLock::new();
                let error = register_into(&slot, staged).unwrap_err();
                assert!(error.contains("minecraft:pipeline/wireframe"), "{error}");
                assert!(error.contains("first"), "{error}");
                assert!(slot.get().is_none());
            },
        );

        // Mod IDs collide even across required and optional registrations: lookup uses only the ID.
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("first", Box::new(Register(&[("shared:glow", false)]))),
                ModEntry::new("second", Box::new(Register(&[("shared:glow", true)]))),
            ]
            .into(),
            || {
                let mut staged = Vec::new();
                mod_api::dispatch_register_render_pipelines(
                    &RegistrationContext::new(),
                    &mut staged,
                );
                let slot = OnceLock::new();
                let error = register_into(&slot, staged).unwrap_err();
                assert!(error.contains("shared:glow"), "{error}");
                assert!(error.contains("first"), "{error}");
                assert!(error.contains("second"), "{error}");
                assert!(slot.get().is_none());
            },
        );
    }
}
