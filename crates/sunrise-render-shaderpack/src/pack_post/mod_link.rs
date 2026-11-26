//! Linking a mod's own vertex stage to a shader pack's fragment stage
//! ([`ModProgramMode::ModVertexPackFragment`](super::mod_pack::ModProgramMode)).
//!
//! The pack's fragment stage reads the varyings its own vertex stage writes. The mod's vertex
//! stage replaces that one, so it must write every varying the fragment stage reads, with the
//! same type and interpolation, under the same GLSL name: the translator gives each pack varying
//! its location, so the mod's outputs are matched by name and renumbered to those locations. The
//! struct member (or GLSL `out` variable) name is the varying's name. Anything else leaves the
//! slot unbound (the mod's own shaders draw), and never panics.

use std::collections::BTreeMap;

use super::mod_pack::MOD_SET_BASE;

/// One user-defined stage input or output: its name (when it has one), location, type,
/// interpolation and sampling.
struct Varying {
    name: Option<String>,
    location: u32,
    ty: naga::TypeInner,
    interpolation: naga::Interpolation,
    sampling: naga::Sampling,
}

fn varyings(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
    name: Option<&str>,
    binding: Option<&naga::Binding>,
    out: &mut Vec<Varying>,
) {
    match binding {
        Some(naga::Binding::Location {
            location,
            interpolation,
            sampling,
            ..
        }) => out.push(Varying {
            name: name.map(str::to_owned),
            location: *location,
            ty: module.types[ty].inner.clone(),
            interpolation: interpolation.unwrap_or(naga::Interpolation::Perspective),
            sampling: sampling.unwrap_or(naga::Sampling::Center),
        }),
        Some(_) => {}
        None => {
            if let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner {
                for member in members {
                    varyings(
                        module,
                        member.ty,
                        member.name.as_deref(),
                        member.binding.as_ref(),
                        out,
                    );
                }
            }
        }
    }
}

/// Gives the user-defined outputs under `binding`, in the order [`varyings`] lists them, the
/// locations `next` yields. A struct whose members change is replaced in place (its handle stays
/// valid); refused when the changed struct would equal another type of the module.
fn renumber(
    module: &mut naga::Module,
    ty: naga::Handle<naga::Type>,
    binding: &mut Option<naga::Binding>,
    next: &mut impl Iterator<Item = u32>,
) -> Result<(), String> {
    match binding {
        Some(naga::Binding::Location { location, .. }) => {
            *location = next.next().unwrap_or(*location);
        }
        Some(_) => {}
        None => {
            let old = module.types[ty].clone();
            let naga::TypeInner::Struct { members, span } = &old.inner else {
                return Ok(());
            };
            let mut members = members.clone();
            for member in &mut members {
                renumber(module, member.ty, &mut member.binding, next)?;
            }
            let new = naga::Type {
                name: old.name.clone(),
                inner: naga::TypeInner::Struct {
                    members,
                    span: *span,
                },
            };
            if new != old {
                if module.types.get(&new).is_some() {
                    return Err(
                        "the mod's vertex output struct cannot take the pack's locations".into(),
                    );
                }
                module.types.replace(ty, new);
            }
        }
    }
    Ok(())
}

