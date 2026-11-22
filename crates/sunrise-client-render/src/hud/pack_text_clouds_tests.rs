//! A pack's `core/clouds` as the atlas loads it (`load_clouds`).
use super::*;
use sunrise_assets::resource_source::Resource;

const VSH: &str = "assets/minecraft/shaders/core/clouds.vsh";
const FSH: &str = "assets/minecraft/shaders/core/clouds.fsh";

/// `clouds.vsh` in shape: `CloudInfo` and the `CloudFaces` texel buffer.
const CLOUDS_VSH: &str = "#version 330\n\
    layout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    vec3 CloudOffset;\n    vec3 CellSize;\n};\n\
    uniform isamplerBuffer CloudFaces;\nlayout(location = 1) out vec4 vertexColor;\n\
    void main() {\n    int x = texelFetch(CloudFaces, gl_VertexIndex / 4 * 3).r;\n\
        gl_Position = vec4(vec3(x) * CellSize + CloudOffset, 1.0);\n    vertexColor = CloudColor;\n}\n";
const CLOUDS_FSH: &str = "#version 330\nlayout(location = 1) in vec4 vertexColor;\n\
    layout(location = 0) out vec4 fragColor;\nvoid main() {\n    fragColor = vertexColor;\n}\n";

fn source(files: &[(&str, &str)]) -> ResourceSource {
    ResourceSource::indexed(
        files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), Resource::bytes(t.as_bytes().to_vec())))
            .collect(),
    )
}

/// The game's two stages, and a pack's fragment stage over them: the pack's clouds load with
/// `CloudInfo` and `CloudFaces` given; with only the game's stages there is no pack shader; a
/// pack stage reading another block fails the load, named by the first pipeline it can't make.
#[test]
fn a_pack_replacing_a_clouds_stage_is_loaded_with_the_faces_and_cloud_info() {
    let defaults = source(&[(VSH, CLOUDS_VSH), (FSH, CLOUDS_FSH)]);
    let pink = CLOUDS_FSH.replace(
        "vertexColor;\n}",
        "vertexColor * vec4(1.0, 0.6, 0.8, 1.0);\n}",
    );
    let root = source(&[(VSH, CLOUDS_VSH), (FSH, &pink)]);
    let programs = loaded(&root, &defaults);
    assert_eq!(programs.classic.other, ["CloudInfo", "CloudFaces"]);
    for program in programs.oit.iter() {
        assert_eq!(program.other, ["CloudInfo", "CloudFaces"]);
    }
    assert!(
        matches!(load_clouds(&defaults, &defaults), Ok(None)),
        "the game's own"
    );
    let other = pink.replace(
        "void main()",
        "layout(std140) uniform Other {\n    vec4 Tint;\n};\nvoid main()",
    );
    let other = other.replace("vec4(1.0, 0.6, 0.8, 1.0)", "Tint");
    let root = source(&[(VSH, CLOUDS_VSH), (FSH, &other)]);
    let refused = load_clouds(&root, &defaults).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/clouds");
    assert!(refused.why.contains("Other"), "{refused}");
}

/// The packs' clouds at `root`, which must load.
fn loaded(root: &ResourceSource, defaults: &ResourceSource) -> Arc<crate::clouds::pack::Programs> {
    load_clouds(root, defaults)
        .expect("the required programs")
        .expect("the pack's clouds")
}

/// The pack's fragment stage with `declarations` before its `main` and `body` in it.
fn fragment(declarations: &str, body: &str) -> String {
    format!(
        "#version 330\nlayout(location = 1) in vec4 vertexColor;\n{declarations}\n\
         void main() {{\n{body}\n}}\n"
    )
}

