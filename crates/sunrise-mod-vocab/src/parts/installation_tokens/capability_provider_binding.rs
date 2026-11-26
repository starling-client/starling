//! Erased provider metadata retained behind the native binding.
use std::any::TypeId;

/// Identity of the erased callback's concrete `Arc` type (including context) and the handler
/// name for diagnostics. Only the identity is used to compare bindings.
pub fn capability_provider_binding(
    provider: &crate::NativeCapabilityProvider,
) -> (TypeId, &'static str) {
    provider.binding()
}