/// The mod's vertex stage `vertex` made ready to run beside `pack_fragment`: its entry point
/// named `main`, its outputs matched by GLSL name to the pack's varyings (`pack_varyings`: name to
/// location, from the translator) and renumbered to their locations, its frame uniforms moved to
/// bind group [`MOD_SET_BASE`] and its own set 1 (when `has_set1`) to the one after. Outputs the
/// pack does not read move to locations the pack does not use. Fails with the reason when a
/// varying the pack reads is missing, unnamed or differs in type, the vertex stage uses something
/// a pack pass has no place for, or the bind groups would go over `max_bind_groups`.
pub fn link_vertex_stage(
    vertex: &naga::Module,
    pack_fragment: &naga::Module,
    pack_varyings: &BTreeMap<String, u32>,
    has_set1: bool,
    max_bind_groups: u32,
) -> Result<naga::Module, String> {
    let needed = MOD_SET_BASE + 1 + u32::from(has_set1);
    if needed > max_bind_groups {
        return Err(format!(
            "its bind groups after the pack's would number {needed}, over this device's limit of {max_bind_groups}"
        ));
    }
    let Some(vertex_entry) = vertex
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Vertex)
    else {
        return Err("the mod's shader has no vertex entry point".into());
    };
    let Some(fragment_entry) = pack_fragment
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Fragment)
    else {
        return Err("the pack's program has no fragment entry point".into());
    };
    let mut outputs = Vec::new();
    if let Some(result) = &vertex_entry.function.result {
        varyings(
            vertex,
            result.ty,
            None,
            result.binding.as_ref(),
            &mut outputs,
        );
    }
    let mut inputs = Vec::new();
    for argument in &fragment_entry.function.arguments {
        varyings(
            pack_fragment,
            argument.ty,
            argument.name.as_deref(),
            argument.binding.as_ref(),
            &mut inputs,
        );
    }
    // The location each of the mod's outputs takes, by its place in `outputs`.
    let mut moved: Vec<Option<u32>> = vec![None; outputs.len()];
    for input in &inputs {
        let Some((name, _)) = pack_varyings.iter().find(|(_, l)| **l == input.location) else {
            return Err(format!(
                "the pack reads varying location {}, which has no GLSL name to match the mod's outputs by",
                input.location
            ));
        };
        let Some(at) = outputs
            .iter()
            .position(|o| o.name.as_deref() == Some(name.as_str()))
        else {
            return Err(format!(
                "the pack reads varying `{name}`, which the mod's vertex stage does not write (outputs are matched by name)"
            ));
        };
        let output = &outputs[at];
        if output.ty != input.ty
            || output.interpolation != input.interpolation
            || output.sampling != input.sampling
        {
            return Err(format!(
                "varying `{name}` differs between the mod's vertex stage and the pack's fragment stage"
            ));
        }
        moved[at] = Some(input.location);
    }
    let mut free = (0u32..).filter(|l| !pack_varyings.values().any(|used| used == l));
    let new_locations: Vec<u32> = moved
        .iter()
        .map(|m| m.unwrap_or_else(|| free.next().unwrap_or(0)))
        .collect();
    let mut linked = vertex.clone();
    linked
        .entry_points
        .retain(|e| e.stage == naga::ShaderStage::Vertex);
    for entry in &mut linked.entry_points {
        entry.name = "main".to_owned();
    }
    if let Some(mut result) = linked
        .entry_points
        .first()
        .and_then(|e| e.function.result.clone())
    {
        renumber(
            &mut linked,
            result.ty,
            &mut result.binding,
            &mut new_locations.into_iter(),
        )?;
        linked.entry_points[0].function.result = Some(result);
    }
    for (_, global) in linked.global_variables.iter_mut() {
        match &mut global.binding {
            Some(binding) => {
                binding.group = match binding.group {
                    0 => MOD_SET_BASE,
                    1 if has_set1 => MOD_SET_BASE + 1,
                    group => {
                        return Err(format!(
                            "the mod's vertex stage binds group {group}, which has no layout in a pack pass"
                        ));
                    }
                };
            }
            None => {
                if !matches!(
                    global.space,
                    naga::AddressSpace::Private | naga::AddressSpace::WorkGroup
                ) {
                    return Err(
                        "the mod's vertex stage uses a global a pack pass cannot bind (push constants)"
                            .into(),
                    );
                }
            }
        }
    }
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(source: &str) -> naga::Module {
        naga::front::wgsl::parse_str(source).expect("test WGSL parses")
    }

    const VERTEX: &str = "
        struct Out {
            @builtin(position) position: vec4<f32>,
            @location(0) color: vec4<f32>,
            @location(1) uv: vec2<f32>,
        };
        struct Globals { scale: vec4<f32> };
        @group(0) @binding(0) var<uniform> globals: Globals;
        @group(1) @binding(0) var<uniform> own: Globals;
        @vertex fn vs(@location(0) p: vec3<f32>) -> Out {
            var o: Out;
            o.position = vec4<f32>(p, 1.0) * globals.scale * own.scale;
            o.color = vec4<f32>(1.0);
            o.uv = p.xy;
            return o;
        }";

    fn fragment(inputs: &str) -> naga::Module {
        module(&format!(
            "@fragment fn fs({inputs}) -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}"
        ))
    }

    fn names(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs.iter().map(|(n, l)| ((*n).to_owned(), *l)).collect()
    }

    /// The linked vertex stage's outputs: name and location.
    fn output_locations(linked: &naga::Module) -> Vec<(String, u32)> {
        let result = linked.entry_points[0].function.result.as_ref().unwrap();
        let mut out = Vec::new();
        varyings(linked, result.ty, None, result.binding.as_ref(), &mut out);
        let mut named: Vec<_> = out
            .into_iter()
            .map(|v| (v.name.unwrap(), v.location))
            .collect();
        named.sort();
        named
    }

    const BOTH: &str = "@location(0) color: vec4<f32>, @location(1) uv: vec2<f32>";

    #[test]
    fn a_matching_interface_links_and_moves_the_mods_sets_after_the_packs() {
        let linked = link_vertex_stage(
            &module(VERTEX),
            &fragment(BOTH),
            &names(&[("color", 0), ("uv", 1)]),
            true,
            8,
        )
        .expect("links");
        let mut groups: Vec<u32> = linked
            .global_variables
            .iter()
            .filter_map(|(_, g)| g.binding.as_ref().map(|b| b.group))
            .collect();
        groups.sort_unstable();
        assert_eq!(groups, [MOD_SET_BASE, MOD_SET_BASE + 1]);
        assert_eq!(linked.entry_points.len(), 1);
        assert_eq!(linked.entry_points[0].name, "main");
    }

    #[test]
    fn outputs_are_matched_by_name_and_renumbered_to_the_packs_locations() {
        // The pack's translator gave `uv` location 0 and `color` location 1: the reverse of the
        // mod's declaration order.
        let linked = link_vertex_stage(
            &module(VERTEX),
            &fragment("@location(1) color: vec4<f32>, @location(0) uv: vec2<f32>"),
            &names(&[("color", 1), ("uv", 0)]),
            true,
            8,
        )
        .expect("links");
        assert_eq!(
            output_locations(&linked),
            [("color".to_owned(), 1), ("uv".to_owned(), 0)]
        );
    }

    #[test]
    fn an_output_the_pack_does_not_read_moves_off_the_locations_the_pack_uses() {
        // The pack reads only `color`, at location 1; its location 0 belongs to a varying the
        // fragment stage does not take, and the mod's `uv` must not land on either.
        let linked = link_vertex_stage(
            &module(VERTEX),
            &fragment("@location(1) color: vec4<f32>"),
            &names(&[("color", 1), ("other", 0)]),
            true,
            8,
        )
        .expect("links");
        assert_eq!(
            output_locations(&linked),
            [("color".to_owned(), 1), ("uv".to_owned(), 2)]
        );
    }

    #[test]
    fn a_missing_mistyped_or_unnamed_varying_is_refused() {
        let missing = link_vertex_stage(
            &module(VERTEX),
            &fragment("@location(2) normal: vec3<f32>"),
            &names(&[("normal", 2)]),
            true,
            8,
        );
        assert!(missing.unwrap_err().contains("`normal`"));
        let mistyped = link_vertex_stage(
            &module(VERTEX),
            &fragment("@location(1) uv: vec3<f32>"),
            &names(&[("uv", 1)]),
            true,
            8,
        );
        assert!(mistyped.unwrap_err().contains("`uv`"));
        let unnamed = link_vertex_stage(
            &module(VERTEX),
            &fragment("@location(3) hidden: vec4<f32>"),
            &names(&[("color", 0)]),
            true,
            8,
        );
        assert!(unnamed.unwrap_err().contains("no GLSL name"));
    }

    #[test]
    fn bind_groups_over_the_device_limit_are_refused() {
        let fragment = fragment("@location(0) color: vec4<f32>");
        let pack = names(&[("color", 0)]);
        assert!(link_vertex_stage(&module(VERTEX), &fragment, &pack, true, 7).is_err());
        assert!(link_vertex_stage(&module(VERTEX), &fragment, &pack, true, 8).is_ok());
    }

    #[test]
    fn a_set_the_mod_did_not_declare_is_refused() {
        let fragment = fragment("@location(0) color: vec4<f32>");
        let pack = names(&[("color", 0)]);
        let error = link_vertex_stage(&module(VERTEX), &fragment, &pack, false, 8).unwrap_err();
        assert!(error.contains("group 1"));
    }
}