/// The packs' clouds as the atlas loads them, the game's stages under a pack's `fsh`.
fn clouds(fsh: &str) -> Result<Option<Arc<crate::clouds::pack::Programs>>, RequiredShaderRefused> {
    let defaults = source(&[(VSH, CLOUDS_VSH), (FSH, CLOUDS_FSH)]);
    let root = source(&[(VSH, CLOUDS_VSH), (FSH, fsh)]);
    load_clouds(&root, &defaults)
}

/// The shader is translated once outside Improved Transparency and once for each of its stages
/// with that stage's defines (`OIT_SNIPPET`'s and the stage's): here the transmittance stage
/// alone writes its two coefficient targets (`OIT_COEFF_ATTACHMENT_COUNT` is 2), and each
/// stage's `#ifdef` picks its own colour.
#[test]
fn a_clouds_shader_is_translated_for_each_improved_transparency_stage() {
    let fsh = fragment(
        "#ifdef OIT_TRANSMITTANCE\nlayout(location = 0) out vec4 coeff[OIT_COEFF_ATTACHMENT_COUNT];\n\
         #else\nlayout(location = 0) out vec4 fragColor;\n#endif",
        "#if defined(OIT_TRANSMITTANCE) && defined(OIT_ALPHA_ONLY) && defined(OIT) \
         && OIT_COEFF_COUNT == 8 && OIT_WAVELET_RANK == 2 && defined(RENDERPEARL_DEPTH_IS_ZERO_TO_ONE)\n\
         coeff[0] = vertexColor;\n    coeff[1] = vertexColor;\n\
         #elif defined(OIT_DEPTH_BOUNDS) && defined(OIT_ALPHA_ONLY)\n    fragColor = vec4(1.0);\n\
         #elif defined(OIT_ACCUMULATE) && !defined(OIT_ALPHA_ONLY)\n    fragColor = vec4(2.0);\n\
         #elif !defined(OIT)\n    fragColor = vertexColor;\n#endif",
    );
    let programs = clouds(&fsh).unwrap().expect("the pack's clouds");
    assert_eq!(programs.classic.fragment_writes(), [0]);
    let oit = &programs.oit;
    let writes: Vec<Vec<u32>> = oit.iter().map(|p| p.fragment_writes()).collect();
    assert_eq!(writes, [vec![0], vec![0, 1], vec![0]]);
}

/// Each stage is given only the samplers its pass binds (`OitRenderPassProvider`): the depth
/// bounds to the transmittance and accumulate stages, the coefficients to the accumulate
/// stage. A stage reading one its pass doesn't bind refuses the Improved Transparency
/// programs, leaving the native clouds there, while the classic one still loads.
#[test]
fn each_improved_transparency_stage_is_given_its_passs_samplers() {
    let samplers = "uniform sampler2D DepthBoundsSampler;\nuniform sampler2D Coeff0;\n\
        uniform sampler2D Coeff1;\nuniform sampler2D Coeff2;\nlayout(location = 0) out vec4 fragColor;";
    let read = |stage: &str, sampler: &str| {
        fragment(
            samplers,
            &format!(
                "fragColor = vertexColor;\n#ifdef {stage}\n\
                 fragColor += texelFetch({sampler}, ivec2(gl_FragCoord.xy), 0);\n#endif"
            ),
        )
    };
    let given = clouds(&fragment(
        samplers,
        "fragColor = vertexColor;\n#if defined(OIT_TRANSMITTANCE) || defined(OIT_ACCUMULATE)\n\
         fragColor += texelFetch(DepthBoundsSampler, ivec2(0), 0);\n#endif\n\
         #ifdef OIT_ACCUMULATE\n\
         fragColor += texelFetch(Coeff0, ivec2(0), 0) + texelFetch(Coeff1, ivec2(0), 0);\n#endif",
    ))
    .expect("every sampler is its stage's")
    .expect("the pack's clouds");
    let oit = &given.oit;
    let at = |name: &str| {
        let index = oit[0].other.iter().position(|n| n == name).unwrap();
        crate::core_shader::other_binding(index)
    };
    let fragment_reads = |program: &CoreProgram, name: &str| {
        program.stages_reading(at(name)) == wgpu::ShaderStages::FRAGMENT
    };
    assert!(!fragment_reads(&oit[0], "DepthBoundsSampler"));
    assert!(fragment_reads(&oit[1], "DepthBoundsSampler"));
    assert!(!fragment_reads(&oit[1], "Coeff0"));
    assert!(fragment_reads(&oit[2], "DepthBoundsSampler"));
    assert!(fragment_reads(&oit[2], "Coeff0") && fragment_reads(&oit[2], "Coeff1"));
    for (stage, sampler, pipeline) in [
        (
            "OIT_DEPTH_BOUNDS",
            "DepthBoundsSampler",
            "minecraft:pipeline/oit_depth_bounds_clouds",
        ),
        (
            "OIT_TRANSMITTANCE",
            "Coeff0",
            "minecraft:pipeline/oit_transmittance_clouds",
        ),
        (
            "OIT_ACCUMULATE",
            "Coeff2",
            "minecraft:pipeline/oit_accumulate_clouds",
        ),
    ] {
        let refused = clouds(&read(stage, sampler)).unwrap_err();
        assert_eq!(refused.pipeline, pipeline, "{stage} {sampler}");
        assert!(refused.why.contains(sampler), "{refused}");
    }
}

