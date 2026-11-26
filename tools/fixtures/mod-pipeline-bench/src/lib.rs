//! The mod `tools/mod_pipeline_bench.py` measures: it registers [`PIPELINES`] render pipelines,
//! half of them textured, and draws [`QUADS_PER_LAYER`] quads through them across four depth
//! layers in `submit_custom_geometry`, every frame, in front of the camera. The driver writes the
//! GLSL core shaders (`assets/pipeline_bench/shaders/core/pNN.{vsh,fsh}`) when it stages the mod,
//! and the benchmark's game (`crates/sunrise-mod-host/examples/mod_pipeline_bench.rs`) links it.
use mod_api::{
    ClientHost, ClientReloadHost, ClientResourcesReloadedEvent, ClientSetupContext,
    ClientSetupEvent, Hooks, Identifier, PipelineBlend, PipelineQuad, PipelineTexture,
    RegisterRenderPipelinesEvent, RegistrationContext, RenderAssets, RenderPipeline,
    RenderPipelineRef, SubmitCustomGeometryEvent, WorldDepth,
};
use std::sync::Mutex;

pub const ID: &str = "pipeline_bench";
pub const VERSION: &str = "0.1.0";
/// Pipelines registered; those from `PIPELINES / 2` on sample a block sprite.
pub const PIPELINES: usize = 32;
/// Quads drawn per depth layer (`LAYERS` layers total), spread over that layer's share of the
/// pipelines.
pub const QUADS_PER_LAYER: usize = 500;
const LAYERS: usize = 4;
const PER_LAYER: usize = PIPELINES / LAYERS;

fn id(index: usize) -> Identifier {
    Identifier::parse(&format!("{ID}:p{index:02}")).expect("bench pipeline id")
}

fn shader_id(index: usize) -> Identifier {
    Identifier::parse(&format!("{ID}:core/p{index:02}")).expect("bench shader id")
}

#[derive(Default)]
pub struct Bench {
    pipelines: Mutex<Vec<RenderPipelineRef>>,
    quads: Mutex<Vec<PipelineQuad>>,
}

impl Bench {
    fn resolve(&self, assets: &dyn RenderAssets) {
        let resolved: Vec<_> = (0..PIPELINES)
            .filter_map(|index| assets.render_pipeline(&id(index)).ok())
            .collect();
        if resolved.len() != PIPELINES {
            eprintln!(
                "pipeline_bench: {} of {PIPELINES} pipelines resolved",
                resolved.len()
            );
        }
        *self.pipelines.lock().unwrap() = resolved;
    }
}

#[mod_api::sunrise_mod]
impl Hooks for Bench {
    fn register_render_pipelines(
        &self,
        _context: &RegistrationContext,
        event: &mut RegisterRenderPipelinesEvent<'_>,
    ) {
        let stone = Identifier::parse("minecraft:block/stone").expect("stone sprite id");
        for index in 0..PIPELINES {
            let blend = if index % 2 == 0 {
                PipelineBlend::Translucent
            } else {
                PipelineBlend::Additive
            };
            let pipeline =
                RenderPipeline::new(id(index), shader_id(index), blend, WorldDepth::Tested);
            event.register(if index >= PIPELINES / 2 {
                pipeline.textured(PipelineTexture::Sprite(stone.clone()))
            } else {
                pipeline
            });
        }
    }

    fn client_setup(&self, context: &mut dyn ClientSetupContext, _: &mut ClientSetupEvent) {
        self.resolve(context.render_assets());
    }

    fn client_resources_reloaded(
        &self,
        context: &dyn ClientReloadHost,
        _: &mut ClientResourcesReloadedEvent,
    ) {
        self.resolve(context.render_assets());
    }

    fn submit_custom_geometry(
        &self,
        _: &dyn ClientHost,
        event: &mut SubmitCustomGeometryEvent<'_>,
    ) {
        let pipelines = self.pipelines.lock().unwrap();
        if pipelines.len() != PIPELINES {
            return;
        }
        let [x, y, z] = event.view.camera.position;
        let mut quads = self.quads.lock().unwrap();
        // Four walls of quads 8..11 blocks ahead along +z, 25 across, spread across all pipelines.
        let share = QUADS_PER_LAYER.div_ceil(PER_LAYER);
        for layer in 0..LAYERS {
            for (n, pipeline) in pipelines[layer * PER_LAYER..][..PER_LAYER]
                .iter()
                .enumerate()
            {
                quads.clear();
                quads.extend(
                    (n * share..QUADS_PER_LAYER.min((n + 1) * share)).map(|quad| {
                        let left = x + (quad % 25) as f64 * 0.5 - 6.25;
                        let bottom = y + (quad / 25) as f64 * 0.5 - 5.0;
                        let depth = z + 8.0 + layer as f64;
                        PipelineQuad::spanning(
                            [
                                [left, bottom, depth],
                                [left, bottom + 0.45, depth],
                                [left + 0.45, bottom + 0.45, depth],
                                [left + 0.45, bottom, depth],
                            ],
                            0x80FF_FFFF,
                        )
                    }),
                );
                if let Err(error) = event.draw.pipeline_quads(*pipeline, &quads) {
                    eprintln!("pipeline_bench: quads refused: {error:?}");
                }
            }
        }
    }
}
