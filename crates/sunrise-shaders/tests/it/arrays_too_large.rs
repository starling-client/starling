//! Types too large to lay out (#4134): naga refuses a type whose size passes `MAX_TYPE_SIZE`
//! (`i32::MAX` bytes). Its GLSL front end refuses such an array, struct or block where it is
//! declared, as a `TypeTooLarge` error at that line, in every way a size can be written; a shader
//! pack's program that declares one is skipped with a note naming its file and line.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use naga::front::glsl::{ErrorKind, Frontend, InterfaceLimits, Options};
use sunrise_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Post, load_post};

/// The errors of a `stage` whose line 2 on is `body`, parsed for a device with `limits`, each
/// with its line.
fn errors_of(
    stage: naga::ShaderStage,
    limits: InterfaceLimits,
    body: &str,
) -> Vec<(ErrorKind, usize)> {
    let source = format!("#version 450\n{body}\nvoid main() {{}}\n");
    let options = Options {
        interface_limits: limits,
        ..Options::from(stage)
    };
    match Frontend::default().parse(&options, &source) {
        Ok(_) => Vec::new(),
        Err(e) => e
            .errors
            .into_iter()
            .map(|e| {
                let start = e.meta.to_range().map_or(0, |r| r.start);
                (e.kind, source[..start].matches('\n').count() + 1)
            })
            .collect(),
    }
}

/// The errors of a fragment stage whose line 2 on is `body`, each with its line.
fn errors(body: &str) -> Vec<(ErrorKind, usize)> {
    errors_of(
        naga::ShaderStage::Fragment,
        InterfaceLimits::WGPU_DEFAULT,
        body,
    )
}

/// The stage interface limits of an AMD RADV device, as wgpu-hal 30 computes them from its Vulkan
/// limits (`vulkan/adapter.rs:1729-1735`): 128 output and input components give 128 / 4 - 1 = 31
/// inter-stage variables, and 32 vertex input attributes (Mesa 26.3 RADV, as `vulkaninfo` reports
/// them). D3D12 and Metal give 31 inter-stage variables too.
pub(crate) const RADV: InterfaceLimits = InterfaceLimits {
    inter_stage_variables: 31,
    vertex_attributes: 32,
};

/// `body` is refused as too large to lay out, at `line` (the stage's first line is the version).
#[track_caller]
fn too_large_at(body: &str, line: usize) {
    let errors = errors(body);
    assert!(
        matches!(errors[..], [(ErrorKind::TypeTooLarge, l)] if l == line),
        "{body}\n{errors:?}"
    );
}

#[test]
fn an_array_too_large_is_refused_however_its_size_is_written() {
    // 99999999999 does not fit 32 bits: the literal is refused (GLSL 4.60 §4.1.3), and the low
    // bits naga keeps, 1215752191, still give an array too large to lay out.
    let errors = errors("float a[99999999999];");
    assert!(
        matches!(&errors[..], [(ErrorKind::SemanticError(m), 2), (ErrorKind::TypeTooLarge, 2)]
            if m == "Integer literal does not fit in 32 bits"),
        "{errors:?}"
    );
    too_large_at("float a[0x7fffffff];", 2);
    too_large_at("float a[017777777777];", 2);
    too_large_at("float a[0x7fffffffu];", 2);
    too_large_at("const int N = 0x7fffffff;\nfloat a[N];", 3);
    too_large_at("#define N 0x7fffffff\nfloat a[N];", 3);
    // Each dimension fits; together they don't.
    too_large_at("float a[0x8000][0x8000];", 2);
    too_large_at("float[0x7fffffff] a;", 2);
    too_large_at("float a, b[0x7fffffff];", 2);
    too_large_at("void f() {\n    float a[0x7fffffff];\n}", 3);
    too_large_at("float f(float a[0x7fffffff]) { return 1.0; }", 2);
}