/// Vanilla requires every clouds program, Improved Transparency on or off: a shader only its
/// classic variant can't use fails the load at `CLOUDS`, and one only a stage can't use at that
/// stage's pipeline.
#[test]
fn a_clouds_program_any_pipeline_refuses_fails_the_load() {
    let fsh = |branch: &str| {
        fragment(
            "layout(std140) uniform Other {\n    vec4 Tint;\n};\nlayout(location = 0) out vec4 fragColor;",
            &format!("fragColor = vertexColor;\n{branch}\n    fragColor *= Tint;\n#endif"),
        )
    };
    let refused = clouds(&fsh("#ifndef OIT")).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/clouds");
    let refused = clouds(&fsh("#ifdef OIT_TRANSMITTANCE")).unwrap_err();
    assert_eq!(
        refused.pipeline,
        "minecraft:pipeline/oit_transmittance_clouds"
    );
    let refused = clouds(&fsh("#ifdef OIT_ACCUMULATE")).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/oit_accumulate_clouds");
}

/// What the device would refuse when it makes a pipeline fails the load too, as vanilla's
/// compile failure does: reading `Sampler0`, which no clouds pass binds; writing the colour
/// target with integers.
#[test]
fn a_clouds_program_its_pipeline_cant_bind_or_target_fails_the_load() {
    let sampler = fragment(
        "uniform sampler2D Sampler0;\nlayout(location = 0) out vec4 fragColor;",
        "fragColor = vertexColor * texture(Sampler0, vec2(0.5));",
    );
    let refused = clouds(&sampler).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/clouds");
    assert!(refused.why.contains("Sampler0"), "{refused}");
    let integers = fragment(
        "layout(location = 0) out ivec4 fragColor;",
        "fragColor = ivec4(vertexColor * 255.0);",
    );
    let refused = clouds(&integers).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/clouds");
    assert!(refused.why.contains("colour target 0"), "{refused}");
}

