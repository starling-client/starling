//! Integer texel buffers in a pack's core shader (26.3's `clouds.vsh` reads its faces from
//! `uniform isamplerBuffer CloudFaces`): a read-only storage buffer the pipeline gives.
use super::*;

const GLUE: Glue = Glue {
    inputs: &[],
    prelude: "",
    setup: "",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
};

/// `clouds.vsh` in shape: a block of its own and an integer texel buffer read with `texelFetch`
/// (spaced oddly once), next to a `sampler2D` read the same way and a function whose name only
/// starts with `texelFetch`.
const VSH: &str = "#version 330\n\
    layout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n};\n\
    layout(std140) uniform CloudInfo {\n    vec4 CloudColor;\n    vec3 CloudOffset;\n    vec3 CellSize;\n};\n\
    uniform isamplerBuffer CloudFaces;\nuniform usamplerBuffer CloudFlags;\nuniform sampler2D Sampler0;\n\
    layout(location = 1) out vec4 vertexColor;\n\
    int texelFetchTwice(int i) { return i * 2; }\n\
    void main() {\n    int index = (gl_VertexIndex / 4) * 3;\n\
        int cellX = texelFetch(CloudFaces, index).r;\n    int flags = texelFetch( CloudFaces ,index + 2).r;\n\
        uint more = texelFetch(CloudFlags, texelFetchTwice(index)).r;\n\
        vec4 tint = texelFetch(Sampler0, ivec2(0), 0);\n\
        gl_Position = ModelViewMat * vec4(vec3(cellX, flags, float(more)) * CellSize + CloudOffset, 1.0);\n\
        vertexColor = CloudColor * tint;\n}\n";
const FSH: &str = "#version 330\nlayout(location = 1) in vec4 vertexColor;\n\
    out vec4 fragColor;\nvoid main() {\n    fragColor = vertexColor;\n}\n";

/// The storage buffers of `module`: each one's binding and element type.
fn storage(module: &naga::Module) -> Vec<(u32, naga::ScalarKind)> {
    module
        .global_variables
        .iter()
        .filter(|(_, v)| matches!(v.space, naga::AddressSpace::Storage { .. }))
        .map(|(_, v)| {
            let binding = v.binding.as_ref().expect("bound").binding;
            let naga::TypeInner::Struct { members, .. } = &module.types[v.ty].inner else {
                panic!("a block");
            };
            let naga::TypeInner::Array { base, .. } = module.types[members[0].ty].inner else {
                panic!("an array");
            };
            let naga::TypeInner::Scalar(scalar) = module.types[base].inner else {
                panic!("of scalars");
            };
            (binding, scalar.kind)
        })
        .collect()
}

#[test]
fn integer_texel_buffers_become_storage_buffers_the_pipeline_gives() {
    let given = |b: &str| matches!(b, "CloudInfo" | "CloudFaces" | "CloudFlags");
    let program = translate_core_with(VSH, FSH, &[], &|_| None, &GLUE, &given)
        .expect("the clouds shaders translate");
    assert_eq!(program.other, ["CloudInfo", "CloudFaces", "CloudFlags"]);
    // Each buffer a read-only storage buffer of its integers, at its binding; Sampler0 keeps
    // its own binding, its texelFetch untouched.
    assert_eq!(
        storage(&program.vertex),
        [
            (other_binding(1), naga::ScalarKind::Sint),
            (other_binding(2), naga::ScalarKind::Uint)
        ]
    );
    assert!(program.vertex.global_variables.iter().any(|(_, v)| {
        v.binding
            .as_ref()
            .is_some_and(|b| b.binding == SAMPLERS[0].1)
    }));
    // Not given, a buffer is refused as an unknown block is.
    let without = |b: &str| matches!(b, "CloudInfo" | "CloudFaces");
    assert!(translate_core_with(VSH, FSH, &[], &|_| None, &GLUE, &without).is_err());
}

/// Two buffers' fetches, interleaved, rewritten in the one pass.
#[test]
fn every_buffers_fetches_are_rewritten_in_one_pass() {
    let text = "uniform isamplerBuffer A;\nuniform usamplerBuffer B;\n\
                int a = texelFetch(A, 1).r + int(texelFetch(B,2).r) + texelFetch(A,3).r;\n";
    assert_eq!(
        texel_fetches(text),
        "uniform isamplerBuffer A;\nuniform usamplerBuffer B;\n\
         int a = hz_fetch_A( 1).r + int(hz_fetch_B(2).r) + hz_fetch_A(3).r;\n"
    );
}

/// The rewrite only touches `texelFetch` calls on a declared integer buffer.
#[test]
fn only_fetches_from_integer_buffers_are_rewritten() {
    let text = "uniform isamplerBuffer Faces;\nint a = texelFetch(Faces, 1).r;\n\
                int b = texelFetchTwice(2);\nvec4 c = texelFetch(Sampler0, ivec2(0), 0);\n\
                int d = texelFetch(FacesMore, 3).r;\nint e = mytexelFetch(Faces, 4);\n\
                int f = texelFetchAll(Faces, 5);\n";
    assert_eq!(
        texel_fetches(text),
        "uniform isamplerBuffer Faces;\nint a = hz_fetch_Faces( 1).r;\n\
         int b = texelFetchTwice(2);\nvec4 c = texelFetch(Sampler0, ivec2(0), 0);\n\
         int d = texelFetch(FacesMore, 3).r;\nint e = mytexelFetch(Faces, 4);\n\
         int f = texelFetchAll(Faces, 5);\n"
    );
}
