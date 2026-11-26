//! A composite-style program whose fragment stage writes no buffer still does what a later
//! program can see (#675): its forced flips (`flip.<program>.<buffer>=true`, spec 6.1.6) at its
//! place in the chain, and its compute stages (2.6). Original constant-colour programs, no pack
//! files.
use super::destination_tests::Fixture;
use std::io::{Cursor, Write};

const VERTEX: &str = "#version 120\nvarying vec2 uv;\nvoid main() { uv=gl_MultiTexCoord0.xy; gl_Position=ftransform(); }\n";

/// A fragment shader that writes nothing.
const NOTHING: &str = "#version 120\nvoid main() {}\n";

fn pack(files: &[(&str, &str)]) -> sunrise_shaders::Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = sunrise_shaders::Pack::from_zip_bytes(
        "flip-only-program",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = sunrise_shaders::Macros::standard();
    let (properties, _) = sunrise_shaders::read_properties(&pack, &mut macros);
    sunrise_shaders::load_post(&pack, "", &properties, &macros, &Default::default())
}

/// Two frames of `files` on a 4×4 screen (every pixel of a frame must be the same), and the
/// pack's notes. `None` without a GPU.
fn frames(files: &[(&str, &str)]) -> Option<(Vec<[u8; 4]>, Vec<String>)> {
    let gpu = Fixture::new()?;
    let mut post = gpu.try_build(pack(files)).expect("the pack runs");
    let frames = (0..2)
        .map(|_| {
            let pixels = gpu.render(&mut post, [4, 4]);
            assert!(pixels.iter().all(|p| *p == pixels[0]), "{pixels:?}");
            pixels[0]
        })
        .collect();
    Some((frames, post.notes))
}

/// Nothing of the pack was left out.
fn assert_all_kept(notes: &[String]) {
    assert!(
        notes.iter().all(|n| n.starts_with("shadow map ")),
        "{notes:?}"
    );
}

/// `deferred` draws 0.25 into colortex2's "alt" side and flips it; `composite` writes nothing
/// but forces colortex2 to flip back, so `final` reads its cleared side (0). Without that flip
/// `final` would read 0.25 (64). `composite` is the first program of its group, so no earlier
/// pass of the group could carry its flip.
#[test]
#[ignore = "daily"]
fn a_program_that_only_flips_flips_its_buffers() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((frames, notes)) = frames(&[
        ("deferred.vsh", VERTEX),
        (
            "deferred.fsh",
            "#version 120\n/* RENDERTARGETS:2 */\nvoid main() { gl_FragData[0]=vec4(0.25,0.25,0.25,1); }\n",
        ),
        ("composite.vsh", VERTEX),
        ("composite.fsh", NOTHING),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex2,uv).r,0.6,0,1); }\n",
        ),
        ("shaders.properties", "flip.composite.colortex2=true\n"),
    ]) else {
        return;
    };
    assert_eq!(frames, [[0, 153, 0, 255]; 2]);
    assert_all_kept(&notes);
}

/// The same in the shadow pass: `shadowcomp` draws blue into shadowcolor0's "alt" side and
/// flips it, `shadowcomp1` writes nothing but flips it back, so `final` reads the side the
/// shadow pass cleared (white). Without that flip it would read blue. `flip.*` names buffers as
/// colortex in every group (sunrise-shaders' `buffer_index`), so `colortex0` is shadowcolor0 here.
#[test]
#[ignore = "daily"]
fn a_shadowcomp_program_that_only_flips_flips_its_buffers() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((frames, notes)) = frames(&[
        ("shadowcomp.vsh", VERTEX),
        (
            "shadowcomp.fsh",
            "#version 120\n/* RENDERTARGETS: 0 */\nvoid main() { gl_FragData[0] = vec4(0.0, 0.0, 1.0, 1.0); }\n",
        ),
        ("shadowcomp1.vsh", VERTEX),
        ("shadowcomp1.fsh", NOTHING),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 330 compatibility\nconst int shadowMapResolution = 4;\n\
             uniform sampler2D shadowcolor0;\n\
             void main() { gl_FragColor = texelFetch(shadowcolor0, ivec2(gl_FragCoord.xy), 0); }\n",
        ),
        ("shaders.properties", "flip.shadowcomp1.colortex0=true\n"),
    ]) else {
        return;
    };
    assert_eq!(frames, [[255; 4]; 2]);
    assert_all_kept(&notes);
}

/// Such programs' flips of one buffer add up as flips do: `deferred` draws 0.25 into colortex2 and
/// colortex3; `composite`, `composite1` and `composite2` write nothing and flip colortex2 three
/// times (back to its cleared side: red 0), `composite` and `composite1` flip colortex3 twice
/// (cancelling: green 64). Dropped programs would leave both at 64, and flips that only set a
/// side (not toggle it) would leave both at 0.
#[test]
#[ignore = "daily"]
fn flips_of_programs_that_only_flip_toggle_their_buffers() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((frames, notes)) = frames(&[
        ("deferred.vsh", VERTEX),
        (
            "deferred.fsh",
            "#version 120\n/* RENDERTARGETS:2,3 */\n\
             void main() { gl_FragData[0]=vec4(0.25,0.25,0.25,1); gl_FragData[1]=vec4(0.25,0.25,0.25,1); }\n",
        ),
        ("composite.vsh", VERTEX),
        ("composite.fsh", NOTHING),
        ("composite1.vsh", VERTEX),
        ("composite1.fsh", NOTHING),
        ("composite2.vsh", VERTEX),
        ("composite2.fsh", NOTHING),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex2;\nuniform sampler2D colortex3;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex2,uv).r,texture2D(colortex3,uv).r,0,1); }\n",
        ),
        (
            "shaders.properties",
            "flip.composite.colortex2=true\nflip.composite1.colortex2=true\n\
             flip.composite2.colortex2=true\nflip.composite.colortex3=true\n\
             flip.composite1.colortex3=true\n",
        ),
    ]) else {
        return;
    };
    assert_eq!(frames, [[0, 64, 0, 255]; 2]);
    assert_all_kept(&notes);
}

