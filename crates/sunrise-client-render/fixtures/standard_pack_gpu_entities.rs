//! Original asset-free standard-pack entity fixture. Run with throwaway SUNRISE_HOME,
//! SUNRISE_LAUNCHER_HOME, XDG_DATA_HOME and XDG_CONFIG_HOME (AGENTS.md).
//! `python3 tools/run_standard_pack_gpu_entities.py <output-dir>`
use client::{
    fog::{FogType, sky_pass_present},
    gfx::{
        Frame, Renderer, ScreenshotRequest,
        entity_skin::{SkinFrame, SkinJob, SkinPart, SkinVertex},
        pack_draw::EntitySpan,
        world_primitives::{Batch, DrawRanges, FrameHooks, PreparedHookGeometry},
    },
    graphics::GraphicsApi,
    hud::HudBatch,
    render_geometry::Vertex,
    render_slots as native_render,
};
use mod_api::RenderLevelStage as Stage;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use sunrise_client_render as client;
use sunrise_world::dimension_type::Skybox;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

static NEEDS_SPANS: AtomicBool = AtomicBool::new(false);
static OBSERVED: AtomicUsize = AtomicUsize::new(0);
fn observe_needs(needs: &mut native_render::RenderNeeds) {
    needs.entity_spans |= NEEDS_SPANS.load(Ordering::Relaxed);
}
fn observe_world(world: &mut native_render::WorldDrawn<'_>) {
    if NEEDS_SPANS.load(Ordering::Relaxed) {
        assert_eq!(world.entities.len(), 12);
        assert_eq!(
            world.spans.iter().map(|s| s.id).collect::<Vec<_>>(),
            [77, 88, 99]
        );
        OBSERVED.fetch_add(1, Ordering::Relaxed);
    }
}
fn install_observer() {
    native_render::install(native_render::Slots {
        world_views: |_| {},
        world_views_hooked: || false,
        world_view_drawn: |_| {},
        world_drawn: observe_world,
        hand_drawn: |_| {},
        render_needs: observe_needs,
        world_drawn_hooked: || true,
    });
}

fn pack(renderer: &mut Renderer, folder: &Path, valid: bool, classic_sky_and_hand: bool) {
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    let vertex = "#version 120\nvarying vec2 uv; void main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}\n";
    let fragment = "#version 120\nuniform int entityId; uniform int blockEntityId; uniform vec4 entityColor; void main(){gl_FragData[0]=vec4(float(entityId)/255.0,entityColor.a,float(blockEntityId)/255.0,1.0);}\n";
    // The terrain programs keep the load free of "nothing draws it" notes; this
    // fixture draws no terrain.
    for program in [
        "gbuffers_entities",
        "gbuffers_block",
        "gbuffers_entities_translucent",
        "gbuffers_terrain",
        "gbuffers_water",
    ] {
        std::fs::write(shaders.join(format!("{program}.vsh")), vertex).unwrap();
        std::fs::write(
            shaders.join(format!("{program}.fsh")),
            if valid {
                fragment
            } else {
                "#version 120\nvoid main(){ invalid shader; }"
            },
        )
        .unwrap();
    }
    let (final_fragment, properties) = if classic_sky_and_hand {
        for (program, vertex, fragment) in [
            (
                "gbuffers_skybasic",
                "gl_Position=ftransform();",
                "vec4(1.0,0.0,0.0,1.0)",
            ),
            (
                "gbuffers_hand",
                "gl_Position=vec4(gl_Vertex.xy,-0.6,1.0);",
                "vec4(0.0,1.0,0.0,1.0)",
            ),
        ] {
            std::fs::write(
                shaders.join(format!("{program}.vsh")),
                format!("#version 120\nvoid main(){{{vertex}}}"),
            )
            .unwrap();
            std::fs::write(
                shaders.join(format!("{program}.fsh")),
                format!(
                    "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){{gl_FragData[0]={fragment};}}"
                ),
            )
            .unwrap();
        }
        (
            "#version 120\n\
         varying vec2 uv;
         uniform sampler2D colortex0;
         uniform sampler2D depthtex1;
         uniform sampler2D depthtex2;
         void main() {
             float withHand = texture2D(depthtex1, uv).r;
             float withoutHand = texture2D(depthtex2, uv).r;
             gl_FragColor = vec4(texture2D(colortex0, uv).rg, withoutHand > withHand ? 1.0 : 0.0, 1.0);
         }",
            "oldLighting=false\nshadow.enabled=false\nblend.gbuffers_hand=off\n",
        )
    } else {
        (
            "#version 120\nvarying vec2 uv; uniform sampler2D colortex0; void main(){gl_FragColor=texture2D(colortex0,uv);}\n",
            "oldLighting=false\nshadow.enabled=false\n",
        )
    };
    std::fs::write(shaders.join("final.vsh"), vertex).unwrap();
    std::fs::write(shaders.join("final.fsh"), final_fragment).unwrap();
    std::fs::write(shaders.join("shaders.properties"), properties).unwrap();
    load_pack(renderer, folder, valid);
}

