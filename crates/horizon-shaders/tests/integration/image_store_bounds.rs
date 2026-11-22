//! A store outside a storage image: GL drops it, and shader packs rely on that (Derivative's
//! `deferred8` runs 32x32 work groups past the screen's edge and stores there unchecked). On
//! Vulkan, wgpu checks image loads where the device lacks `robustImageAccess`; the vendored naga
//! then skips such a store too, which the GPU would otherwise write wherever it lands.

use naga::proc::BoundsCheckPolicy;

const SOURCE: &str = "#version 450\nlayout(local_size_x = 32, local_size_y = 32) in;\n\
                      layout(set = 0, binding = 0, rgba16f) uniform image2D img;\n\
                      void main() { imageStore(img, ivec2(gl_GlobalInvocationID.xy), vec4(1.0)); }\n";

/// The module as wgpu-hal writes it for Vulkan with `image_load` as the image policy.
fn spirv(image_load: BoundsCheckPolicy) -> Vec<u32> {
    let module = naga::front::glsl::Frontend::default()
        .parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Compute),
            SOURCE,
        )
        .unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let options = naga::back::spv::Options {
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            image_load,
            ..Default::default()
        },
        ..Default::default()
    };
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Compute,
        entry_point: "main".into(),
    };
    naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline)).unwrap()
}

/// The opcodes of a module's instructions, in order (after the five words of its header).
fn opcodes(words: &[u32]) -> Vec<u32> {
    let mut ops = Vec::new();
    let mut i = 5;
    while i < words.len() {
        ops.push(words[i] & 0xffff);
        i += ((words[i] >> 16) as usize).max(1);
    }
    ops
}

const IMAGE_WRITE: u32 = 99;
const IMAGE_QUERY_SIZE: u32 = 104;
const SELECTION_MERGE: u32 = 247;
const BRANCH_CONDITIONAL: u32 = 250;

#[test]
fn a_store_is_checked_where_loads_are() {
    for policy in [
        BoundsCheckPolicy::Restrict,
        BoundsCheckPolicy::ReadZeroSkipWrite,
    ] {
        let ops = opcodes(&spirv(policy));
        let write = ops
            .iter()
            .position(|&op| op == IMAGE_WRITE)
            .expect("the store");
        let before = &ops[..write];
        assert!(
            before.contains(&IMAGE_QUERY_SIZE)
                && before.contains(&SELECTION_MERGE)
                && before.contains(&BRANCH_CONDITIONAL),
            "{policy:?}: the store is not behind a check of the image's size: {ops:?}"
        );
    }
}

#[test]
fn a_store_stays_as_written_on_a_robust_device() {
    let ops = opcodes(&spirv(BoundsCheckPolicy::Unchecked));
    assert!(ops.contains(&IMAGE_WRITE));
    assert!(!ops.contains(&IMAGE_QUERY_SIZE), "{ops:?}");
}
