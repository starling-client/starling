//! Original GLSL for #327: a `const` local initialized with a constant expression is a constant
//! expression (GLSL 4.60, 4.3.3), so it may size an array (4.1.9) and label a `case` (6.2),
//! where it is the innermost declaration of its name in scope (4.2.2).
use std::io::{Cursor, Write};

use sunrise_shaders::{
    GameInputs, GbuffersInputs, Macros, Pack, Preprocessed, Program, Target, load_post,
    read_properties, translate_composite,
};

fn stage(code: &str) -> Preprocessed {
    let mut pp = Preprocessed::from_source("original.fsh", code);
    pp.version = Some((330, Some("compatibility".into())));
    pp
}

fn fragment(source: &str) -> Result<Program, Vec<sunrise_shaders::Error>> {
    translate_composite(
        &stage("void main() { gl_Position = gl_Vertex; }"),
        &stage(source),
        Target::Window,
        naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
    )
}

/// Every function of the fragment stage: the pack's `main` is called from the entry point.
fn functions(program: &Program) -> impl Iterator<Item = &naga::Function> {
    program.fragment.functions.iter().map(|(_, f)| f)
}

/// The sizes of the local arrays, in declaration order.
fn local_array_sizes(program: &Program) -> Vec<u32> {
    functions(program)
        .flat_map(|f| f.local_variables.iter())
        .filter_map(|(_, local)| match program.fragment.types[local.ty].inner {
            naga::TypeInner::Array {
                size: naga::ArraySize::Constant(size),
                ..
            } => Some(size.get()),
            _ => None,
        })
        .collect()
}