#[test]
fn a_struct_or_block_too_large_is_refused() {
    too_large_at("struct S {\n    float a[0x7fffffff];\n};", 3);
    // Members that fit alone and not together.
    too_large_at(
        "struct S {\n    float a[0x1fffffff];\n    float b[0x1fffffff];\n};",
        4,
    );
    // Members that fit, a size that passes the limit once rounded up to the struct's alignment.
    too_large_at("struct S {\n    vec4 v;\n    float a[0x1ffffffa];\n};", 4);
    too_large_at("struct S {\n    float a[0x10000000];\n};\nS s[4];", 5);
    // std140 rounds a float array's stride up to 16 bytes, past the limit where std430 fits.
    too_large_at(
        "layout(std140, binding = 0) uniform B {\n    float a[0x10000000];\n};",
        3,
    );
    too_large_at(
        "layout(std430, binding = 0) buffer B {\n    float a[0x10000000];\n    float b[0x10000000];\n};",
        4,
    );
    too_large_at(
        "layout(std140, binding = 0) uniform B {\n    vec4 v[0x100000];\n} b[0x1000];",
        2,
    );
}

/// A stage's location-bound interface variables are an entry-point argument or result member per
/// element: a stage whose variables of one direction take more locations than the device gives
/// that direction is refused, at the variable that passes the limit, before any argument is made,
/// whatever the sizes' form. Vertex outputs and fragment inputs take `inter_stage_variables`,
/// vertex inputs `vertex_attributes`, fragment outputs wgpu-hal's 8 color attachments. Under the
/// limits (17 to 31 locations on RADV) they parse as on main.
#[test]
fn an_interface_past_the_devices_limit_is_refused() {
    use naga::ShaderStage::{Fragment, Vertex};
    let refused_at = |stage, limits, body: &str, limit: u32, line: usize| {
        let errors = errors_of(stage, limits, body);
        assert!(
            matches!(errors[..], [(ErrorKind::InterfaceTooLarge { limit: l }, at)] if l == limit && at == line),
            "{body}\n{errors:?}"
        );
    };
    let parses = |stage, limits, body: &str| {
        let errors = errors_of(stage, limits, body);
        assert!(errors.is_empty(), "{body}\n{errors:?}");
    };
    // Fragment inputs: RADV's 31.
    parses(Fragment, RADV, "layout(location = 0) in vec4 v[31];");
    parses(Fragment, RADV, "layout(location = 0) in vec4 v[20];");
    refused_at(Fragment, RADV, "layout(location = 0) in vec4 v[32];", 31, 2);
    refused_at(
        Fragment,
        RADV,
        "layout(location = 0) in float v[4][8];",
        31,
        2,
    );
    refused_at(
        Fragment,
        RADV,
        "const int N = 0x1fffffff;\nlayout(location = 0) in float v[N];",
        31,
        3,
    );
    // Two variables that fit alone, not together: refused at the second.
    refused_at(
        Fragment,
        RADV,
        "layout(location = 0) in vec4 a[16];\nlayout(location = 16) in vec4 b[16];",
        31,
        3,
    );
    // Fragment outputs: 8 color attachments, counted apart from the inputs.
    parses(
        Fragment,
        RADV,
        "layout(location = 0) in vec4 i[31];\nlayout(location = 0) out vec4 o[8];",
    );
    refused_at(Fragment, RADV, "layout(location = 0) out vec4 c[9];", 8, 2);
    refused_at(
        Fragment,
        RADV,
        "layout(location = 0) out vec4 c[0x7ffffff];",
        8,
        2,
    );
    // Vertex inputs: 32 attributes; vertex outputs: 31 inter-stage variables.
    parses(
        Vertex,
        RADV,
        "layout(location = 0) in vec4 a[32];\nlayout(location = 0) out vec4 o[31];",
    );
    refused_at(Vertex, RADV, "layout(location = 0) in vec4 a[33];", 32, 2);
    refused_at(Vertex, RADV, "layout(location = 0) out vec4 o[32];", 31, 2);
    // The limits are the device's: wgpu's defaults (16) refuse what RADV takes.
    refused_at(
        Fragment,
        InterfaceLimits::WGPU_DEFAULT,
        "layout(location = 0) in vec4 v[20];",
        16,
        2,
    );
}

