//! A pack's shadow program with a geometry stage, loaded with the game's entity inputs
//! (`pack_entities`), whose vertex stage names a member as a game input.
use crate::gfx::pack_entities;

/// A shadow program with a geometry stage, loaded with the game's entity inputs, whose
/// vertex stage has a buffer block with a field named `vaNormal` and also reads the game's
/// `vaNormal` undeclared: the field keeps its name, as `params.vaNormal` does, so the
/// program loads for entities and block entities, as one with the field named otherwise.
#[test]
fn a_shadow_geometry_stage_with_a_buffer_field_named_as_a_game_input_loads() {
    use std::io::Write;
    let program = |field: &str| {
        let vsh = format!(
            "#version 430\nlayout(std430, binding = 2) buffer Params {{ float w; vec3 {field}; }} params;\n\
             out vec3 vNormal;\nvoid main() {{\n    vNormal = params.{field} + vaNormal;\n    \
             gl_Position = gl_ModelViewProjectionMatrix * vec4(vaPosition, 1.0);\n}}\n"
        );
        let gsh = "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 3) out;\n\
                   in vec3 vNormal[];\nout vec3 gNormal;\nvoid main() {\n    for (int i = 0; i < 3; i++) {\n        \
                   gNormal = vNormal[i];\n        gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                   EndPrimitive();\n}\n";
        let fsh = "#version 430\nin vec3 gNormal;\nvoid main() { gl_FragData[0] = vec4(gNormal, 1.0); }\n";
        let files = [
            ("shaders/shaders.properties", "shadow.enabled=true\n"),
            ("shaders/shadow.vsh", vsh.as_str()),
            ("shaders/shadow.gsh", gsh),
            ("shaders/shadow.fsh", fsh),
        ];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, source) in files {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        let pack = sunrise_shaders::Pack::from_zip_bytes(
            "buffer-field",
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
        let mut macros = sunrise_shaders::Macros::standard();
        let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
        let inputs = sunrise_shaders::GameInputs {
            entities: Some(pack_entities::inputs()),
            entity_geometry: Some(pack_entities::geometry_inputs()),
            ..Default::default()
        };
        let post = sunrise_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        let loaded = |p: &Option<sunrise_shaders::PostProgram>| {
            p.as_ref().map(|p| (p.name.clone(), p.geometry_triangles))
        };
        (
            [loaded(&post.shadow_entities), loaded(&post.shadow_block)],
            post.notes,
        )
    };
    for field in ["vaNormal", "normal"] {
        let (loaded, notes) = program(field);
        let shadow = Some(("shadow".to_owned(), 1));
        assert_eq!(loaded, [shadow.clone(), shadow], "{field}: {notes:?}");
    }
}