/// A `composite` whose fragment stage writes nothing still runs its compute stage (spec 2.6),
/// with or without a forced flip: `composite.csh` stores (0.2, 0.4, 0.6) into `imgA`, cleared
/// every frame, and `final` shows it. Left out, `final` would show the cleared image (0).
#[test]
#[ignore = "daily"]
fn a_program_that_writes_no_buffer_runs_its_compute_stages() {
    let _gpu = crate::gfx::test_gpu::lock();
    for flip in ["", "flip.composite.colortex2=true\n"] {
        let properties =
            format!("image.imgA = texA RGBA RGBA8 UNSIGNED_BYTE true false 4 4\n{flip}");
        let Some((frames, notes)) = frames(&[
            (
                "composite.csh",
                "#version 430\nlayout(local_size_x = 1, local_size_y = 1) in;\n\
                 const ivec3 workGroups = ivec3(4, 4, 1);\n\
                 layout(rgba8) writeonly uniform image2D imgA;\n\
                 void main() { imageStore(imgA, ivec2(gl_GlobalInvocationID.xy), vec4(0.2, 0.4, 0.6, 1.0)); }\n",
            ),
            ("composite.vsh", VERTEX),
            ("composite.fsh", NOTHING),
            ("final.vsh", VERTEX),
            (
                "final.fsh",
                "#version 130\nuniform sampler2D texA;\n\
                 void main() { gl_FragColor = texelFetch(texA, ivec2(gl_FragCoord.xy), 0); }\n",
            ),
            ("shaders.properties", &properties),
        ]) else {
            return;
        };
        assert_eq!(frames, [[51, 102, 153, 255]; 2], "{flip:?}");
        assert_all_kept(&notes);
    }
}

/// A fragment stage that writes no buffer but binds a storage buffer is left out with a note
/// naming its program; its forced flip still happens (`final` reads colortex2's cleared side).
#[test]
#[ignore = "daily"]
fn a_program_that_only_flips_notes_its_storage_buffer_fragment_stage() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((frames, notes)) = frames(&[
        ("deferred.vsh", VERTEX),
        (
            "deferred.fsh",
            "#version 120\n/* RENDERTARGETS:2 */\nvoid main() { gl_FragData[0]=vec4(0.25,0.25,0.25,1); }\n",
        ),
        ("composite.vsh", VERTEX),
        (
            "composite.fsh",
            "#version 430 compatibility\nlayout(std430, binding = 0) buffer Counter { uint n; } counter;\n\
             void main() { atomicAdd(counter.n, 1u); }\n",
        ),
        ("final.vsh", VERTEX),
        (
            "final.fsh",
            "#version 120\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() { gl_FragColor=vec4(texture2D(colortex2,uv).r,0.6,0,1); }\n",
        ),
        (
            "shaders.properties",
            "bufferObject.0 = 16\nflip.composite.colortex2=true\n",
        ),
    ]) else {
        return;
    };
    assert_eq!(frames, [[0, 153, 0, 255]; 2]);
    let left_out: Vec<_> = (notes.iter())
        .filter(|n| !n.starts_with("shadow map "))
        .collect();
    assert!(
        left_out.len() == 1 && left_out[0].starts_with("composite:"),
        "{notes:?}"
    );
}

/// The same in the shadow pass: `shadowcomp` writes nothing but its compute stage stores
/// (0.2, 0.4, 0.6) into `imgA`, with or without a forced flip, and `final` shows it. `final`
/// samples shadowcolor0 too, so the shadow pass runs.
#[test]
#[ignore = "daily"]
fn a_shadowcomp_program_that_writes_no_buffer_runs_its_compute_stages() {
    let _gpu = crate::gfx::test_gpu::lock();
    for flip in ["", "flip.shadowcomp.colortex0=true\n"] {
        let properties =
            format!("image.imgA = texA RGBA RGBA8 UNSIGNED_BYTE true false 4 4\n{flip}");
        let Some((frames, notes)) = frames(&[
            (
                "shadowcomp.csh",
                "#version 430\nlayout(local_size_x = 1, local_size_y = 1) in;\n\
                 const ivec3 workGroups = ivec3(4, 4, 1);\n\
                 layout(rgba8) writeonly uniform image2D imgA;\n\
                 void main() { imageStore(imgA, ivec2(gl_GlobalInvocationID.xy), vec4(0.2, 0.4, 0.6, 1.0)); }\n",
            ),
            ("shadowcomp.vsh", VERTEX),
            ("shadowcomp.fsh", NOTHING),
            ("final.vsh", VERTEX),
            (
                "final.fsh",
                "#version 330 compatibility\nconst int shadowMapResolution = 4;\n\
                 uniform sampler2D shadowcolor0;\nuniform sampler2D texA;\n\
                 void main() {\n    ivec2 p = ivec2(gl_FragCoord.xy);\n\
                     gl_FragColor = texelFetch(texA, p, 0) + 0.0 * texelFetch(shadowcolor0, p, 0);\n}\n",
            ),
            ("shaders.properties", &properties),
        ]) else {
            return;
        };
        assert_eq!(frames, [[51, 102, 153, 255]; 2], "{flip:?}");
        assert_all_kept(&notes);
    }
}