/// The issue's second shape: the oversized array was laid out, and the unwrap panicked, at the
/// next array specifier, here inside the uniform block.
#[test]
fn an_array_after_one_too_large_does_not_panic() {
    too_large_at(
        "float fz_sized[0x7fffffff];\nuniform FzBlock { mat4 fz_m; vec4 fz_v[4]; float fz_f, fz_g; };",
        2,
    );
}

/// The largest sizes that lay out still parse: the limit is naga's own, not a lower one.
#[test]
fn types_at_the_limit_still_parse() {
    // 0x1fffffff floats: 0x7ffffffc bytes.
    for body in [
        "float a[0x1fffffff];",
        "struct S {\n    float a[0x1fffffff];\n};",
        // 0x7ffffff floats at std140's 16-byte stride: 0x7ffffff0 bytes.
        "layout(std140, binding = 0) uniform B {\n    float a[0x7ffffff];\n};",
    ] {
        let errors = errors(body);
        assert!(errors.is_empty(), "{body}\n{errors:?}");
    }
}

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const VSH: &str = "#version 120\nvarying vec2 texcoord;\nvoid main() {\n\
                   gl_Position = ftransform();\ntexcoord = gl_MultiTexCoord0.xy;\n}\n";
const FSH: &str = "#version 120\nvarying vec2 texcoord;\n\
                   void main() {\ngl_FragData[0] = vec4(texcoord, 0.0, 1.0);\n}\n";

/// Loads a pack of `files` (under `shaders/`) beside a `composite` program that translates.
fn load(files: &[(&str, &str)]) -> Post {
    load_for(InterfaceLimits::WGPU_DEFAULT, files)
}

/// [`load`] for a device with `limits`.
fn load_for(limits: InterfaceLimits, files: &[(&str, &str)]) -> Post {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "sunrise-arrays-too-large-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let shaders = fixture.0.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    for (name, text) in [("composite.vsh", VSH), ("composite.fsh", FSH)]
        .iter()
        .chain(files)
    {
        std::fs::write(shaders.join(name), text).unwrap();
    }
    let pack = Pack::open(&fixture.0).unwrap();
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let (properties, notes) = sunrise_shaders::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    load_post(
        &pack,
        "",
        &properties,
        &macros,
        &GameInputs {
            terrain: Some(GbuffersInputs::default()),
            interface_limits: limits,
            ..Default::default()
        },
    )
}

/// `program` was skipped with a note at `file:line`, and `composite` still loaded.
#[track_caller]
fn skipped_at(post: &Post, program: &str, at: &str) {
    let notes = post.notes.join("\n");
    assert!(notes.contains(&format!("{program}: skipped")), "{notes}");
    assert!(
        post.notes.iter().any(|n| n.contains(&format!("{at}: "))),
        "{notes}"
    );
    assert!(
        post.programs
            .iter()
            .any(|p| p.name == "composite" && p.program.is_some()),
        "{notes}"
    );
}

/// The issue's first shape, as the fuzzer shrank it.
#[test]
fn a_shadow_program_with_an_array_too_large_is_skipped() {
    let post = load(&[
        ("shaders.properties", "shadow.enabled=true\n"),
        ("shadow.vsh", ""),
        ("shadow.fsh", "float fz_sized[99999999999];\n"),
    ]);
    assert!(post.shadow.is_none());
    skipped_at(&post, "shadow", "shadow.fsh:1");
}

