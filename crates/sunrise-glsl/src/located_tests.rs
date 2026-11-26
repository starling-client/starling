//! Errors at the file, line and column the author wrote, through nested includes and rewrites.
use super::tests::{FSH_FILE, LIMITS, VSH_FILE, translate_core_at};
use super::*;

const GLUE: Glue = Glue {
    inputs: &[("Position", "vec3")],
    prelude: "layout(location = 0) in vec3 hz_position;\nint hz_unused() {\n    return 0;\n}",
    setup: "Position = hz_position;",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: LIMITS,
};

const VSH: &str = "#version 330\nin vec3 Position;\nout vec4 vertexColor;\n\
    void main() { vertexColor = vec4(1.0); gl_Position = vec4(Position, 1.0); }\n";

/// The files these tests include, by id.
const FILES: [(&str, &str); 6] = [
    // A function that parses and fails validation: a constant index past its array, line 4.
    (
        "minecraft:shaders/include/pick.glsl",
        "// pick\nfloat pick() {\n    float a[2] = float[2](1.0, 2.0);\n    return a[5];\n}\n",
    ),
    // An inactive branch, then a quoted include from this file's folder.
    (
        "minecraft:shaders/include/outer.glsl",
        "#ifdef MISSING\nfloat dropped;\n#endif\n#include \"inner/deep.glsl\"\nfloat outer_kept;\n",
    ),
    // A comment over two lines, a continued line and a `#line` before the error on line 6.
    (
        "minecraft:shaders/include/inner/deep.glsl",
        "/* a comment\n   over lines */\nfloat spliced = \\\n    1.0;\n#line 100\nfloat broken = undefined_thing;\n",
    ),
    // A helper taking the sampler it reads, above the sampler's declaration: copied below it.
    (
        "minecraft:shaders/include/tap.glsl",
        "vec4 tap(sampler2D s, vec2 uv) {\n    vec4 c = texture(s, uv);\n    return texture(s, uv) * missing_scale;\n}\n",
    ),
    // An include of a file nobody has, on line 2.
    (
        "minecraft:shaders/include/asks.glsl",
        "float asks_first;\n#include <minecraft:absent.glsl>\n",
    ),
    // Comments kept (an expansion without defines keeps every line as written).
    (
        "minecraft:shaders/include/plain.glsl",
        "float a;\n/* two\n   lines */ float b;\n",
    ),
];

fn read(id: &str) -> Result<Option<String>, String> {
    Ok(FILES
        .iter()
        .find(|(file, _)| *file == id)
        .map(|(_, text)| (*text).to_owned()))
}

fn translate(vertex: &str, fragment: &str) -> Result<Translated, Vec<Error>> {
    let stages = Stages {
        vertex_dir: "minecraft:shaders/core/",
        fragment_dir: "minecraft:shaders/core/",
        vertex,
        fragment,
        vertex_file: VSH_FILE,
        fragment_file: FSH_FILE,
    };
    translate_core_at(&stages, &[], &read, &GLUE, &|_| false)
}

/// The file and line of each error.
fn located(errors: &[Error]) -> Vec<(&str, u32)> {
    errors.iter().map(|e| (e.file.as_str(), e.line)).collect()
}

/// An error in a nested include names its file, physical line and column, past the lines an
/// inactive branch drops, a comment, a continued line and a `#line`.
#[test]
fn an_error_in_a_nested_include_names_that_file_and_line() {
    let fsh = "#version 330\n// the stage\n#include <minecraft:outer.glsl>\n\
        in vec4 vertexColor;\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor * broken; }\n";
    let errors = translate(VSH, fsh).unwrap_err();
    assert_eq!(
        (errors[0].file.as_str(), errors[0].line, errors[0].column),
        ("minecraft:shaders/include/inner/deep.glsl", 6, 16)
    );
    assert_eq!(errors[0].stage, Some(ShaderStage::Fragment));
}

