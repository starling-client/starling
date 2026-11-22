//! The stages' WGSL against the Rust definitions: the preludes of both lines compile, the
//! frame's WGSL layout is the uploaded struct's, 0.1's WGSL is what it always was, and
//! docs/api/shaders-reference.md and shaders-reference-0.2.md are up to date. Rewrite the
//! references with `SUNRISE_WRITE_DOCS=1 cargo test -p sunrise-shader-api --test it reference::`.
use std::path::Path;
use sunrise_shader_api::{Api, Frame, Pack, StageKind, compose, package::sha256_hex, reference};

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn validate(text: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(text)
        .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    let mut capabilities = naga::valid::Capabilities::default();
    if text.contains("enable wgpu_ray_query;") {
        capabilities |= naga::valid::Capabilities::RAY_QUERY;
    }
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}\n{text}", e.emit_to_string(text)));
    module
}

/// Vanilla's own entity colour (`entity_color` in the renderer's entity.wgsl), from the stage's
/// inputs: the reference prints the same function.
const VANILLA_ENTITY: &str = "\
fn entity(input: EntityInput) -> Surface {
    let base = input.albedo.rgb * input.tint.rgb * input.shade;
    // Vanilla lays the flash over the surface in its gamma (the 2.2 power), before the light.
    let gamma = vec3<f32>(1.0 / 2.2);
    let flashed = pow(mix(pow(base, gamma), pow(input.overlay.rgb, gamma), input.overlay.a), vec3<f32>(2.2));
    return Surface(vec4<f32>(flashed * input.lightmap, input.albedo.a * input.tint.a));
}
";

#[test]
fn the_reference_prints_vanilla_s_entity_function_as_it_is_checked() {
    let doc = reference::markdown(Api::V0_2);
    assert!(doc.contains(&format!("```wgsl\n{VANILLA_ENTITY}```")));
    assert!(!reference::markdown(Api::V0_1).contains("EntityInput"));
}

/// Vanilla's look, written as each stage.
fn vanilla(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Sky => "fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n",
        StageKind::Material => {
            "fn material(input: MaterialInput) -> Surface {\n    \
             return Surface(input.albedo * input.tint * vec4<f32>(input.lightmap, 1.0));\n}\n"
        }
        StageKind::Vertex => {
            "fn vertex(input: VertexInput) -> vec3<f32> { return input.position; }\n"
        }
        StageKind::Weather => {
            "fn weather(input: WeatherInput) -> vec4<f32> { return input.color; }\n"
        }
        StageKind::Post => "fn post(input: PostInput) -> vec4<f32> { return input.color; }\n",
        StageKind::Tonemap => {
            "fn tonemap(input: TonemapInput) -> vec3<f32> { return input.color; }\n"
        }
        StageKind::Entity => VANILLA_ENTITY,
    }
}

#[test]
fn every_stage_compiles_with_vanilla_s_function() {
    for api in Api::SERVED {
        for &kind in api.stages() {
            let text = format!(
                "{}{}{}",
                kind.prelude(api),
                vanilla(kind),
                kind.entries(api)
            );
            let module = validate(&text);
            assert!(
                module
                    .entry_points
                    .iter()
                    .all(|e| e.name.starts_with("sunrise_")),
                "{api} {kind:?}"
            );
        }
    }
    assert_eq!(Api::V0_1.stages().len(), 5);
    assert!(!Api::V0_1.stages().contains(&StageKind::Tonemap));
    assert!(!Api::V0_1.stages().contains(&StageKind::Entity));
}