fn load_pack(renderer: &mut Renderer, folder: &Path, valid: bool) {
    let pack = sunrise_shaders::Pack::open(folder).unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    renderer.define_pack_macros(&mut macros);
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &renderer.pack_inputs());
    assert_eq!(post.notes.is_empty(), valid, "{:?}", post.notes);
    let (_, notes) = renderer
        .set_shader_pack(0, Some(post), &[])
        .unwrap()
        .unwrap();
    assert!(
        !valid || notes.iter().all(|n| !n.contains("skipped")),
        "{notes:?}"
    );
}

fn read(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    bytes.truncate(info.buffer_size());
    bytes
}

#[derive(Default)]
struct WorldStageProbe {
    copies: Vec<Vertex>,
    feature_lines: Option<[[f32; 8]; 2]>,
    feature_draws: [DrawRanges; 2],
    stages: Vec<mod_api::RenderLevelStage>,
    prepared_after_sky: Option<bool>,
}

impl FrameHooks for WorldStageProbe {
    fn on_custom_geometry(&mut self, _: &[mod_api::RenderableSection], _: bool) {}
    fn on_after_sky(&mut self, _: &[mod_api::RenderableSection], _: bool) {
        self.stages.push(mod_api::RenderLevelStage::AfterSky);
    }
    fn on_prepare_render_buffers(
        &mut self,
        _: &[mod_api::RenderableSection],
        _: bool,
        out: &mut PreparedHookGeometry,
    ) {
        self.prepared_after_sky =
            Some(self.stages.last() == Some(&mod_api::RenderLevelStage::AfterSky));
        out.entities.append(&mut self.copies);
        if !out.entities.is_empty() {
            out.entity_spans.push(EntitySpan {
                start: 0,
                stage: sunrise_shaders::render_stage("BLOCK_ENTITIES"),
                id: 123,
                ..Default::default()
            });
        }
    }
    fn on_render_stage(
        &mut self,
        stage: mod_api::RenderLevelStage,
        _: &[mod_api::RenderableSection],
        _: bool,
    ) -> &[DrawRanges] {
        self.stages.push(stage);
        match stage {
            Stage::AfterOpaqueFeatures if self.feature_lines.is_some() => &self.feature_draws[..1],
            Stage::AfterTranslucentFeatures if self.feature_lines.is_some() => {
                &self.feature_draws[1..]
            }
            _ => &[],
        }
    }
    fn prepared_primitives(&self) -> Batch<'_> {
        let Some(lines) = &self.feature_lines else {
            return Batch::default();
        };
        Batch {
            lines_through: &lines[..1],
            lines: &lines[1..],
            stage_ends: [[0, 0, 1, 1]; client::gfx::world_primitives::STAGES - 1],
            ..Default::default()
        }
    }
    fn on_after_level(&mut self, _: &[mod_api::RenderableSection], _: bool) {
        self.stages.push(mod_api::RenderLevelStage::AfterLevel);
    }
}