/// The stage keeps its line and column past glue, rewritten declarations and macro expansion.
#[test]
fn an_error_in_the_stage_keeps_its_line_past_the_rewrites() {
    let vsh = "#version 330\nuniform sampler2D Sampler0;\nin vec3 Position;\n\
        out vec4 vertexColor;out vec2 uv;\n#define S 100.0\nvoid main() {\n\
            vertexColor = texture(Sampler0, vec2(0.0));\n\
            uv = vec2(S); gl_Position = vec4(Position, nope);\n}\n";
    let fsh = "#version 330\nin vec4 vertexColor;\nin vec2 uv;\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor * uv.x; }\n";
    let errors = translate(vsh, fsh).unwrap_err();
    assert_eq!(
        (errors[0].file.as_str(), errors[0].line, errors[0].column),
        (VSH_FILE, 8, 44)
    );
    assert_eq!(errors[0].stage, Some(ShaderStage::Vertex));
    assert!(errors[0].message.starts_with('('), "{errors:?}");
}

/// A helper taking a sampler is copied per sampler passed, below the sampler's declaration:
/// an error names its original line and column, even after a parameter changes on that line.
#[test]
fn an_error_in_a_copied_helper_names_its_bodys_line() {
    let fsh = "#version 330\n#moj_import <minecraft:tap.glsl>\nuniform sampler2D Sampler0;\n\
        in vec4 vertexColor;\nout vec4 fragColor;\n\
        void main() { fragColor = tap(Sampler0, vec2(0.5)) * vertexColor; }\n";
    let errors = translate(VSH, fsh).unwrap_err();
    assert_eq!(
        (errors[0].file.as_str(), errors[0].line, errors[0].column),
        ("minecraft:shaders/include/tap.glsl", 3, 29)
    );
}

/// An include refused (a file nobody has) is named at the directive asking for it, in the
/// included file that has it.
#[test]
fn a_refused_include_names_the_directive_in_the_file_that_asks() {
    let fsh = "#version 330\n#include <minecraft:asks.glsl>\nin vec4 vertexColor;\n\
        out vec4 fragColor;\nvoid main() { fragColor = vertexColor; }\n";
    let errors = translate(VSH, fsh).unwrap_err();
    assert_eq!(
        errors,
        [Error {
            file: "minecraft:shaders/include/asks.glsl".into(),
            line: 2,
            column: 0,
            stage: Some(ShaderStage::Fragment),
            unprovided: None,
            message: "minecraft:shaders/include/absent.glsl not found".into(),
        }]
    );
}

/// A location the fragment stage reads and the vertex stage doesn't write is named at the
/// input that reads it.
#[test]
fn a_location_no_stage_writes_names_the_input() {
    let fsh = "#version 330\nin vec4 vertexColor;\nlayout(location = 5) in vec4 extra;\n\
        out vec4 fragColor;\nvoid main() { fragColor = vertexColor * extra; }\n";
    let errors = translate(VSH, fsh).unwrap_err();
    assert_eq!(
        errors,
        [Error {
            file: FSH_FILE.into(),
            line: 3,
            column: 1,
            stage: Some(ShaderStage::Fragment),
            unprovided: None,
            message: "the fragment shader reads location 5, which the vertex shader doesn't write"
                .into(),
        }]
    );
}

