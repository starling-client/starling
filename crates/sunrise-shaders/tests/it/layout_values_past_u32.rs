//! Layout qualifier values a pack writes past what they can be (#4912). naga's GLSL front end gives
//! each element or member of an interface variable the location after the one before it: one
//! past `u32::MAX` is a `LocationTooLarge` error at the declaration, not an overflow. A shader
//! pack's fragment output whose `location` or `component` is past `u32` is refused at its file and
//! line, not wrapped to another location or component; one written in any form GLSL's integer
//! constant expressions take is read as its value.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use naga::front::glsl::{ErrorKind, Frontend, Options};
use sunrise_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Post, load_post};

/// The errors of a `stage` whose line 2 on is `body`, each with its line.
fn errors(stage: naga::ShaderStage, body: &str) -> Vec<(ErrorKind, usize)> {
    let source = format!("#version 450\n{body}\nvoid main() {{}}\n");
    match Frontend::default().parse(&Options::from(stage), &source) {
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

const STRUCT: &str = "struct S {\n    vec4 a;\n    vec4 b;\n};";

#[test]
fn an_interface_variable_whose_locations_pass_u32_is_refused() {
    use naga::ShaderStage::{Fragment, Vertex};
    for (stage, body, line) in [
        // The issue's shape: element 1 would take location 2^32.
        (
            Fragment,
            "layout(location = 4294967295u) out vec4 c[2];".to_owned(),
            2,
        ),
        (
            Fragment,
            "layout(location = 4294967294u) out vec4 c[3];".to_owned(),
            2,
        ),
        (
            Fragment,
            "layout(location = 4294967295u) in vec4 v[2];".to_owned(),
            2,
        ),
        // A struct's second member.
        (
            Vertex,
            format!("{STRUCT}\nlayout(location = 4294967295u) out S s;"),
            6,
        ),
        // An array of structs: element 1 at u32::MAX, its member `b` past it.
        (
            Vertex,
            format!("{STRUCT}\nlayout(location = 4294967294u) out S s[2];"),
            6,
        ),
    ] {
        let errors = errors(stage, &body);
        assert!(
            matches!(errors[..], [(ErrorKind::LocationTooLarge, l)] if l == line),
            "{body}\n{errors:?}"
        );
    }
    // The last locations that fit still parse.
    for (stage, body) in [
        (
            Fragment,
            "layout(location = 4294967295u) out vec4 c[1];".to_owned(),
        ),
        (
            Fragment,
            "layout(location = 4294967294u) out vec4 c[2];".to_owned(),
        ),
        (
            Vertex,
            format!("{STRUCT}\nlayout(location = 4294967294u) out S s;"),
        ),
    ] {
        let errors = errors(stage, &body);
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

/// Loads a pack of a `composite` program that translates and a `deferred` one whose fragment
/// stage declares `outputs` on line 2 and writes `body` in `main`.
fn deferred(outputs: &str, body: &str) -> Post {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "sunrise-layout-values-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let shaders = fixture.0.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    let fsh = format!("#version 430\n{outputs}\nvoid main() {{ {body} }}\n");
    for (name, text) in [
        ("composite.vsh", VSH),
        ("composite.fsh", FSH),
        ("deferred.vsh", VSH),
        ("deferred.fsh", &fsh),
    ] {
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
            ..Default::default()
        },
    )
}

/// `deferred` was skipped with a note at `at`, and `composite` still loaded.
#[track_caller]
fn skipped_at(post: &Post, at: &str) {
    let notes = post.notes.join("\n");
    assert!(
        !post.programs.iter().any(|p| p.name == "deferred"),
        "{notes}"
    );
    assert!(notes.contains("deferred: skipped"), "{notes}");
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

/// The fragment output locations of the loaded `deferred`.
#[track_caller]
fn outputs(post: &Post) -> Vec<u32> {
    let deferred = post.programs.iter().find(|p| p.name == "deferred");
    let program = deferred.and_then(|p| p.program.as_ref());
    program
        .unwrap_or_else(|| panic!("{:?}", post.notes))
        .interface
        .outputs
        .clone()
}

/// The issue's first shape and its kin: a component past `u32`, or one past the location's
/// four components however large, is refused at its line, not added to the output's width.
#[test]
fn a_fragment_output_at_a_component_past_four_is_refused() {
    for component in [
        "18446744073709551615",
        "4294967295",
        "4294967296",
        "4294967295u",
        "0xffffffff",
        "4",
        // Decimal 8, as before octal was read.
        "08",
    ] {
        let post = deferred(
            &format!("layout(location = 0, component = {component}) out float x;"),
            "x = 1.0;",
        );
        skipped_at(&post, "deferred.fsh:2");
        // The issue asks for the note an unsupported component already had.
        assert!(
            post.notes
                .iter()
                .any(|n| n.contains("`x`: a `float` output at component ")),
            "{:?}",
            post.notes
        );
    }
}

/// A location past `u32` is refused at its line, where it was read as the next free location
/// (0 here).
#[test]
fn a_fragment_output_at_a_location_past_u32_is_refused() {
    for location in [
        "4294967296",
        "18446744073709551615",
        "99999999999999999999",
        "0x100000000",
        "4294967295 + 1",
    ] {
        let post = deferred(
            &format!("layout(location = {location}) out vec4 c;"),
            "c = vec4(1.0);",
        );
        skipped_at(&post, "deferred.fsh:2");
    }
}

/// A value Sunrise can't read (missing, negative, not a constant) is not refused: the output
/// takes the next free location, or component 0, as before.
#[test]
fn an_unread_layout_value_still_takes_the_next_location() {
    for layout in [
        "location",
        "location = -1",
        "location = UNDEFINED",
        "location = 0, component = -1",
        "location = 0, component = UNDEFINED",
    ] {
        let post = deferred(&format!("layout({layout}) out vec4 c;"), "c = vec4(1.0);");
        assert_eq!(outputs(&post), [0], "{layout}");
    }
}

/// The issue's second shape, as a pack writes it, and any location past `i32::MAX`: Sunrise
/// declares the output again with the location as an `int` literal, so it is refused at the
/// pack's own line. The largest `int` location still translates.
#[test]
fn a_fragment_output_past_an_int_location_is_refused() {
    for (outputs, body) in [
        (
            "layout(location = 4294967295) out vec4 c[2];",
            "c[0] = vec4(1.0); c[1] = vec4(1.0);",
        ),
        (
            "layout(location = 4294967295u) out vec4 c[2];",
            "c[0] = vec4(1.0); c[1] = vec4(1.0);",
        ),
        (
            "layout(location = 2147483648) out vec4 c;",
            "c = vec4(1.0);",
        ),
    ] {
        let post = deferred(outputs, body);
        skipped_at(&post, "deferred.fsh:2");
    }
    let post = deferred(
        "layout(location = 2147483647) out vec4 c;",
        "c = vec4(1.0);",
    );
    assert_eq!(outputs(&post), [2147483647]);
}

/// A location or component written in another of GLSL's integer forms (a `u` suffix, hex,
/// octal, a constant expression; 4.1.3 and 4.4) is that value, not the next free location.
#[test]
fn a_fragment_output_location_in_any_integer_form_is_its_value() {
    for location in ["2", "2u", "0x2", "02", "1 + 1", "(4 - 2)", "max(1, 2)"] {
        let post = deferred(
            &format!("layout(location = {location}) out vec4 c;"),
            "c = vec4(1.0);",
        );
        assert_eq!(outputs(&post), [2], "{location} {:?}", post.notes);
    }
    for component in ["1u", "0x1", "01", "2 - 1"] {
        let post = deferred(
            &format!(
                "layout(location = 3, component = 0) out float r;\n\
                 layout(location = 3, component = {component}) out vec3 gba;"
            ),
            "r = 1.0; gba = vec3(0.5);",
        );
        assert_eq!(outputs(&post), [3], "{component}");
    }
    // A leading `0` before digits octal doesn't have is read as decimal, as before octal was
    // read: never as a value past `u32`.
    for (location, value) in [("08", 8), ("09", 9), ("018u", 18)] {
        let post = deferred(
            &format!("layout(location = {location}) out vec4 c;"),
            "c = vec4(1.0);",
        );
        assert_eq!(outputs(&post), [value], "{location} {:?}", post.notes);
    }
}
