//! Compute pipelines mods register (`RegisterRenderPipelinesEvent::compute_pipeline`): the
//! registrations, kept once at start, and what each resource load makes of them. A load reads
//! every registered pipeline's GLSL compute shader (`assets/<namespace>/shaders/compute/<path>.csh`,
//! resolving `#include <ns:path>` and `#include "relative"` through the active resource stack),
//! translates it through [`sunrise_mod_pipeline::translate_compute`] and hands validated naga
//! modules to the renderer (`gfx::mod_compute`), which checks them against its device's limits
//! and features. A compute pipeline the load cannot build is logged and resolves to
//! [`ComputePipelineError::Invalid`]; it never fails the load, as no frame depends on it.

use crate::mod_pipelines::RegistrationError;
use mod_api::{ComputePipeline, ComputePipelineError, ComputePipelineRef, Identifier};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use sunrise_mod_pipeline::{
    COMPUTE_FOLDER, ComputeOptions, ComputeSources, DEVICELESS_INTERFACE_LIMITS,
};

static REGISTERED: OnceLock<Vec<ComputePipeline>> = OnceLock::new();

/// Validate and publish the mods' compute registrations once at startup: each ID at most once
/// and each pipeline's defines and bind-group layout well formed. A pipeline's place in the
/// list is its slot.
pub fn register(pipelines: Vec<ComputePipeline>) -> Result<(), RegistrationError> {
    let mut seen = BTreeMap::new();
    for pipeline in &pipelines {
        if let Some(earlier) = seen.insert(&pipeline.id, pipeline.owner()) {
            return Err(RegistrationError::DuplicateModId {
                earlier: earlier.to_owned(),
                owner: pipeline.owner().to_owned(),
                id: pipeline.id.clone(),
            });
        }
        if let Err(reason) = pipeline.validate() {
            return Err(RegistrationError::InvalidComputePipeline(format!(
                "mod {}: compute pipeline {}: {reason}",
                pipeline.owner(),
                pipeline.id
            )));
        }
    }
    REGISTERED
        .set(pipelines)
        .map_err(|_| RegistrationError::AlreadyPublished)
}

/// The published compute registrations, empty before startup registration.
pub fn registered() -> &'static [ComputePipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

/// One compute shader a load translated.
#[derive(Clone, Debug)]
pub struct BuiltCompute {
    /// The owning mod, not necessarily the pipeline namespace.
    pub owner: String,
    pub id: Identifier,
    pub shader: Arc<sunrise_glsl::ComputeTranslated>,
    pub bind_group_layout: mod_api::BindGroupLayout,
}

/// Why a device refused a compute pipeline its load had translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Over a device limit, or the device rejected the module.
    Invalid,
    /// Needs a feature the adapter lacks (subgroup operations).
    UnsupportedFeature,
}

/// The registered compute pipelines as one resource load built them, by slot.
#[derive(Clone, Debug, Default)]
pub struct ComputeSet {
    keys: Vec<Identifier>,
    built: Vec<Option<BuiltCompute>>,
    generation: u64,
    refused: Arc<Mutex<Vec<(usize, Refusal)>>>,
}