/// Expanded without defines (a post shader's way), each line is where the author wrote it, and
/// text wrapped around and edited into it keeps that; Sunrise's own lines name the stage file.
#[test]
fn an_expansion_without_defines_locates_each_line() {
    let includes = Includes {
        read: &read,
        namespace: "minecraft",
        folder: "shaders/include/",
        once: &["moj_import"],
    };
    let mut expanded = expand_includes(
        "#version 330\n#include <minecraft:plain.glsl>\nfloat after;\n",
        "minecraft:shaders/post/blur.fsh",
        "minecraft:shaders/post/",
        &includes,
        None,
    )
    .unwrap();
    let at_a = expanded.text().find("float a;").unwrap();
    expanded.edit(vec![(at_a..at_a + 8, "float a;\nfloat added;".into())]);
    expanded.wrap("#version 450\n", "\nvoid main() {}\n");
    let at = |needle: &str| {
        let error = expanded.error_at(expanded.text().find(needle), String::new());
        (error.file, error.line)
    };
    let plain = "minecraft:shaders/include/plain.glsl";
    let stage = "minecraft:shaders/post/blur.fsh";
    assert_eq!(at("float a;"), (plain.into(), 1));
    assert_eq!(at("float added;"), (plain.into(), 1));
    assert_eq!(at("float b;"), (plain.into(), 3));
    assert_eq!(at("float after;"), (stage.into(), 3));
    assert_eq!(at("#version 450"), (stage.into(), 0));
    assert_eq!(at("void main()"), (stage.into(), 0));
}

/// An error naga's validator finds (the module parses) in an included file names that file and
/// the line its span starts on.
#[test]
fn a_validation_error_in_an_include_names_its_file_and_line() {
    let fsh = "#version 330\n#include <minecraft:pick.glsl>\nin vec4 vertexColor;\n\
        out vec4 fragColor;\nvoid main() { fragColor = vertexColor * pick(); }\n";
    let errors = translate(VSH, fsh).unwrap_err();
    assert_eq!(
        located(&errors),
        [("minecraft:shaders/include/pick.glsl", 4)]
    );
    // Not the parser's: its errors quote the line they read.
    assert!(!errors[0].message.starts_with('('), "{errors:?}");
}

/// A validation error whose span is in no line the author wrote (an entry point using what its
/// stage forbids, the entry point being Sunrise's `main`) names the stage file with no line.
#[test]
fn a_validation_error_outside_the_authors_lines_names_the_stage_file() {
    let vsh = "#version 330\nin vec3 Position;\nout vec4 vertexColor;\n\
        void main() { vertexColor = vec4(dFdx(Position.x)); gl_Position = vec4(Position, 1.0); }\n";
    let fsh = "#version 330\nin vec4 vertexColor;\nout vec4 fragColor;\n\
        void main() { fragColor = vertexColor; }\n";
    let errors = translate(vsh, fsh).unwrap_err();
    assert_eq!(located(&errors), [(VSH_FILE, 0)]);
}

/// A translated program says where a stage declares a resource, a stage reading it first, and
/// where the fragment stage declares an output: what checks after translation name.
#[test]
fn a_program_locates_its_declarations_and_outputs() {
    let vsh = "#version 330\nuniform sampler2D Sampler1;\nin vec3 Position;\nout vec4 vertexColor;\n\
        void main() { vertexColor = vec4(1.0); gl_Position = vec4(Position, 1.0); }\n";
    let fsh = "#version 330\n#include <minecraft:plain.glsl>\nuniform sampler2D Sampler1;\n\
        in vec4 vertexColor;\nout vec4 fragColor;\nlayout(location = 1) out vec4 extra;\n\
        void main() { fragColor = vertexColor * texture(Sampler1, vec2(0.5)); extra = vec4(a + b); }\n";
    let program = translate(vsh, fsh).unwrap();
    let at = |l: Location| (l.file, l.line);
    // Sampler1 (texture at 6, sampler object at 7): declared by both, read by the fragment.
    assert_eq!(at(program.declared_at(6)), (FSH_FILE.into(), 3));
    assert_eq!(at(program.declared_at(7)), (FSH_FILE.into(), 3));
    assert_eq!(at(program.output_at(0)), (FSH_FILE.into(), 5));
    assert_eq!(at(program.output_at(1)), (FSH_FILE.into(), 6));
    // Nothing declared there: the vertex file, no line; no output there: the fragment file.
    assert_eq!(at(program.declared_at(40)), (VSH_FILE.into(), 0));
    assert_eq!(at(program.output_at(3)), (FSH_FILE.into(), 0));
}