/// The issue's second shape: an array too large, then a uniform block with arrays.
#[test]
fn a_deferred_program_with_an_array_too_large_is_skipped() {
    let post = load(&[
        ("deferred.vsh", VSH),
        (
            "deferred.fsh",
            "#version 120\nfloat fz_sized[0x7fffffff];\n\
             uniform FzBlock { mat4 fz_m; vec4 fz_v[4]; float fz_f, fz_g; };\n\
             void main() {\ngl_FragData[0] = vec4(fz_f);\n}\n",
        ),
    ]);
    assert!(!post.programs.iter().any(|p| p.name == "deferred"));
    skipped_at(&post, "deferred", "deferred.fsh:2");
}

/// A compute stage whose buffer block's offsets pass `u32` before a vector an atomic function
/// takes a component of: Sunrise's split of that vector leaves it whole, and naga refuses the
/// array.
#[test]
fn a_compute_stage_with_a_buffer_too_large_is_skipped() {
    let post = load(&[(
        "composite_a.csh",
        "#version 430\nlayout(local_size_x = 1) in;\n\
         layout(std430, binding = 0) buffer B {\n    float big[2000000000];\n    uvec2 v;\n} b;\n\
         void main() { atomicAdd(b.v.x, 1u); }\n",
    )]);
    skipped_at(&post, "composite_a", "composite_a.csh:4");
}

/// A varying from a pack program to the next, given in `vsh`'s and `fsh`'s line 2, for a device
/// with `limits`.
fn deferred_with_varying_for(limits: InterfaceLimits, declaration: &str, read: &str) -> Post {
    let vsh =
        format!("#version 120\n{declaration}\nvoid main() {{\ngl_Position = ftransform();\n}}\n");
    let fsh = format!(
        "#version 120\n{declaration}\nvoid main() {{\ngl_FragData[0] = vec4({read});\n}}\n"
    );
    load_for(limits, &[("deferred.vsh", &vsh), ("deferred.fsh", &fsh)])
}

/// [`deferred_with_varying_for`] an AMD RADV device.
fn deferred_with_varying(declaration: &str, read: &str) -> Post {
    deferred_with_varying_for(RADV, declaration, read)
}

/// A program's varyings take a location each (a split one, each part): past the device's
/// `max_inter_stage_shader_variables` (31 on RADV) a varying is refused at its line before a part
/// is made for each element, since the device could never build the pipeline. Under it, 17 to 31
/// locations still translate, as on main.
#[test]
fn a_varying_past_the_devices_limit_is_refused() {
    for (declaration, read) in [
        // A part for each of 99999999999 elements, which ran out of memory.
        ("varying float big[99999999999];", "big[0]"),
        // Sizes whose product overflows usize.
        (
            "varying float big[99999999999][99999999999][99999999999];",
            "big[0][0][0]",
        ),
        ("varying float big[32];", "big[0]"),
        ("varying float big[4][8];", "big[0][0]"),
        // Two varyings that fit alone, not together: the second is refused.
        (
            "varying float a0[16];\nvarying float a1[16];",
            "a0[0] + a1[0]",
        ),
        // Refused before its parts are made: making them would run out of memory.
        (
            "varying float a0[16];\nvarying float a1[99999999999];",
            "a0[0] + a1[0]",
        ),
        // A struct's members, four floats to a part: 32 parts.
        ("struct S { float a[128]; };\nvarying S big;", "big.a[0]"),
        (
            "struct S { float a[99999999999]; };\nvarying S big;",
            "big.a[0]",
        ),
        // Two struct varyings of 16 parts each: the second has only the room the first left.
        (
            "struct S { vec4 a[16]; };\nvarying S s0;\nvarying S s1;",
            "s0.a[0].x + s1.a[0].x",
        ),
    ] {
        let post = deferred_with_varying(declaration, read);
        assert!(
            !post.programs.iter().any(|p| p.name == "deferred"),
            "{declaration}"
        );
        // The vertex stage's declaration, on its last line.
        let line = 1 + declaration.lines().count();
        skipped_at(&post, "deferred", &format!("deferred.vsh:{line}"));
    }
    // A size the split doesn't read (an expression) reaches naga, which refuses it before making
    // an argument for each element. The varying is declared in Sunrise's header by then, so the
    // note names that.
    let post = deferred_with_varying("varying float big[0x4000 * 0x4000];", "big[0]");
    assert!(!post.programs.iter().any(|p| p.name == "deferred"));
    skipped_at(&post, "deferred", "<sunrise>:3");
    let translates = |post: &Post, declaration: &str| {
        assert!(
            post.programs
                .iter()
                .any(|p| p.name == "deferred" && p.program.is_some()),
            "{declaration}: {:?}",
            post.notes
        );
    };
    for (declaration, read) in [
        // At the limit: 31 locations.
        ("varying float edge[31];", "edge[30]"),
        ("struct S { vec4 a[31]; };\nvarying S edge;", "edge.a[30].x"),
        // 20 and 24 locations: past wgpu's default 16, within RADV's 31.
        ("varying vec4 mid[20];", "mid[19].x"),
        ("struct S { vec4 a[24]; };\nvarying S mid;", "mid.a[23].x"),
    ] {
        translates(&deferred_with_varying(declaration, read), declaration);
    }
    // The limit is the device's: with wgpu's default 16, 20 locations are refused.
    let post = deferred_with_varying_for(
        InterfaceLimits::WGPU_DEFAULT,
        "varying vec4 mid[20];",
        "mid[19].x",
    );
    skipped_at(&post, "deferred", "deferred.vsh:2");
}