/// A pack replacing only an include the clouds' stages use changes them: the stages are
/// compared with their includes resolved (here the include turns on a second output). The same
/// include as the game's changes nothing.
#[test]
fn a_pack_replacing_only_an_include_of_the_clouds_changes_them() {
    const INCLUDE: &str = "assets/minecraft/shaders/include/extra.glsl";
    let fsh = fragment(
        "#moj_import <minecraft:extra.glsl>\nlayout(location = 0) out vec4 fragColor;\n\
         #if EXTRA\nlayout(location = 1) out vec4 extraColor;\n#endif",
        "fragColor = vertexColor;\n#if EXTRA\n    extraColor = vertexColor;\n#endif",
    );
    let game = [
        (VSH, CLOUDS_VSH),
        (FSH, &fsh),
        (INCLUDE, "#define EXTRA 0\n"),
    ];
    let defaults = source(&game);
    assert!(matches!(load_clouds(&defaults, &defaults), Ok(None)));
    let root = source(&[
        (VSH, CLOUDS_VSH),
        (FSH, &fsh),
        (INCLUDE, "#define EXTRA 1\n"),
    ]);
    let programs = loaded(&root, &defaults);
    assert_eq!(programs.classic.fragment_writes(), [0, 1]);
}

/// An include the stage's defines drop is never read, as vanilla's shaderc preprocesses: one the
/// packs haven't refuses only the programs whose defines keep it.
#[test]
fn a_missing_include_refuses_only_the_programs_that_keep_it() {
    let behind = |branch: &str| {
        fragment(
            &format!(
                "{branch}\n#moj_import <minecraft:nowhere.glsl>\n#endif\n\
                 layout(location = 0) out vec4 fragColor;"
            ),
            "fragColor = vertexColor;",
        )
    };
    assert!(clouds(&behind("#ifdef NEVER_DEFINED")).unwrap().is_some());
    let refused = clouds(&behind("#ifdef OIT_TRANSMITTANCE")).unwrap_err();
    assert_eq!(
        refused.pipeline,
        "minecraft:pipeline/oit_transmittance_clouds"
    );
    assert!(
        refused
            .why
            .contains("minecraft:shaders/include/nowhere.glsl"),
        "{refused}"
    );
}

/// `GlslCompiler` defines `RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` for every program on a backend whose
/// depth runs from 0 to 1, `CLOUDS` among them: a shader reading a block no pipeline gives where
/// it is undefined still loads, outside Improved Transparency and under it.
#[test]
fn every_clouds_program_has_the_zero_to_one_depth_define() {
    let fsh = fragment(
        "layout(std140) uniform Other {\n    vec4 Tint;\n};\nlayout(location = 0) out vec4 fragColor;",
        "fragColor = vertexColor;\n#ifndef RENDERPEARL_DEPTH_IS_ZERO_TO_ONE\n    fragColor *= Tint;\n#endif",
    );
    assert!(clouds(&fsh).expect("every program").is_some());
}

#[test]
fn only_an_active_include_override_selects_or_refuses_clouds() {
    const INCLUDE: &str = "assets/minecraft/shaders/include/variant.glsl";
    for (condition, active) in [("0", false), ("defined(OIT_TRANSMITTANCE)", true)] {
        let fsh = format!("#if {condition}\n#include <variant.glsl>\n#endif\n{CLOUDS_FSH}");
        let defaults = source(&[(VSH, CLOUDS_VSH), (FSH, &fsh), (INCLUDE, "")]);
        let root = source(&[
            (VSH, CLOUDS_VSH),
            (FSH, &fsh),
            (INCLUDE, "uniform float Broken;"),
        ]);
        if active {
            let refused = load_clouds(&root, &defaults).unwrap_err();
            assert_eq!(
                refused.pipeline,
                "minecraft:pipeline/oit_transmittance_clouds"
            );
            let valid = source(&[
                (VSH, CLOUDS_VSH),
                (FSH, &fsh),
                (INCLUDE, "const float tint = 1.0;"),
            ]);
            assert!(load_clouds(&valid, &defaults).unwrap().is_some());
        } else {
            assert!(load_clouds(&root, &defaults).unwrap().is_none());
            let changed = format!("{CLOUDS_VSH}\n// pack override");
            let root = source(&[
                (VSH, &changed),
                (FSH, &fsh),
                (INCLUDE, "uniform float Broken;"),
            ]);
            assert!(load_clouds(&root, &defaults).unwrap().is_some());
        }
    }
}