/// The SHA-256 of 0.1's prelude and entries of each stage, as they were before 0.2: the line
/// packs run on today doesn't move.
const PINNED_0_1: [(StageKind, &str, &str); 5] = [
    (
        StageKind::Sky,
        "1ff46a47e01b634f73249b6743de7567ec936ac41c9cfc8e086b2c06b0d31399",
        "3a42f8c9cc3216ae7754335fbfcbd104351d5daf08646ef5e58f5490e6251dd7",
    ),
    (
        StageKind::Vertex,
        "2295b7f480edd68a043082a279e9e2aa4dc75201453e24af7e955ea16324f9f6",
        "e8c380a2e1a5a8fb8727b6a8a34b2c023b13b481a7937ec79990add67a171ffb",
    ),
    (
        StageKind::Material,
        "29c6f30be560c6099b8ef2dc8ce8abecad1395e4cdf1c5703571038dc0a35ecf",
        "77bd3366d1292a1166022b0861a679c5f1ae3fc20e69813c488270a27d8a6bb0",
    ),
    (
        StageKind::Weather,
        "c4345cb53837d6a0adf38ff5675b33aa6346da7764e3e2e69125b6c74b7c6728",
        "73851f10f2a08d73f3074804ebb2348714c0c7400fe784035417c8a6471d41b9",
    ),
    (
        StageKind::Post,
        "a3d09e9106d1946df159e7b6fb610546995ae4a3271b102e797f6091f5cec1c6",
        "3a8f48dc3d9518d4a998dc98acb3a657a4f83fe1dcb642493bfdbf6de5d2688e",
    ),
];

#[test]
fn the_0_1_wgsl_is_unchanged() {
    for (kind, prelude, entries) in PINNED_0_1 {
        let have = (
            sha256_hex(kind.prelude(Api::V0_1).as_bytes()),
            sha256_hex(kind.entries(Api::V0_1).as_bytes()),
        );
        assert_eq!(have, (prelude.to_owned(), entries.to_owned()), "{kind:?}");
    }
}

/// The `Frame` struct the line's sky prelude declares, as (name, offset) and its span.
fn frame_layout(api: Api) -> (Vec<(String, usize)>, usize) {
    let text = format!("{}{}", StageKind::Sky.prelude(api), vanilla(StageKind::Sky));
    let module = validate(&text);
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("Frame"))
        .expect("Frame in the prelude");
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("Frame is a struct")
    };
    let members = members
        .iter()
        .map(|m| (m.name.clone().unwrap(), m.offset as usize))
        .collect();
    (members, *span as usize)
}

#[test]
fn the_frame_s_wgsl_layout_is_the_uploaded_struct_s() {
    let rust: Vec<_> = Frame::WGSL
        .fields
        .iter()
        .zip(Frame::OFFSETS)
        .map(|(f, &o)| (f.name.to_owned(), o))
        .collect();
    let (wgsl, span) = frame_layout(Api::V0_2);
    assert_eq!(span, std::mem::size_of::<Frame>());
    assert_eq!(wgsl, rust);
    // 0.1 declares the start of the same buffer.
    let (wgsl, span) = frame_layout(Api::V0_1);
    assert_eq!(wgsl, rust[..Frame::FIELDS_0_1]);
    assert_eq!(span, rust[Frame::FIELDS_0_1].1);
}

#[test]
fn the_sample_pack_s_stages_compile() {
    let dir = root().join("crates/sunrise-shader-api/tests/samples/schema-1");
    let pack = Pack::read(&dir).unwrap();
    let m = &pack.manifest;
    let stages = [
        (StageKind::Sky, m.sky.iter().collect::<Vec<_>>()),
        (StageKind::Material, m.material.iter().collect()),
        (StageKind::Post, m.post.iter().collect()),
    ];
    for (kind, list) in stages {
        for (entry, stage) in list.into_iter().enumerate() {
            let composed = compose(&pack, kind, &stage.file, entry);
            validate(&composed.text);
            // Errors point into the pack's own file.
            let part = composed.parts.iter().find(|(f, _)| *f == stage.file);
            let part = part.unwrap().1.clone();
            let needle = format!("fn {}(", kind.function());
            let at = part.start + composed.text[part].find(&needle).unwrap();
            let source = pack.source(&stage.file).unwrap();
            let line = source.lines().position(|l| l.starts_with(&needle)).unwrap() + 1;
            assert_eq!(composed.locate(at), (stage.file.as_str(), line, 1));
        }
    }
}

#[test]
fn the_references_are_up_to_date() {
    for api in Api::SERVED {
        let doc = reference::path(api);
        let path = root().join(doc);
        let want = reference::markdown(api);
        if std::env::var_os("SUNRISE_WRITE_DOCS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            have == want,
            "{doc} is out of date: SUNRISE_WRITE_DOCS=1 cargo test -p sunrise-shader-api --test it reference::"
        );
    }
}