impl ComputeSet {
    /// Translate every registered pipeline, reading `.csh` and include files via `read`
    /// (`Ok(None)` when absent). A refused shader is logged with its reason and left out.
    pub(crate) fn build(
        registered: &[ComputePipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<Option<String>, String> + Sync,
    ) -> Self {
        let include = |id: &str| -> Result<Option<String>, String> {
            let Some((ns, rest)) = id.split_once(':') else {
                return Ok(None);
            };
            read(&format!("assets/{ns}/{rest}"))
        };
        let built = registered
            .iter()
            .map(|pipeline| {
                let loaded = Self::translate(pipeline, &read, &include);
                match loaded {
                    Ok(built) => Some(built),
                    Err(reason) => {
                        log::warn!(
                            "Sunrise: mod {} compute pipeline {} refused: {reason}",
                            pipeline.owner(),
                            pipeline.id
                        );
                        None
                    }
                }
            })
            .collect();
        Self {
            keys: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            refused: Default::default(),
        }
    }

    fn translate(
        pipeline: &ComputePipeline,
        read: &(impl Fn(&str) -> Result<Option<String>, String> + Sync),
        include: &(dyn Fn(&str) -> Result<Option<String>, String> + Sync),
    ) -> Result<BuiltCompute, String> {
        let file = format!(
            "assets/{}/shaders/compute/{}.csh",
            pipeline.shader.namespace(),
            pipeline.shader.path()
        );
        let source = read(&file)
            .map_err(|cause| format!("{file}: {cause}"))?
            .ok_or_else(|| format!("{file}: not found"))?;
        let path = pipeline.shader.path();
        let folder = path.rfind('/').map_or("", |i| &path[..=i]);
        let dir = format!("{}:{COMPUTE_FOLDER}/{folder}", pipeline.shader.namespace());
        let defines: Vec<String> = pipeline
            .defines
            .iter()
            .map(|(name, value)| match value {
                Some(v) => format!("{name} {v}"),
                None => name.clone(),
            })
            .collect();
        let define_refs: Vec<&str> = defines.iter().map(String::as_str).collect();
        let mod_bindings =
            crate::mod_pipelines::mod_bindings_from_layout(&pipeline.bind_group_layout);
        let translated = sunrise_mod_pipeline::translate_compute(
            &ComputeSources {
                file: &file,
                dir: &dir,
                source: &source,
            },
            &ComputeOptions {
                defines: &define_refs,
                mod_bindings: &mod_bindings,
                check_backends: false,
                interface: DEVICELESS_INTERFACE_LIMITS,
            },
            include,
        )
        .map_err(|problems| {
            problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        Ok(BuiltCompute {
            owner: pipeline.owner().to_owned(),
            id: pipeline.id.clone(),
            shader: Arc::new(translated),
            bind_group_layout: pipeline.bind_group_layout.clone(),
        })
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<BuiltCompute>] {
        &self.built
    }

    /// Marks `slot` as one the device refused, for `reason`; false when it already was, so it is
    /// reported once.
    pub(crate) fn refuse(&self, slot: usize, reason: Refusal) -> bool {
        let mut refused = self.refused.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = !refused.iter().any(|(s, _)| *s == slot);
        if fresh {
            refused.push((slot, reason));
        }
        fresh
    }

    fn refusal(&self, slot: usize) -> Option<Refusal> {
        self.refused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find_map(|(s, reason)| (*s == slot).then_some(*reason))
    }

    fn error_of(&self, slot: usize) -> Option<ComputePipelineError> {
        match self.refusal(slot) {
            Some(Refusal::UnsupportedFeature) => Some(ComputePipelineError::UnsupportedFeature),
            Some(Refusal::Invalid) => Some(ComputePipelineError::Invalid),
            None => None,
        }
    }

    /// The handle of the registered compute pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<ComputePipelineRef, ComputePipelineError> {
        let slot = self
            .keys
            .iter()
            .position(|registered| registered == id)
            .ok_or(ComputePipelineError::Unknown)?;
        if self.built[slot].is_none() {
            return Err(ComputePipelineError::Invalid);
        }
        if let Some(error) = self.error_of(slot) {
            return Err(error);
        }
        Ok(mod_api::__private::built::compute_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle dispatches with, checked once when a dispatch is recorded.
    pub fn slot(&self, pipeline: ComputePipelineRef) -> Result<u32, ComputePipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(ComputePipelineError::Stale);
        }
        if let Some(error) = self.error_of(fields.slot as usize) {
            return Err(error);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(ComputePipelineError::Invalid),
            None => Err(ComputePipelineError::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    const SHADER: &str = "#version 450\nlayout(local_size_x = 8, local_size_y = 8) in;\nlayout(std430) buffer Counts { uint counts[]; };\nvoid main() { counts[gl_GlobalInvocationID.x] = 1u; }\n";

    fn pipeline(name: &str) -> ComputePipeline {
        let layout = mod_api::BindGroupLayout::builder()
            .entry(mod_api::BindEntry::new(
                "Counts",
                mod_api::BindingKind::StorageBuffer {
                    read_only: false,
                    min_size: 4,
                },
            ))
            .build()
            .unwrap();
        ComputePipeline::builder(id(name), id(name))
            .bind_group_layout(layout)
            .build()
    }

    fn read(file: &str) -> Result<Option<String>, String> {
        Ok((file == "assets/mine/shaders/compute/count.csh").then(|| SHADER.to_owned()))
    }

    #[test]
    fn a_load_translates_what_it_finds_and_resolves_the_rest_invalid() {
        let set = ComputeSet::build(&[pipeline("mine:count"), pipeline("mine:gone")], 3, read);
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(built.shader.workgroup_size, [8, 8, 1]);
        assert!(set.built()[1].is_none());
        let handle = set.resolve(&id("mine:count")).unwrap();
        assert_eq!(set.slot(handle), Ok(0));
        assert_eq!(
            set.resolve(&id("mine:gone")),
            Err(ComputePipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("mine:other")),
            Err(ComputePipelineError::Unknown)
        );
    }

    #[test]
    fn a_handle_of_an_earlier_load_is_stale_and_a_device_refusal_shows() {
        let first = ComputeSet::build(&[pipeline("mine:count")], 1, read);
        let handle = first.resolve(&id("mine:count")).unwrap();
        let second = ComputeSet::build(&[pipeline("mine:count")], 2, read);
        assert_eq!(second.slot(handle), Err(ComputePipelineError::Stale));
        assert!(second.refuse(0, Refusal::UnsupportedFeature));
        assert!(!second.refuse(0, Refusal::Invalid));
        assert_eq!(
            second.resolve(&id("mine:count")),
            Err(ComputePipelineError::UnsupportedFeature)
        );
    }

    #[test]
    fn a_shader_that_declares_an_interface_is_refused() {
        let bad = "#version 450\nlayout(local_size_x = 1) in;\nout vec4 colour;\nvoid main() {}\n";
        let set = ComputeSet::build(&[pipeline("mine:count")], 1, |_| Ok(Some(bad.to_owned())));
        assert!(set.built()[0].is_none());
    }
}
