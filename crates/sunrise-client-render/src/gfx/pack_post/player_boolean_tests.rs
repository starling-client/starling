//! A compute program and a quad program that declare a player boolean only custom uniforms read
//! (`is_wet`, spec 3.4; decision D-2) and one the uniform table lists (`is_sneaking`, 5.2): the
//! built passes give the first no built-in value and the second its own. The programs are
//! original.
use super::destination_tests::Fixture;
use super::*;
use std::io::{Cursor, Write};
use sunrise_shaders::{Group, Post};

fn pack() -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, source: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    };
    let vertex = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";
    add(
        "composite.csh",
        "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1,1,1);\n\
         uniform bool is_wet;\nuniform bool is_sneaking;\n\
         layout(rgba8) writeonly uniform image2D colorimg0;\n\
         void main() {\n    imageStore(colorimg0, ivec2(0), vec4(is_wet ? 1.0 : 0.0, is_sneaking ? 1.0 : 0.0, 0.0, 1.0));\n}\n",
    );
    add("composite.vsh", vertex);
    add(
        "composite.fsh",
        "#version 330 compatibility\nuniform bool is_wet;\nuniform bool is_sneaking;\n\
         void main() { gl_FragColor = vec4(is_wet ? 1.0 : 0.0, is_sneaking ? 1.0 : 0.0, 0.0, 1.0); }\n",
    );
    add("final.vsh", vertex);
    add(
        "final.fsh",
        "#version 330 compatibility\nuniform sampler2D colortex0;\nin vec2 uv;\n\
         void main() { gl_FragColor = texelFetch(colortex0, ivec2(gl_FragCoord.xy), 0); }\n",
    );
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "original-player-booleans",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

/// Whether `pass` binds `is_wet` and `is_sneaking` to their built-in values.
fn binds(pass: &Pass) -> [bool; 2] {
    ["is_wet", "is_sneaking"].map(|name| {
        let builtin = sunrise_render_shaderpack::pack_uniforms::index(name).expect("a built-in");
        pass.uniforms.iter().any(|u| u.0 == Some(builtin))
    })
}

#[test]
#[ignore = "daily"]
fn compute_and_quad_programs_read_only_the_listed_player_booleans() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Fixture::new() else { return };
    let post = pack();
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let Ok(post) = gpu.try_build(post) else {
        panic!("the pack runs")
    };
    let quad = post
        .passes
        .iter()
        .find(|p| p.group == Group::Composite)
        .expect("the composite pass");
    assert_eq!(binds(quad), [false, true], "quad program");
    let [compute] = quad.compute.as_slice() else {
        panic!("one compute program: {:?}", post.notes)
    };
    assert_eq!(binds(compute), [false, true], "compute program");
}