/// Draws actual translucent entities over an opaque-stage line, and checks that the
/// translucent-stage line reaches the picture afterwards on both pack timings.
fn feature_stage_order(
    renderer: &mut Renderer,
    frame: Frame<'_>,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use sunrise_render_core::TRANSLUCENT_ITEM_OVERLAY;

    let quad = |x: f32, y: f32, z: f32, layer: u32, color: u32| {
        [[-x, -y, z], [-x, y, z], [x, y, z], [x, -y, z]].map(|pos| Vertex {
            pos,
            uv: [0.5; 2],
            layer,
            color,
            light: 0xf0f0,
            normal: [0, 0, 127, 0],
            glint_uv: [0.0; 2],
        })
    };
    let mut entities = Vec::from(quad(1.8, 1.2, 5.0, 0, u32::MAX));
    entities.extend(quad(0.55, 1.0, 4.75, TRANSLUCENT_ITEM_OVERLAY, 0xff00_ff00));
    let entity_stage = sunrise_shaders::render_stage("ENTITIES");
    let spans = [
        EntitySpan {
            stage: entity_stage,
            id: 200,
            ..Default::default()
        },
        EntitySpan {
            start: 4,
            stage: entity_stage,
            id: 37,
            translucent: true,
            ..Default::default()
        },
    ];
    let frame = Frame {
        skin: None,
        entities: &entities,
        entity_spans: &spans,
        skybox: false,
        sky_pass: false,
        ..frame
    };
    for (name, separate) in [
        ("engine", None),
        ("pack-early", Some(false)),
        ("pack-late", Some(true)),
    ] {
        if let Some(separate) = separate {
            let folder = out.join("valid-pack");
            std::fs::write(
                folder.join("shaders/shaders.properties"),
                format!(
                    "oldLighting=false\nshadow.enabled=false\nseparateEntityDraws={separate}\n"
                ),
            )?;
            load_pack(renderer, &folder, true);
        }
        let mut hooks = WorldStageProbe {
            feature_lines: Some([
                [
                    -1.4,
                    0.0,
                    5.0,
                    1.4,
                    0.0,
                    5.0,
                    f32::from_bits(0xffff_0000),
                    4.0,
                ],
                [
                    -0.32,
                    0.4,
                    4.5,
                    0.32,
                    0.4,
                    4.5,
                    f32::from_bits(0xff00_00ff),
                    4.0,
                ],
            ]),
            feature_draws: [
                DrawRanges {
                    primitives: [0..0, 0..0, 0..0, 0..1],
                    ..Default::default()
                },
                DrawRanges {
                    primitives: [0..0, 0..0, 0..1, 0..0],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let path = out.join(format!("feature-order-{name}.png"));
        renderer.screenshot = Some(ScreenshotRequest {
            path: path.clone(),
            size: None,
            scaled: false,
            background: false,
        });
        renderer.render(&frame, Some(&mut hooks))?;
        renderer
            .screenshot_result
            .take()
            .expect("feature order readback")?
            .save()?;
        let image = read(&path);
        let pixels = image.as_chunks::<4>().0;
        let center = pixels[90 * 320 + 160];
        if separate.is_some() {
            assert_eq!(
                center[0], 37,
                "{name}: translucent pack entity covers the opaque-stage line"
            );
        } else {
            assert!(
                center[1] > center[0] && center[1] > center[2],
                "{name}: translucent item covers the opaque-stage line: {center:?}"
            );
        }
        assert_eq!(
            pixels[90 * 320 + 180][..3],
            [255, 0, 0],
            "{name}: opaque-stage line survives over the opaque entity outside the translucent item"
        );
        assert!(
            pixels
                .as_chunks::<320>()
                .0
                .iter()
                .skip(70)
                .take(20)
                .any(|row| row[160][..3] == [0, 0, 255]),
            "{name}: translucent-stage line draws on top of the translucent entity"
        );
        let feature_stages: Vec<_> = hooks
            .stages
            .into_iter()
            .filter(|s| {
                matches!(
                    s,
                    Stage::AfterOpaqueFeatures | Stage::AfterTranslucentFeatures
                )
            })
            .collect();
        assert_eq!(
            feature_stages,
            [Stage::AfterOpaqueFeatures, Stage::AfterTranslucentFeatures],
            "{name}"
        );
    }
    renderer.drop_shader_packs();
    Ok(())
}

fn fixture(window: Arc<Window>, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    install_observer();
    let white = [255; 16 * 16 * 4];
    let zero = [0; 4];
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        true,
        &white,
        16,
        16,
        1,
        &[],
        &[],
        (&white, 16, 16),
        None,
        (&zero, 1, 1),
        (&zero, 1, 1),
        (&zero, 1),
        Arc::new(client::gfx::panorama::Faces::colour([0, 0, 0, 255])),
        None,
        sunrise_client_render::gfx::RendererDevice::New,
        GraphicsApi::Vulkan,
        false,
        (true, true),
        &client::graphics::Graphics::default(),
        Default::default(),
        Default::default(),
    ))?;
    renderer.resize(320, 180);
    assert!(
        renderer.skins_entities(),
        "fixture requires compute skinning"
    );
    let unmodified_tagged = renderer.skins_tagged();
    pack(&mut renderer, &out.join("valid-pack"), true, false);
    assert!(
        !renderer.skins_tagged(),
        "eye/tagged layers keep the CPU path with a pack"
    );

    let spans = [77, 88, 99]
        .into_iter()
        .enumerate()
        .map(|(i, id)| EntitySpan {
            start: i as u32 * 4,
            stage: sunrise_shaders::render_stage("ENTITIES"),
            id,
            hidden: i == 2,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let mut skin = SkinFrame {
        reset: true,
        vertices: 12,
        spans: spans.clone(),
        parts: vec![SkinPart {
            matrix: glam::Mat4::IDENTITY.to_cols_array(),
            normal: glam::Mat3::IDENTITY.to_cols_array(),
        }],
        ..Default::default()
    };
    for (i, x) in [-1.8, 0.3, -0.7].into_iter().enumerate() {
        for (pos, uv) in [
            [x, -0.8, 5.],
            [x, 0.8, 5.],
            [x + 1.4, 0.8, 5.],
            [x + 1.4, -0.8, 5.],
        ]
        .into_iter()
        .zip([[0., 0.], [0., 1.], [1., 1.], [1., 0.]])
        {
            skin.new_model.push(SkinVertex {
                pos,
                uv,
                axis: 5,
                part: 0,
            });
        }
        skin.jobs.push(SkinJob {
            first: i as u32 * 4,
            dst: i as u32 * 4,
            count: 4,
            rect: [0., 0., 1., 1.],
            color: u32::MAX,
            light: 0xf0f0,
            overlay: if i == 0 { 1 } else { 5 },
            ..Default::default()
        });
    }
    let cpu = client::gfx::entity_skin::expand(&skin.new_model, &skin);
    let hud = HudBatch::scratch([320, 180]);
    let mut frame = fixture_frame(&hud);
    frame.title = false;
    frame.camera = glam::DVec3::ZERO;
    frame.eye = glam::DVec3::ZERO;
    frame.pitch = 0.;
    frame.entities = &cpu;
    frame.entity_spans = &spans;
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("cpu.png"),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, None)?;
    renderer
        .screenshot_result
        .take()
        .expect("CPU readback")?
        .save()?;
    let expected = read(&out.join("cpu.png"));
    let matches = expected
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] == [77, 77, 0])
        .count();
    assert!(
        matches > 100,
        "original entity-ID/hurt output, got {matches} pixels"
    );
    let white = expected
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[..3] == [88, 192, 0])
        .count();
    assert!(
        white > 100,
        "original entity-ID/white-flash output, got {white} pixels"
    );
    assert_eq!(
        expected
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] == 99)
            .count(),
        0,
        "hidden caster is not camera-visible"
    );
    assert!(
        renderer.skins_entities(),
        "compiled standard-pack stages must accept GPU posing"
    );
    frame.entities = &[];
    frame.entity_spans = &[];
    frame.skin = Some(&skin);
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("gpu.png"),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, None)?;
    renderer
        .screenshot_result
        .take()
        .expect("GPU readback")?
        .save()?;
    assert_eq!(
        read(&out.join("gpu.png")),
        expected,
        "CPU and GPU standard-pack pixels"
    );

    NEEDS_SPANS.store(true, Ordering::Relaxed);
    assert!(
        !renderer.skins_entities(),
        "native CPU-stream observer keeps its fallback"
    );
    frame.skin = None;
    frame.entities = &cpu;
    frame.entity_spans = &spans;
    renderer.render(&frame, None)?;
    assert_eq!(OBSERVED.load(Ordering::Relaxed), 1);
    NEEDS_SPANS.store(false, Ordering::Relaxed);
    assert!(renderer.skins_entities());
    // The second stream must retain its block-entity material after the frame's existing
    // ordinary-entity spans. The pack's fragment shader emits the block ID in blue.
    let mut hooks = WorldStageProbe {
        copies: cpu[4..8]
            .iter()
            .map(|vertex| Vertex {
                pos: [vertex.pos[0], vertex.pos[1], vertex.pos[2] - 0.25],
                ..*vertex
            })
            .collect(),
        ..Default::default()
    };
    frame.skybox = false;
    frame.end_sky = false;
    frame.sky_pass = sky_pass_present(FogType::Atmospheric, false, Skybox::None);
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("world-stage-pack.png"),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, Some(&mut hooks))?;
    renderer
        .screenshot_result
        .take()
        .expect("world stage readback")?
        .save()?;
    assert!(
        read(&out.join("world-stage-pack.png"))
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] == 123)
    );
    assert!(
        !hooks.stages.contains(&Stage::AfterSky),
        "a dimension with no skybox has no sky stage"
    );
    assert!(hooks.stages.contains(&Stage::AfterTranslucentParticles));
    assert!(hooks.stages.ends_with(&[
        Stage::AfterOpaqueBlocks,
        Stage::AfterOpaqueFeatures,
        Stage::AfterTranslucentFeatures,
        Stage::AfterTranslucentBlocks,
        Stage::AfterLevel
    ]));
    assert_eq!(hooks.prepared_after_sky, Some(false));
    // Reuse the same renderer without submitting the extra model a second time.
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("world-stage-pack-next.png"),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, Some(&mut WorldStageProbe::default()))?;
    renderer
        .screenshot_result
        .take()
        .expect("next frame readback")?
        .save()?;
    assert!(
        !read(&out.join("world-stage-pack-next.png"))
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] == 123)
    );
    frame.sky_pass = sky_pass_present(FogType::Atmospheric, false, Skybox::Overworld);
    for draw_sky in [true, false] {
        // The Sky option changes pixels, not the stage's admission.
        frame.skybox = draw_sky;
        let mut sky_hooks = WorldStageProbe::default();
        renderer.render(&frame, Some(&mut sky_hooks))?;
        assert_eq!(sky_hooks.stages.first(), Some(&Stage::AfterSky));
        assert_eq!(sky_hooks.prepared_after_sky, Some(true));
    }
    frame.skybox = true;

    // Exercise the actual classic pack route, not just an engine sky with a pack active.
    // The pack's final shader distinguishes sky, hand, and depth1 with the hand vs depth2
    // without it. Adding the live hook must not move the pack hand past either snapshot.
    pack(
        &mut renderer,
        &out.join("classic-boundary-pack"),
        true,
        true,
    );
    let hand = [
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ]
    .map(|pos| Vertex {
        pos,
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light: 0,
        normal: [0, 0, 127, 0],
        glint_uv: [0.0; 2],
    });
    frame.entities = &[];
    frame.entity_spans = &[];
    frame.hand = &hand;
    let mut classic = WorldStageProbe::default();
    for (name, hooks) in [
        ("classic-boundary-control.png", None),
        (
            "classic-boundary-hooked.png",
            Some(&mut classic as &mut dyn FrameHooks),
        ),
    ] {
        renderer.screenshot = Some(ScreenshotRequest {
            path: out.join(name),
            size: None,
            scaled: false,
            background: false,
        });
        renderer.render(&frame, hooks)?;
        renderer
            .screenshot_result
            .take()
            .expect("classic boundary readback")?
            .save()?;
    }
    let pixels = read(&out.join("classic-boundary-hooked.png"));
    assert_eq!(pixels, read(&out.join("classic-boundary-control.png")));
    let (pixels, _) = pixels.as_chunks::<4>();
    assert!(
        pixels.iter().any(|pixel| pixel[..3] == [255, 0, 0]),
        "pack-owned sky"
    );
    assert!(
        pixels.iter().any(|pixel| pixel[..3] == [0, 255, 255]),
        "pack-owned hand only in depthtex1"
    );
    assert_eq!(classic.prepared_after_sky, Some(false));
    assert_eq!(
        classic
            .stages
            .iter()
            .filter(|&&stage| stage == Stage::AfterSky)
            .count(),
        1,
        "the pack's red sky must have exactly one AfterSky notification"
    );
    assert!(
        classic.stages.ends_with(&[
            Stage::AfterSky,
            Stage::AfterOpaqueBlocks,
            Stage::AfterOpaqueFeatures,
            Stage::AfterTranslucentFeatures,
            Stage::AfterTranslucentBlocks,
            Stage::AfterLevel
        ]),
        "the pack sky, block, and feature stages follow the engine world stages and precede AfterLevel: {:?}",
        classic.stages
    );
    assert_eq!(
        classic
            .stages
            .iter()
            .filter(|&&stage| stage == Stage::AfterLevel)
            .count(),
        1
    );
    // A classic pack keeps the inline End sky outside dragon fog. Even with no
    // End sky sprite, that pass owns the notification. In boss fog the pack owns it.
    frame.skybox = false;
    frame.end_sky = true;
    frame.sky_pass = sky_pass_present(FogType::Atmospheric, false, Skybox::End);
    for boss_world_fog in [false, true] {
        frame.boss_world_fog = boss_world_fog;
        frame.sky_blocked = boss_world_fog;
        let mut end = WorldStageProbe::default();
        renderer.render(&frame, Some(&mut end))?;
        assert_eq!(
            end.stages
                .iter()
                .filter(|&&stage| stage == Stage::AfterSky)
                .count(),
            1,
            "classic-pack End with boss fog {boss_world_fog}"
        );
        assert_eq!(end.prepared_after_sky, Some(false));
        if boss_world_fog {
            assert!(end.stages.ends_with(&[
                Stage::AfterSky,
                Stage::AfterOpaqueBlocks,
                Stage::AfterOpaqueFeatures,
                Stage::AfterTranslucentFeatures,
                Stage::AfterTranslucentBlocks,
                Stage::AfterLevel
            ]));
        } else {
            assert_eq!(end.stages.first(), Some(&Stage::AfterSky));
        }
    }
    // The classic pack still draws its boss-fog fan, but powder snow vetoes the stage.
    frame.sky_pass = sky_pass_present(FogType::PowderSnow, false, Skybox::End);
    let mut blocked_sky = WorldStageProbe::default();
    renderer.render(&frame, Some(&mut blocked_sky))?;
    assert!(
        !blocked_sky.stages.contains(&Stage::AfterSky),
        "powder-snow camera fog has no sky stage, even with a pack sky"
    );
    frame.skybox = true;
    frame.end_sky = false;
    frame.boss_world_fog = false;
    frame.sky_blocked = false;
    frame.sky_pass = sky_pass_present(FogType::Atmospheric, false, Skybox::Overworld);
    frame.hand = &[];

    pack(&mut renderer, &out.join("failed-pack"), false, false);
    assert!(
        !renderer.skins_entities(),
        "failed entity programs retain CPU posing"
    );
    renderer.drop_shader_packs();
    assert!(
        renderer.skins_entities(),
        "ordinary path returns when the pack is dropped"
    );
    assert_eq!(renderer.skins_tagged(), unmodified_tagged);

    // A pack can claim the sun without replacing the disc. Its marker must be drawn before
    // the sole AfterSky notice, and the blue engine background must still reach the pack.
    let folder = out.join("celestial-only-pack");
    let shaders = folder.join("shaders");
    std::fs::create_dir_all(&shaders)?;
    // The fixture has no terrain; declare both programs to keep pack diagnostics about the sky.
    for program in ["gbuffers_terrain", "gbuffers_water"] {
        std::fs::write(
            shaders.join(format!("{program}.vsh")),
            "#version 120\nvoid main(){gl_Position=ftransform();}",
        )?;
        std::fs::write(
            shaders.join(format!("{program}.fsh")),
            "#version 120\nvoid main(){gl_FragData[0]=vec4(1.0);}",
        )?;
    }
    for (file, source) in [
        (
            "gbuffers_skytextured.vsh",
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy/100.0,0.9,1.0);}",
        ),
        (
            "gbuffers_skytextured.fsh",
            "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(0.0,1.0,0.0,1.0);}",
        ),
        (
            "final.vsh",
            "#version 120\nvarying vec2 uv; void main(){gl_Position=ftransform(); uv=gl_MultiTexCoord0.xy;}",
        ),
        (
            "final.fsh",
            "#version 120\nvarying vec2 uv; uniform sampler2D colortex0; void main(){gl_FragColor=texture2D(colortex0,uv);}",
        ),
        (
            "shaders.properties",
            "shadow.enabled=false\nblend.gbuffers_skytextured=off\n",
        ),
    ] {
        std::fs::write(shaders.join(file), source)?;
    }
    load_pack(&mut renderer, &folder, true);
    let celestial = [
        [-25.0, -25.0, 100.0],
        [25.0, -25.0, 100.0],
        [25.0, 25.0, 100.0],
        [-25.0, 25.0, 100.0],
    ]
    .map(|pos| Vertex { pos, ..hand[0] });
    frame.sky.sky_color = [0.0, 0.0, 1.0];
    frame.sky.fog_color = [0.0; 3];
    // The sky disc's fog ends at the render distance, in blocks: the fixture's 12 hides the
    // whole disc, which lies at least 16 blocks away, in the black fog. 12 chunks shows it.
    let fixture_distance = std::mem::replace(&mut frame.distance, 192.0);
    let mut celestial_hooks = WorldStageProbe::default();
    frame.celestial = &celestial;
    frame.celestial_bodies = &[client::gfx::CelestialBody::Sun];
    renderer.screenshot = Some(ScreenshotRequest {
        path: out.join("celestial-only.png"),
        size: None,
        scaled: false,
        background: false,
    });
    renderer.render(&frame, Some(&mut celestial_hooks))?;
    renderer
        .screenshot_result
        .take()
        .expect("celestial pack readback")?
        .save()?;
    let celestial_image = read(&out.join("celestial-only.png"));
    let pixels = celestial_image.as_chunks::<4>().0;
    assert_eq!(pixels[90 * 320 + 160][..3], [0, 255, 0]);
    assert!(
        pixels
            .iter()
            .any(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] > 0),
        "the engine disc was not drawn"
    );
    assert_eq!(celestial_hooks.prepared_after_sky, Some(false));
    assert_eq!(
        celestial_hooks
            .stages
            .iter()
            .filter(|&&stage| stage == Stage::AfterSky)
            .count(),
        1
    );
    assert!(celestial_hooks.stages.ends_with(&[
        Stage::AfterSky,
        Stage::AfterOpaqueBlocks,
        Stage::AfterOpaqueFeatures,
        Stage::AfterTranslucentFeatures,
        Stage::AfterTranslucentBlocks,
        Stage::AfterLevel,
    ]));
    frame.celestial = &[];
    frame.celestial_bodies = &[];
    frame.distance = fixture_distance;
    renderer.drop_shader_packs();

    let mut ordering_frame = fixture_frame(&hud);
    ordering_frame.title = false;
    ordering_frame.camera = glam::DVec3::ZERO;
    ordering_frame.eye = glam::DVec3::ZERO;
    ordering_frame.pitch = 0.0;
    ordering_frame.eye_rotation = [0.0; 2];
    feature_stage_order(&mut renderer, ordering_frame, out)?;

    // A native sky uses the same no-pass notification boundary, even with its own timing pass.
    use sunrise_shader_api::{MANIFEST, Pack, check};
    let files = [
        (MANIFEST, "schema = 1\nid = \"test\"\nname = \"Test\"\nversion = \"1\"\napi = \"0.1\"\n[sky]\nfile = \"sky.wgsl\"\n"),
        ("sky.wgsl", "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }"),
    ].map(|(name, text)| (name.to_owned(), text.as_bytes().to_vec())).into();
    let report = check(&Pack::from_files(files).expect("native sky pack")).expect("valid sky");
    renderer.set_native_shader_pack(Some((report, vec![], vec![], vec![], Default::default())))?;
    renderer.set_pass_timing(true);
    let mut native_sky_hooks = WorldStageProbe::default();
    renderer.render(&frame, Some(&mut native_sky_hooks))?;
    assert_eq!(native_sky_hooks.stages.first(), Some(&Stage::AfterSky));
    assert_eq!(native_sky_hooks.prepared_after_sky, Some(true));

    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "cpu_gpu_pixels_equal": true, "entity_id_hurt_pixels": matches, "hidden_subject_pixels": 0,
                "entity_id_white_flash_pixels": white,
                "graphics_adapter": client::crash::system_details("fixture").into_iter()
                    .find(|(name, _)| name == "Graphics Adapter").map(|(_, detail)| detail),
            "native_observer_frames": OBSERVED.load(Ordering::Relaxed), "player_game_started": false,
            "fixture_only_not_gameplay_or_fps_proof": true
        }))?,
    )?;
    Ok(())
}