/// A struct varying's member of a vector or matrix size GLSL doesn't have (`vec5`, `mat2x5`) can't
/// travel: the program is skipped with the struct's note, not a panic slicing its swizzle.
#[test]
fn a_struct_varying_of_a_vector_size_glsl_lacks_is_skipped() {
    for member in ["vec5 a;", "mat2x5 a;", "mat5 a;"] {
        let post = deferred_with_varying(&format!("struct S {{ {member} }};\nvarying S s;"), "1.0");
        assert!(
            !post.programs.iter().any(|p| p.name == "deferred"),
            "{member}: {:?}",
            post.notes
        );
        assert!(
            post.notes.iter().any(|n| n == "deferred: skipped"),
            "{member}: {:?}",
            post.notes
        );
    }
}

/// A shadow program writing as many varyings as the device takes leaves no room for the boats'
/// water patches (one varying more): translucent terrain's program with them is refused at
/// translation, and the one without them (`shadow_water_plain`) draws it instead, as when a GPU
/// refused it at pipeline creation. With room for the patches, both load.
#[test]
fn a_shadow_program_at_the_devices_limit_keeps_translucent_shadows() {
    let shadow = |outputs: usize| {
        let declared: String = (0..outputs)
            .map(|i| format!("varying vec4 v{i};\n"))
            .collect();
        let written: String = (0..outputs).map(|i| format!("v{i}=vec4({i}.0);")).collect();
        let read: String = (0..outputs).map(|i| format!("+v{i}")).collect();
        let vsh = format!(
            "#version 120\n{declared}void main() {{ gl_Position=ftransform(); {written} }}\n"
        );
        let fsh =
            format!("#version 120\n{declared}void main() {{ gl_FragData[0]=vec4(0.0){read}; }}\n");
        load_for(
            RADV,
            &[
                ("shaders.properties", "shadow.enabled=true\n"),
                ("shadow.vsh", &vsh),
                ("shadow.fsh", &fsh),
            ],
        )
    };
    let full = shadow(31);
    assert!(full.shadow.is_some(), "{:?}", full.notes);
    assert!(full.shadow_water.is_none(), "{:?}", full.notes);
    assert!(full.shadow_water_plain.is_some(), "{:?}", full.notes);
    assert!(
        !full.notes.iter().any(|n| n == "shadow: skipped"),
        "{:?}",
        full.notes
    );
    let room = shadow(30);
    assert!(room.shadow_water.is_some(), "{:?}", room.notes);
    assert!(room.shadow_water_plain.is_some(), "{:?}", room.notes);
}