/// The labels of every `switch`, in order.
fn case_labels(program: &Program) -> Vec<naga::SwitchValue> {
    functions(program)
        .flat_map(|f| f.body.iter())
        .filter_map(|s| match s {
            naga::Statement::Switch { cases, .. } => Some(cases.iter().map(|c| c.value)),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn a_const_local_sizes_arrays_where_it_is_the_innermost_name() {
    let program = fragment(
        "const int n = 2;\n\
         void main() {\n\
         const int n = 5;\n\
         float a[n];\n\
         a[0] = 1.0;\n\
         const int m = 3;\n\
         float s = a[0];\n\
         {\n\
         const int m = 7;\n\
         float b[m];\n\
         b[0] = 2.0;\n\
         s += b[0];\n\
         }\n\
         const int k = m + 1;\n\
         float c[k] = float[k](1.0, 2.0, 3.0, 4.0);\n\
         gl_FragColor = vec4(s + c[3]);\n\
         }\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(local_array_sizes(&program), [5, 7, 4]);
}

#[test]
fn a_const_local_labels_a_case() {
    let program = fragment(
        "uniform int pick;\n\
         void main() {\n\
         const int three = 3;\n\
         float v = 0.0;\n\
         switch (pick) {\n\
         case three: v = 1.0; break;\n\
         default: v = 0.5; break;\n\
         }\n\
         gl_FragColor = vec4(v);\n\
         }\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        case_labels(&program),
        [naga::SwitchValue::I32(3), naga::SwitchValue::Default]
    );
}

/// An array constructor inside an array size or a `case` label: its own size is a constant
/// expression nested in the outer one, and still sees the names where the outer one is written,
/// also over a global of the same name (whose 2 would make each constructor too long).
fn nested(body: &str) -> [Program; 2] {
    let local = format!(
        "uniform int pick;\nvoid main() {{\nconst int n = 3;\n{body}gl_FragColor = vec4(1.0);\n}}\n"
    );
    let shadowing = format!("const int n = 2;\n{local}");
    [local, shadowing].map(|source| fragment(&source).unwrap_or_else(|e| panic!("{e:?}")))
}

#[test]
fn a_constructor_in_an_array_size_is_sized_by_the_const_local() {
    for program in nested("float a[int[n](4, 5, 6)[2]];\na[0] = 1.0;\n") {
        assert_eq!(local_array_sizes(&program), [6]);
    }
}

#[test]
fn a_constructor_in_a_case_label_is_sized_by_the_const_local() {
    for program in nested("switch (pick) {\ncase int[n](7, 8, 9)[1]: break;\ndefault: break;\n}\n")
    {
        assert_eq!(
            case_labels(&program),
            [naga::SwitchValue::I32(8), naga::SwitchValue::Default]
        );
    }
}

/// A shader with a global `n` of 3 where `inner` declares a `const` local `n` of 2 in a nested
/// scope, then sizes an array and labels a case with `n` after that scope has ended.
fn after_scope(inner: &str) -> Program {
    fragment(&format!(
        "uniform int pick;\nconst int n = 3;\nvoid main() {{\nfloat s = 0.0;\n{inner}\nfloat a[n];\n\
         a[0] = s;\nswitch (pick) {{ case n: a[0] = 1.0; break; default: break; }}\n\
         gl_FragColor = vec4(a[0]);\n}}\n"
    ))
    .unwrap_or_else(|e| panic!("{inner}\n{e:?}"))
}

/// Sizes and labels with the global `n` of 3 after a scope that declared a local `n` of 2.
fn assert_global_after(inner: &str, labels_before: &[naga::SwitchValue]) {
    let program = after_scope(inner);
    assert_eq!(local_array_sizes(&program), [3], "{inner}");
    let mut labels = labels_before.to_vec();
    labels.extend([naga::SwitchValue::I32(3), naga::SwitchValue::Default]);
    assert_eq!(case_labels(&program), labels, "{inner}");
}

// A name declared in a nested scope is not in scope after it (GLSL 4.60, 4.2.2): the
// if-statement and else-statement, a loop's body (braced or not) and a switch's body (6.2).

#[test]
fn a_const_local_in_a_switch_body_ends_with_the_switch() {
    assert_global_after(
        "switch (pick) { default: const int n = 2; s = float(n); break; }",
        &[naga::SwitchValue::Default],
    );
}

#[test]
fn a_const_local_in_an_unbraced_if_ends_with_it() {
    assert_global_after("if (pick == 1) const int n = 2;", &[]);
}

#[test]
fn a_const_local_in_an_unbraced_else_ends_with_it() {
    assert_global_after("if (pick == 1) s = 1.0; else const int n = 2;", &[]);
}

#[test]
fn a_const_local_in_an_unbraced_while_ends_with_it() {
    assert_global_after("while (pick > 9) const int n = 2;", &[]);
}

#[test]
fn a_const_local_in_an_unbraced_do_ends_with_it() {
    assert_global_after("do const int n = 2; while (pick > 9);", &[]);
}

#[test]
fn a_do_bodys_name_is_seen_by_its_condition() {
    // Accepted before the body had a scope: the condition still sees an unbraced body's name.
    fragment(
        "void main() {\nfloat s = 0.0;\ndo int x = 1; while (x < 3);\ngl_FragColor = vec4(s);\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn an_if_statements_name_is_not_in_its_else() {
    let program = fragment(
        "uniform int pick;\nconst int n = 3;\nvoid main() {\nfloat s = 0.0;\n\
         if (pick == 1) const int n = 2; else { float b[n]; b[0] = 1.0; s = b[0]; }\n\
         gl_FragColor = vec4(s);\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(local_array_sizes(&program), [3]);
}

#[test]
fn a_for_conditions_name_is_in_its_body() {
    fragment(
        "void main() {\nfloat s = 0.0;\n\
         for (int i = 0; bool go = i < 1; i++) s += go ? 1.0 : 0.0;\n\
         gl_FragColor = vec4(s);\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn a_const_local_in_a_for_or_a_block_ends_with_it() {
    // Scoped before this change too: guards that they stay so.
    assert_global_after("for (int i = 0; i < 1; i++) const int n = 2;", &[]);
    assert_global_after("for (const int n = 2; pick > 9;) s = 1.0;", &[]);
    assert_global_after("{ const int n = 2; s = float(n); }", &[]);
}

#[test]
fn a_const_local_in_one_case_is_in_scope_in_the_next() {
    // The cases share the switch's one scope (6.2), so a later case sees an earlier one's name.
    let program = fragment(
        "uniform int pick;\nvoid main() {\nfloat s = 0.0;\nswitch (pick) {\n\
         case 0: const int k = 4; s = float(k); break;\n\
         default: float b[k]; b[0] = 1.0; s = b[0]; break;\n}\n\
         gl_FragColor = vec4(s);\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(local_array_sizes(&program), [4]);
}

#[test]
fn names_that_are_not_constant_expressions_still_size_nothing() {
    // A `const` local from a uniform is not a constant expression; one in another function is
    // not in scope. GLSL refuses both, and so does the translation.
    let from_uniform = "uniform float k;\nvoid main() { const int n = int(k); float a[n]; a[0] = 1.0; gl_FragColor = vec4(a[0]); }\n";
    let other_function = "float f() { const int n = 4; return float(n); }\nvoid main() { float a[n]; a[0] = f(); gl_FragColor = vec4(a[0]); }\n";
    assert!(fragment(from_uniform).is_err());
    assert!(fragment(other_function).is_err());
}

#[test]
fn a_block_program_sized_by_a_const_local_draws_block_entities() {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let vertex = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
    for (path, text) in [
        ("shaders/gbuffers_terrain.vsh", vertex),
        (
            "shaders/gbuffers_terrain.fsh",
            "#version 120\nvoid main() { gl_FragData[0] = vec4(1.0); }\n",
        ),
        ("shaders/gbuffers_block.vsh", vertex),
        (
            "shaders/gbuffers_block.fsh",
            "#version 120\nvoid main() {\n    const int samples = 4;\n    float w[samples];\n    \
             for (int i = 0; i < samples; i++) w[i] = 0.25;\n    \
             gl_FragData[0] = vec4(w[0] + w[3]);\n}\n",
        ),
    ] {
        archive
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    }
    let pack =
        Pack::from_zip_bytes("original-327.zip", archive.finish().unwrap().into_inner()).unwrap();
    let mut macros = Macros::standard();
    let (properties, notes) = read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let inputs = GameInputs {
        entities: Some(GbuffersInputs::default()),
        ..GameInputs::default()
    };
    let post = load_post(&pack, "", &properties, &macros, &inputs);
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(
        post.block.map(|p| p.name).as_deref(),
        Some("gbuffers_block")
    );
}

// `.length()` of a `const` array inside an array size: the array is lowered outside the
// constant context, for its type only, so the name must resolve to an expression of the
// function's own arena (GLSL 4.60, 4.3.3: the length of an explicitly sized array is a constant
// expression). Each case sizes `a` with a length no other array in the shader has.

/// The local array sizes of `main` when `size` sizes `float a[size]`, after `decls` (globals)
/// and `locals` (the start of `main`).
fn sized_by(decls: &str, locals: &str, size: &str) -> Vec<u32> {
    let source = format!(
        "{decls}\nvoid main() {{\n{locals}\nfloat a[{size}];\na[0] = 1.0;\n\
         gl_FragColor = vec4(a[0]);\n}}\n"
    );
    local_array_sizes(&fragment(&source).unwrap_or_else(|e| panic!("{source}\n{e:?}")))
}

/// Sized global arrays declared before the `const` arrays, so a handle into the wrong arena
/// lands on one of them (or past the end) instead of on the `const` array.
const SIZED_GLOBALS: &str = "uniform float u9[9];\nuniform vec2 v11[11];\n\
                             const int g2[2] = int[2](20, 21);\n";

#[test]
fn length_of_a_global_const_array_sizes_an_array() {
    let decls = "const float g[4] = float[4](1.0, 2.0, 3.0, 4.0);";
    assert_eq!(sized_by(decls, "", "g.length()"), [4]);
    let decls = format!("{SIZED_GLOBALS}{decls}");
    assert_eq!(sized_by(&decls, "", "g.length()"), [4]);
}

#[test]
fn length_of_a_local_const_array_sizes_an_array() {
    let locals = "const float l[3] = float[3](1.0, 2.0, 3.0);";
    assert_eq!(sized_by("", locals, "l.length()"), [3]);
    assert_eq!(sized_by(SIZED_GLOBALS, locals, "l.length()"), [3]);
}

#[test]
fn length_of_an_element_of_a_const_array_of_arrays_sizes_an_array() {
    // The indexed element's type is resolved in the same arena as the array.
    let global = "const float gm[2][5] = float[2][5](float[5](1.0, 2.0, 3.0, 4.0, 5.0), \
                  float[5](6.0, 7.0, 8.0, 9.0, 10.0));";
    assert_eq!(
        sized_by(&format!("{SIZED_GLOBALS}{global}"), "", "gm[1].length()"),
        [5]
    );
    let local = "const float lm[2][6] = float[2][6](float[6](1.0, 2.0, 3.0, 4.0, 5.0, 6.0), \
                 float[6](7.0, 8.0, 9.0, 10.0, 11.0, 12.0));";
    assert_eq!(sized_by(SIZED_GLOBALS, local, "lm[0].length()"), [6]);
}

#[test]
fn length_of_a_member_of_a_const_struct_sizes_an_array() {
    let decls = format!(
        "{SIZED_GLOBALS}struct S {{ float f; int arr[7]; }};\n\
         const S gs = S(1.0, int[7](1, 2, 3, 4, 5, 6, 7));"
    );
    assert_eq!(sized_by(&decls, "", "gs.arr.length()"), [7]);
    let local = "const S ls = S(2.0, int[7](1, 2, 3, 4, 5, 6, 7));";
    let decls = format!("{SIZED_GLOBALS}struct S {{ float f; int arr[7]; }};");
    assert_eq!(sized_by(&decls, local, "ls.arr.length()"), [7]);
}

#[test]
fn length_of_a_const_array_as_an_argument_sizes_an_array() {
    // A call's arguments are lowered in the constant context, through the constant itself.
    let decls = format!("{SIZED_GLOBALS}const float g[4] = float[4](1.0, 2.0, 3.0, 4.0);");
    let locals = "const float l[3] = float[3](1.0, 2.0, 3.0);";
    assert_eq!(
        sized_by(&decls, locals, "max(g.length(), l.length()) + 4"),
        [8]
    );
    assert_eq!(
        sized_by(&decls, locals, "g2[1] - g.length() - l.length()"),
        [14]
    );
}

#[test]
fn length_of_a_const_array_labels_a_case() {
    let program = fragment(
        "uniform int pick;\nuniform float u9[9];\n\
         const float g[4] = float[4](1.0, 2.0, 3.0, 4.0);\nvoid main() {\n\
         const float l[3] = float[3](1.0, 2.0, 3.0);\nfloat v = 0.0;\nswitch (pick) {\n\
         case g.length(): v = 1.0; break;\ncase l.length(): v = 2.0; break;\n\
         default: break;\n}\ngl_FragColor = vec4(v + u9[0]);\n}\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        case_labels(&program),
        [
            naga::SwitchValue::I32(4),
            naga::SwitchValue::I32(3),
            naga::SwitchValue::Default
        ]
    );
}