struct App<'a>(&'a Path);
impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .unwrap(),
        );
        if let Err(error) = fixture(window, self.0) {
            eprintln!("fixture failed: {error}");
            std::process::exit(1);
        }
        event_loop.exit();
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}
fn run(out: &Path) {
    for key in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        assert!(
            std::env::var_os(key).is_some(),
            "set throwaway {key} (AGENTS.md)"
        );
    }
    let mut builder = EventLoop::builder();
    #[cfg(all(test, target_os = "linux"))]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_any_thread(true);
    }
    #[cfg(all(test, target_os = "windows"))]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    builder.build().unwrap().run_app(&mut App(out)).unwrap();
}

#[cfg(not(test))]
pub(crate) fn main() {
    let output = std::env::args_os()
        .nth(1)
        .expect("output directory required");
    run(Path::new(&output));
}

#[cfg(test)]
pub(crate) fn run_daily() {
    const OUTPUT: &str = "SUNRISE_GPU_ENTITY_FIXTURE_OUTPUT";
    if let Some(output) = std::env::var_os(OUTPUT) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("standard_pack_gpu_entities refused: no Vulkan adapter");
        run(Path::new(&output));
        return;
    }

    // Keep render slots, the event loop and every writable home in a fresh process.
    let scratch = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "standard_pack_gpu_entities",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(OUTPUT, scratch.path().join("output"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("SUNRISE_NO_AUDIO", "1")
        .env("SUNRISE_TELEMETRY_URL", "off");
    for name in [
        "SUNRISE_HOME",
        "SUNRISE_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_RUNTIME_DIR",
        "SUNRISE_LAUNCHER_TRASH",
        "SUNRISE_LAUNCHER_CLIPBOARD",
        "SUNRISE_LAUNCHER_DESKTOP",
    ] {
        let home = scratch.path().join(name);
        std::fs::create_dir(&home).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        command.env(name, home);
    }
    assert!(
        command.status().unwrap().success(),
        "GPU entity fixture failed"
    );
}

fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        roll: 0.0,
        fov_degrees: 85.0,
        cull_fov_degrees: 85.0,
        eye: glam::DVec3::new(0.5, 100.0, 0.5),
        eye_rotation: [0.0, 10.0],
        first_person: true,
        bob: glam::Mat4::IDENTITY,
        view_transform: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: client::environment::Sky::default(),
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        distance: 12.0,
        detail_distance: 12.0,
        shadow_distance: None,
        depth_far: 1000.0,
        underwater: false,
        water_plane: None,
        cave_culling: false,
        title: true,
        sky_pass: false,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        boss_world_fog: false,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        pvp: Default::default(),
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        explicit_particles: None,
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: Default::default(),
        entity_motion: &[],
        entity_motion_posed: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        colored_lights: &[],
        emissive_layers: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        world_primitives: Default::default(),
        shadow_only: &[],
        celestial: &[],
        celestial_bodies: &[],
        stars: None,
        hand: &[],
        hand_items: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        chunk_fade: 0.0,
        post_effects: client::post_chain::Request::none(),
        menu_blur: None,
        menu_blur_cache: false,
    }
}
