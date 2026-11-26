use super::*;

/// The lowest stage interface limits a device has.
const LIMITS: InterfaceLimits = crate::tests::LIMITS;

/// Line `n` of a stage as the author's file `pack.csh` at line `100 + n`.
fn locate(line: usize) -> Location {
    Location {
        file: "pack.csh".into(),
        line: 100 + line as u32,
    }
}

fn binding(group: u32, binding: u32) -> naga::ResourceBinding {
    naga::ResourceBinding { group, binding }
}

/// The binding of the global named `name`.
fn bound(module: &naga::Module, name: &str) -> Option<naga::ResourceBinding> {
    (module.global_variables.iter())
        .find(|(_, g)| g.name.as_deref() == Some(name))
        .and_then(|(_, g)| g.binding)
}

fn compute(header: &str, code: &str, bindings: &BindingMap) -> Result<naga::Module, Vec<Error>> {
    compile_stage(
        &Source {
            stage: ShaderStage::Compute,
            header,
            code,
            locate: &locate,
        },
        bindings,
        LIMITS,
    )
}

#[test]
fn the_binding_map_moves_what_the_code_declares_not_the_header() {
    // The header and the code both declare a resource at set 0, binding 2, and the main reads
    // both: unmoved they collide, and moved alike they collide in set 4.
    let header = "#version 450\nlayout(set = 0, binding = 2) uniform Frame { float t; } frame;\n";
    let code = "layout(local_size_x = 1) in;\n\
                layout(std430, binding = 2) buffer Data { float v[]; } data;\n\
                void main() { data.v[0] = frame.t; }\n";
    let bindings = BindingMap::from([(binding(0, 2), binding(4, 2))]);
    let module = compute(header, code, &bindings).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(bound(&module, "frame"), Some(binding(0, 2)));
    assert_eq!(bound(&module, "data"), Some(binding(4, 2)));
}

#[test]
fn the_binding_map_is_keyed_by_group_and_binding() {
    let header = "#version 450\n";
    let code = "layout(local_size_x = 1) in;\n\
                layout(std430, set = 1, binding = 2) buffer A { float a[]; } ga;\n\
                layout(std430, set = 0, binding = 2) buffer B { float b[]; } gb;\n\
                layout(std430, set = 1, binding = 3) buffer C { float c[]; } gc;\n\
                void main() { ga.a[0] = gb.b[0] + gc.c[0]; }\n";
    let bindings = BindingMap::from([(binding(1, 2), binding(6, 0))]);
    let module = compute(header, code, &bindings).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(bound(&module, "ga"), Some(binding(6, 0)));
    assert_eq!(bound(&module, "gb"), Some(binding(0, 2)));
    assert_eq!(bound(&module, "gc"), Some(binding(1, 3)));
}

#[test]
fn parse_errors_name_where_the_caller_says_their_line_came_from() {
    let header = "#version 450\n// ours\n";
    let code = "layout(local_size_x = 1) in;\nvoid main() {\n    float x = undefined_name;\n}\n";
    let errors = compute(header, code, &BindingMap::new()).unwrap_err();
    // The undefined name is on line 4 of the stage (from 0): the code's third.
    assert_eq!(
        errors
            .iter()
            .map(|e| (e.file.as_str(), e.line, e.stage))
            .collect::<Vec<_>>(),
        [("pack.csh", 104, Some(ShaderStage::Compute))]
    );
}

#[test]
fn validation_errors_name_where_the_caller_says_their_line_came_from() {
    // Two buffers the main reads at one binding: naga's validator refuses the second, which is
    // on line 3 of the stage (from 0).
    let header = "#version 450\n";
    let code = "layout(local_size_x = 1) in;\n\
                layout(std430, binding = 2) buffer A { float a[]; } ga;\n\
                layout(std430, binding = 2) buffer B { float b[]; } gb;\n\
                void main() { ga.a[0] = gb.b[0]; }\n";
    let errors = compute(header, code, &BindingMap::new()).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .map(|e| (e.file.as_str(), e.line))
            .collect::<Vec<_>>(),
        [("pack.csh", 103)]
    );
}
