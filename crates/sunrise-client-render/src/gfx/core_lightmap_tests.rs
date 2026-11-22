use super::*;

/// Two states: a dim, tinted End-like one (green ambient, no sky light) and a night with
/// night vision, a boss bar and some darkness.
const STATES: [LightmapInfo; 2] = [
    LightmapInfo {
        sky_factor: 0.0,
        block_factor: 1.4,
        night_vision: 0.0,
        darkness: 0.0,
        boss_darkening: 0.0,
        brightness: 1.0,
        block_light_tint: [1.0, 0.847, 0.549],
        sky_light_color: [0.675, 0.376, 0.804],
        ambient_color: [0.247, 0.278, 0.247],
        night_vision_color: [0.6; 3],
    },
    LightmapInfo {
        sky_factor: 0.701,
        block_factor: 1.448,
        night_vision: 0.325,
        darkness: 0.091,
        boss_darkening: 0.499,
        brightness: 0.473,
        block_light_tint: [1.0, 0.598, 0.319],
        sky_light_color: [0.805, 0.815, 1.0],
        ambient_color: [0.03, 0.023, 0.036],
        night_vision_color: [0.6; 3],
    },
];

/// The probe shader's state: values a unorm byte holds exactly (0.6 is 153 of 255, 0.4 is 102).
const PROBE: LightmapInfo = LightmapInfo {
    sky_factor: 0.6,
    night_vision_color: [0.0, 0.4, 0.0],
    ..STATES[1]
};

/// `LightmapInfo` as `lightmap.fsh` declares it in std140: six floats, then four vec3s each on
/// its own 16 bytes.
#[test]
fn the_lightmap_info_block_is_laid_out_as_std140() {
    let bytes = STATES[1].std140();
    let f: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(f.len(), 24);
    assert_eq!(f[..6], [0.701, 1.448, 0.325, 0.091, 0.499, 0.473]);
    assert_eq!(f[8..11], [1.0, 0.598, 0.319]);
    assert_eq!(f[12..15], [0.805, 0.815, 1.0]);
    assert_eq!(f[16..19], [0.03, 0.023, 0.036]);
    assert_eq!(f[20..23], [0.6; 3]);
    assert_eq!([f[6], f[7], f[11], f[15], f[19], f[23]], [0.0; 6]);
}

/// The adapter's device, or none (said on stderr) where the machine has no adapter.
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let device = crate::gfx::test_gpu::optional_device();
    if device.is_none() {
        eprintln!("skipped: no adapter");
    }
    device.map(|(_, device, queue)| (device, queue))
}

/// A 16x16 light texture to draw into.
fn target(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The light texture `core` draws for `info`, read back through its own readback (the copy,
/// the mapping after the submission, then `collect`): RGBA rows of sky level by block level.
fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    core: &mut CoreLightmap,
    info: &LightmapInfo,
) -> [u8; 16 * 16 * 4] {
    draw_with(device, queue, core, info, None)
}

/// [`draw`] with `poly_globals` uploaded.
fn draw_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    core: &mut CoreLightmap,
    info: &LightmapInfo,
    poly_globals: Option<&super::super::expression_blocks::PolyGlobalsBytes>,
) -> [u8; 16 * 16 * 4] {
    let target = target(device);
    let mut uploads = super::super::upload::Uploads::new(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    core.extra.set_globals(poly_globals);
    core.upload(&mut uploads, &mut encoder, [0; 48], info);
    core.draw(
        &mut encoder,
        &target,
        &target.create_view(&Default::default()),
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    (core.submitted().expect("a readback"))();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mut pixels = [0u8; 16 * 16 * 4];
    assert!(core.collect(&mut pixels), "read back");
    pixels
}

/// A pack's own shader: each texel's block level in red, its sky level in green, `SkyFactor`
/// in blue and `NightVisionColor`'s green in alpha.
const PROBE_FSH: &str = "#version 330\nlayout(std140) uniform LightmapInfo {\n\
    float SkyFactor;\n    float BlockFactor;\n    float NightVisionFactor;\n\
    float DarknessScale;\n    float BossOverlayWorldDarkeningFactor;\n    float BrightnessFactor;\n\
    vec3 BlockLightTint;\n    vec3 SkyLightColor;\n    vec3 AmbientColor;\n    vec3 NightVisionColor;\n\
} lightmapInfo;\n\
    layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
    void main() {\n    float block_level = floor(texCoord.x * 16) / 15;\n\
        float sky_level = floor(texCoord.y * 16) / 15;\n\
        fragColor = vec4(block_level, sky_level, lightmapInfo.SkyFactor, \
        lightmapInfo.NightVisionColor.g);\n}\n";

const SCREENQUAD_VSH: &str = "#version 330\nlayout(location = 0) out vec2 texCoord;\n\
    void main() {\n    vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n\
        gl_Position = vec4(uv * vec2(2, 2) + vec2(-1, -1), 0, 1);\n    texCoord = uv;\n}\n";

/// The pack's shader covers the whole texture with row `n` at sky level `n` and column `n` at
/// block level `n` (GL's rows run from the bottom), reads `LightmapInfo` where `Lightmap.render`
/// writes it, and its pixels come back for F3's lightmap chart.
#[test]
#[ignore = "daily"]
fn a_pack_lightmap_shader_fills_the_texture_by_sky_row_and_block_column() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let program = crate::core_shader::translate_core_with(
        SCREENQUAD_VSH,
        PROBE_FSH,
        &[],
        &|_| None,
        &GLUE,
        &|b| b == INFO_BLOCK,
    )
    .expect("the shaders translate");
    let mut core = CoreLightmap::new(&device, &program, &[]).expect("the lightmap pipeline");
    core.set_readback(&device, true);
    let pixels = draw(&device, &queue, &mut core, &PROBE);
    for sky in 0..16usize {
        for block in 0..16usize {
            let got = &pixels[(sky * 16 + block) * 4..][..4];
            // Level n of 15 is byte 17n; `SkyFactor` 0.6 is 153, `NightVisionColor.g` 0.4 is 102.
            let want = [block as u8 * 17, sky as u8 * 17, 153, 102];
            assert_eq!(got, want, "sky {sky} block {block}");
        }
    }
    // Frames read back out of their buffers' order: the newest one wins. P lands in buffer 0
    // and is taken in; Q goes to buffer 1, R to buffer 0 again, and both land before F3 looks.
    let frame = |core: &mut CoreLightmap, info: &LightmapInfo, poll: bool| {
        let (buffer, data) = core.uniforms([0; 48], info);
        queue.write_buffer(buffer, 0, data);
        let mut encoder = device.create_command_encoder(&Default::default());
        let target = target(&device);
        core.draw(
            &mut encoder,
            &target,
            &target.create_view(&Default::default()),
        );
        queue.submit([encoder.finish()]);
        (core.submitted().expect("a readback"))();
        if poll {
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
    };
    let mut pixels = [0u8; 16 * 16 * 4];
    frame(&mut core, &STATES[0], true);
    frame(&mut core, &PROBE, false);
    assert!(core.collect(&mut pixels), "P");
    frame(&mut core, &STATES[0], true);
    assert!(core.collect(&mut pixels));
    assert_eq!(pixels[2], 0, "R's SkyFactor (0), not Q's (153)");
}

fn resources() -> std::path::PathBuf {
    std::env::var_os("SUNRISE_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        })
}

/// The light textures vanilla's `lightmap.fsh` draws for [`STATES`], every texel
/// (`core_lightmap_vanilla.hex`): not Sunrise's, but `lightmap.fsh`'s math run line by line in
/// float32 by an independent script (numpy, written for this test), rounded to the nearest byte
/// as a unorm target stores it. The states are picked so that every channel of every texel is
/// at least 0.005 of a byte from a rounding tie, far past float32's error, so a driver that
/// stores a unorm byte as the nearest one (lavapipe, which the daily GPU lane runs) stores these
/// bytes. Vulkan lets a driver's float-to-unorm conversion stray a little from the nearest byte:
/// this machine's RADV stores 196 for state 0's texel at sky 0, block 3 (196.574 here), so the
/// test is exact under lavapipe (`VK_ICD_FILENAMES=.../lvp_icd.json`).
fn vanilla_textures() -> [[u8; 16 * 16 * 4]; 2] {
    let mut out = [[0u8; 16 * 16 * 4]; 2];
    let mut state = 0;
    let mut at = 0;
    for line in include_str!("core_lightmap_vanilla.hex").lines() {
        if let Some(n) = line.strip_prefix("# state ") {
            state = n.parse().expect("a state");
            at = 0;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        for texel in line.split(' ') {
            for byte in 0..4 {
                out[state][at] = u8::from_str_radix(&texel[byte * 2..byte * 2 + 2], 16).unwrap();
                at += 1;
            }
        }
    }
    out
}

/// Vanilla's own `screenquad` and `lightmap` through the pack path draw vanilla's light texture
/// exactly, all 256 texels, in two lighting states ([`vanilla_textures`]), row by sky level and
/// column by block level.
#[test]
#[ignore = "daily"]
fn vanillas_lightmap_shaders_draw_vanillas_light_texture() {
    let _gpu = crate::gfx::test_gpu::lock();
    let root = crate::resource_source::ResourceSource::directory(resources());
    let shader = |name: &str| root.read_to_string(&format!("assets/minecraft/shaders/core/{name}"));
    let (Ok(vertex), Ok(fragment)) = (shader("screenquad.vsh"), shader("lightmap.fsh")) else {
        eprintln!(
            "skipped: no prepared resources at {}",
            resources().display()
        );
        return;
    };
    let include = |id: &str| {
        let (namespace, path) = id.split_once(':')?;
        root.read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    };
    let program =
        crate::core_shader::translate_core_with(&vertex, &fragment, &[], &include, &GLUE, &|b| {
            b == INFO_BLOCK
        })
        .expect("vanilla's lightmap shaders translate");
    let Some((device, queue)) = device() else {
        return;
    };
    let mut core = CoreLightmap::new(&device, &program, &[]).expect("the lightmap pipeline");
    core.set_readback(&device, true);
    let expected = vanilla_textures();
    for (info, want) in STATES.iter().zip(expected) {
        let gpu = draw(&device, &queue, &mut core, info);
        for sky in 0..16 {
            for block in 0..16 {
                let i = (sky * 16 + block) * 4;
                assert_eq!(gpu[i..i + 4], want[i..i + 4], "sky {sky} block {block}");
            }
        }
    }
}

/// F3's lightmap chart closed (the default), a frame draws the texture and copies nothing:
/// no readback buffer exists and nothing is mapped after the submission. Opened, the frame's
/// texture comes back; closed again, the buffers go, and a chart opened later starts from new
/// buffers rather than an old frame's copy.
#[test]
#[ignore = "daily"]
fn the_light_texture_is_read_back_only_while_f3_charts_it() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let program = crate::core_shader::translate_core_with(
        SCREENQUAD_VSH,
        PROBE_FSH,
        &[],
        &|_| None,
        &GLUE,
        &|b| b == INFO_BLOCK,
    )
    .expect("the shaders translate");
    let mut core = CoreLightmap::new(&device, &program, &[]).expect("the lightmap pipeline");
    // One frame: its uniforms, its draw, its submission, and the readback it starts (if any).
    let frame = |core: &mut CoreLightmap, info: &LightmapInfo| {
        let (buffer, data) = core.uniforms([0; 48], info);
        queue.write_buffer(buffer, 0, data);
        let mut encoder = device.create_command_encoder(&Default::default());
        let target = target(&device);
        core.draw(
            &mut encoder,
            &target,
            &target.create_view(&Default::default()),
        );
        queue.submit([encoder.finish()]);
        let started = core.submitted().map(|read| read()).is_some();
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        started
    };
    assert!(core.readback.is_none(), "closed when made");
    assert!(!frame(&mut core, &PROBE), "no copy while closed");
    let mut pixels = [7u8; 16 * 16 * 4];
    assert!(!core.collect(&mut pixels));
    assert_eq!(pixels, [7; 16 * 16 * 4], "nothing read while closed");

    core.set_readback(&device, true);
    assert!(frame(&mut core, &PROBE), "a copy while open");
    assert!(core.collect(&mut pixels));
    // PROBE's SkyFactor (0.6, 153 of 255) in blue.
    assert_eq!(pixels[2], 153);

    // A copy is mapped and waiting when the chart closes: the buffers go with it.
    assert!(frame(&mut core, &STATES[0]));
    core.set_readback(&device, false);
    assert!(core.readback.is_none());
    assert!(!frame(&mut core, &STATES[0]), "no copy once closed");
    core.set_readback(&device, true);
    let mut fresh = [7u8; 16 * 16 * 4];
    assert!(
        !core.collect(&mut fresh),
        "the copy from before the close is gone"
    );
    assert_eq!(fresh, [7; 16 * 16 * 4]);
}

/// A pack's lightmap whose vertex stage reads expression uniforms `V0`, `V1`, ... (`vertex` of
/// them) and whose fragment stage reads `F0`, `F1`, ... (`fragment`), with every one's
/// expression from its `shader_modifiers` file, resolved as the atlas resolves them. The
/// fragment writes `F0` in red, `SkyFactor` in green and `F1` in blue.
fn expression_lightmap(vertex: usize, fragment: usize) -> (CoreProgram, Vec<ExpressionUniform>) {
    let block =
        |name: String| format!("layout(std140) uniform {name} {{\n    float {name}v;\n}};\n");
    let blocks = |prefix: &str, n: usize| -> String {
        (0..n).map(|i| block(format!("{prefix}{i}"))).collect()
    };
    let reads = |prefix: &str, n: usize, from: usize| -> String {
        (from..n)
            .map(|i| format!(" + 0.0 * {prefix}{i}v"))
            .collect()
    };
    let vsh = format!(
        "#version 330\n{}layout(location = 0) out vec2 texCoord;\nvoid main() {{\n    \
         vec2 uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);\n    \
         gl_Position = vec4(uv * vec2(2, 2) + vec2(-1, -1), 0.0{}, 1);\n    texCoord = uv;\n}}\n",
        blocks("V", vertex),
        reads("V", vertex, 0)
    );
    let second = if fragment > 1 { "F1v" } else { "0.0" };
    let fsh = format!(
        "#version 330\nlayout(std140) uniform LightmapInfo {{\n    float SkyFactor;\n}} info;\n\
         {}layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
         void main() {{\n    fragColor = vec4(F0v{}, info.SkyFactor, {second}, 1.0);\n}}\n",
        blocks("F", fragment),
        reads("F", fragment, 2)
    );
    let program =
        crate::core_shader::translate_core_with(&vsh, &fsh, &[], &|_| None, &GLUE, &|_| true)
            .expect("the shaders translate");
    let names: Vec<String> = (0..vertex)
        .map(|i| format!("V{i}"))
        .chain((0..fragment).map(|i| format!("F{i}")))
        .collect();
    let file: serde_json::Map<String, serde_json::Value> =
        names.iter().map(|n| (n.clone(), "0".into())).collect();
    let pack = sunrise_assets::resource_source::ResourceSource::indexed(
        [(
            "assets/minecraft/polytone/shader_modifiers/core/lightmap.json".to_owned(),
            sunrise_assets::resource_source::Resource::bytes(
                serde_json::Value::Object(file).to_string().into_bytes(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    let modifiers = crate::polytone_post::ShaderModifiers::read(&pack, Default::default());
    let expressions = modifiers
        .resolve((SHADERS[0], SHADERS[1]), &program, &[INFO_BLOCK])
        .expect("the uniforms resolve");
    (program, expressions)
}

/// The expression uniforms' values (`Renderer::evaluate_pack_expressions`) reach the blocks the
/// fragment stage reads, each its own, beside `LightmapInfo`: exact bytes for values a unorm
/// byte holds (0.2 is 51, 0.6 is 153), the two in channels of their own.
#[test]
#[ignore = "daily"]
fn a_lightmap_shader_reads_its_expression_uniforms_values() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let (program, expressions) = expression_lightmap(0, 2);
    let mut core = CoreLightmap::new(&device, &program, &expressions).expect("the pipeline");
    core.set_readback(&device, true);
    let names: Vec<String> = (core.extra.values().iter())
        .map(|(e, _)| e.name.clone())
        .collect();
    assert_eq!(names, ["F0", "F1"]);
    // 0.2 is 51 of 255 and 0.6 is 153; `SkyFactor` 0.6 in green.
    for (first, second, texel) in [
        (0.2, 0.6, [51, 153, 153, 255]),
        (0.6, 0.2, [153, 153, 51, 255]),
    ] {
        core.extra.values()[0].1 = first;
        core.extra.values()[1].1 = second;
        let pixels = draw(&device, &queue, &mut core, &PROBE);
        for drawn in pixels.chunks(4) {
            assert_eq!(drawn, texel);
        }
    }
}

/// Polytone's `PolyGlobals` from the built-in `polytone_globals.glsl` beside `LightmapInfo` and
/// an expression uniform, each in its own block: the player's block height (64) in red,
/// `SkyFactor` (0.6, 153) in green and `F0` (0.2, 51) in blue, exact bytes; zeros when no
/// globals are given.
#[test]
#[ignore = "daily"]
fn a_lightmap_shader_reads_poly_globals() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
        layout(std140) uniform LightmapInfo {\n    float SkyFactor;\n} info;\n\
        layout(std140) uniform F0 {\n    float F0v;\n};\n\
        layout(location = 0) in vec2 texCoord;\nlayout(location = 0) out vec4 fragColor;\n\
        void main() {\n    fragColor = vec4(float(PolyPlayerBlockPos.y) / 255.0, info.SkyFactor, \
        F0v, 1.0);\n}\n";
    let include = |id: &str| {
        sunrise_polytone::globals::is_include(id)
            .then(|| sunrise_polytone::globals::GLSL.to_owned())
    };
    let program =
        crate::core_shader::translate_core_with(SCREENQUAD_VSH, fsh, &[], &include, &GLUE, &|_| {
            true
        })
        .expect("the shaders translate");
    let pack = sunrise_assets::resource_source::ResourceSource::indexed(
        [(
            "assets/minecraft/polytone/shader_modifiers/core/lightmap.json".to_owned(),
            sunrise_assets::resource_source::Resource::bytes(b"{\"F0\": \"0.2\"}".to_vec()),
        )]
        .into_iter()
        .collect(),
    );
    let modifiers = crate::polytone_post::ShaderModifiers::read(&pack, Default::default());
    let expressions = modifiers
        .resolve(
            (SHADERS[0], SHADERS[1]),
            &program,
            &[INFO_BLOCK, sunrise_polytone::globals::BLOCK],
        )
        .expect("the uniforms resolve");
    let mut core = CoreLightmap::new(&device, &program, &expressions).expect("the pipeline");
    core.set_readback(&device, true);
    assert!(core.extra.reads_globals());
    core.extra.evaluate(&Default::default());
    let globals = sunrise_polytone::globals::Globals {
        player: [3.5, 64.2, 7.0],
        ..Default::default()
    }
    .std140();
    for (given, texel) in [
        (Some(&globals), [64, 153, 51, 255]),
        (None, [0, 153, 51, 255]),
    ] {
        let pixels = draw_with(&device, &queue, &mut core, &PROBE, given);
        for drawn in pixels.chunks(4) {
            assert_eq!(drawn, texel);
        }
    }
}

/// A device binds at most `max_uniform_buffers_per_shader_stage` blocks to each stage (12 by
/// default): each block counts only for the stages that read it. Seven expression uniforms in
/// each stage are drawn (seven and eight with `LightmapInfo`); twelve in the fragment stage
/// beside `LightmapInfo` are refused when the pipeline is built, so the texture stays native.
#[test]
#[ignore = "daily"]
fn a_lightmap_shaders_blocks_count_for_the_stages_that_read_them() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, _)) = device() else {
        return;
    };
    assert_eq!(device.limits().max_uniform_buffers_per_shader_stage, 12);
    let (program, expressions) = expression_lightmap(7, 7);
    assert!(CoreLightmap::new(&device, &program, &expressions).is_some());
    let (program, expressions) = expression_lightmap(0, 11);
    assert!(CoreLightmap::new(&device, &program, &expressions).is_some());
    let (program, expressions) = expression_lightmap(0, 12);
    assert!(CoreLightmap::new(&device, &program, &expressions).is_none());
}

/// What F3's lightmap chart paints across the chart closing and opening again and fullbright
/// turning on and off (the renderer's `lightmap_sample`): only pixels of the texture drawn now,
/// never an old sample. No adapter needed: the chart's state alone.
#[test]
fn the_lightmap_chart_never_shows_a_stale_sample() {
    let mut chart = LightmapChart::new();
    let pack = Drawn::now(false, true);
    let (native, white) = (Drawn::now(false, false), Drawn::now(true, true));
    assert_eq!(
        (pack, native, white),
        (Drawn::Pack, Drawn::Native, Drawn::Native)
    );
    assert_eq!(chart.shown(pack), None, "nothing read yet");
    // The chart open: a pack-drawn copy lands.
    chart.pixels = [9; 16 * 16 * 4];
    chart.pack_copied();
    assert_eq!(chart.shown(pack), Some(&[9; 16 * 16 * 4]));
    // Closed, then opened again: nothing until a fresh copy.
    chart.forget_pack();
    assert_eq!(chart.shown(pack), None);
    chart.pixels = [10; 16 * 16 * 4];
    chart.pack_copied();
    assert_eq!(chart.shown(pack), Some(&[10; 16 * 16 * 4]));
    // Fullbright on: the pack's copy is not the white texture drawn now, until the native path
    // writes it.
    assert_eq!(chart.shown(white), None);
    chart.native([255; 16 * 16 * 4]);
    assert_eq!(chart.shown(white), Some(&[255; 16 * 16 * 4]));
    // Fullbright off with the pack's shader: the white pixels are not its texture.
    assert_eq!(chart.shown(pack), None);
    // Closing the chart leaves a native sample alone: it is current.
    chart.forget_pack();
    assert_eq!(chart.shown(native), Some(&[255; 16 * 16 * 4]));
}

/// Fullbright stops a pack's shader with copies mapped or still in flight: once it draws again,
/// those copies are dropped as they land, and the chart reads only a copy made after.
#[test]
#[ignore = "daily"]
fn copies_from_before_fullbright_are_never_read() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((device, queue)) = device() else {
        return;
    };
    let program = crate::core_shader::translate_core_with(
        SCREENQUAD_VSH,
        PROBE_FSH,
        &[],
        &|_| None,
        &GLUE,
        &|b| b == INFO_BLOCK,
    )
    .expect("the shaders translate");
    let mut core = CoreLightmap::new(&device, &program, &[]).expect("the lightmap pipeline");
    core.set_readback(&device, true);
    let frame = |core: &mut CoreLightmap, info: &LightmapInfo, poll: bool| {
        let (buffer, data) = core.uniforms([0; 48], info);
        queue.write_buffer(buffer, 0, data);
        let mut encoder = device.create_command_encoder(&Default::default());
        let target = target(&device);
        core.draw(
            &mut encoder,
            &target,
            &target.create_view(&Default::default()),
        );
        queue.submit([encoder.finish()]);
        (core.submitted().expect("a readback"))();
        if poll {
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        }
    };
    // One copy mapped, one in flight, when fullbright starts.
    frame(&mut core, &PROBE, true);
    frame(&mut core, &PROBE, false);
    core.forget_copies();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mut pixels = [7u8; 16 * 16 * 4];
    assert!(!core.collect(&mut pixels), "both dropped");
    assert_eq!(pixels, [7; 16 * 16 * 4]);
    // The pack draws again: its next copy is read (STATES[0]'s SkyFactor, 0, in blue).
    frame(&mut core, &STATES[0], true);
    assert!(core.collect(&mut pixels));
    assert_eq!(pixels[2], 0);
}

/// The `notGamma` scale multiplies left to right as `lightmap.fsh` writes it. At 0.0015 Java's
/// float `1.0f - x * x * x * x` (jshell) gives bits 1002711808; squaring twice gives 1002711680.
#[test]
fn the_not_gamma_scale_multiplies_in_the_shaders_order() {
    assert_eq!(max_scaled(0.0015).to_bits(), 1_002_711_808);
}
