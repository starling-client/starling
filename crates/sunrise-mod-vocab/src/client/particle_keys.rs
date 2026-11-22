//! Load-owned particle identities and the resolved provider table.
use super::particle_types::{ParticleCodec, ParticleRegistryView, TypedParticleOptions};
use crate::Compound;
use std::sync::OnceLock;
use sunrise_mod_runtime::registry_keys::{ParticleKind, Particles};

/// A particle requested at registration, resolved once when providers load.
// no wire: decoding validates the u32 slot with ParticleKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the load-owned table creates keys
pub struct ParticleKey(pub(crate) u32);

fn resolved() -> &'static OnceLock<Particles> {
    static EMPTY: OnceLock<Particles> = OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |parts| &parts.particles)
}
impl ParticleKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::Particle,
            slot,
        )
        .unwrap_or_else(|| {
            resolved()
                .get()
                .map_or(&crate::parts::requested_keys().particles, |p| &p.names)
                .id(slot)
                .is_some()
        });
        known
            .then_some(Self(slot))
            .ok_or("unknown ParticleKey slot")
    }
}

/// Whether the immutable provider table has already been published for these Parts.
// hidden: the owning content loader retains this process-lifetime table across pack reloads.
#[doc(hidden)]
pub fn particles_resolved() -> bool {
    resolved().get().is_some()
}

/// Resolve provider names once at load, before publishing either names or providers.
// hidden: only the game supplies the completed particle registry.
#[doc(hidden)]
pub fn resolve_particles<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<ParticleKind>,
) -> Result<(), Vec<String>> {
    let requested = &crate::parts::requested_keys().particles;
    let installed = crate::parts::installed();
    let declared = requested
        .entries
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            installed?
                .keys
                .particle_types
                .contains_key(&ParticleKey(slot as u32))
                .then_some(entry.id.as_str())
        });
    let particles = sunrise_mod_runtime::registry_keys::resolve_particles(
        requested,
        ids.into_iter().chain(declared),
        resolve,
    )?;
    resolved()
        .set(particles)
        .map_err(|_| vec!["particle registry already resolved".to_owned()])
}

/// Find a registered particle without allocating or parsing a name.
// hidden: generated metadata forwarders call this lookup at the owning load boundary.
#[doc(hidden)]
pub fn particle_key(id: &crate::Identifier) -> Option<ParticleKey> {
    Some(ParticleKey(*resolved().get()?.names.identifiers.get(id)?))
}

/// Map an engine vanilla provider index without a name lookup or allocation.
// hidden: only the game's conversion boundary knows engine provider indices.
#[doc(hidden)]
pub fn particle_vanilla_key(index: u32) -> Option<ParticleKey> {
    Some(ParticleKey(*resolved().get()?.vanilla_slots.get(&index)?))
}
/// Map an already retained custom provider name at an engine capture boundary.
// hidden: the game's capture boundary reads already retained engine provider names.
#[doc(hidden)]
pub fn particle_named_key(name: &str) -> Option<ParticleKey> {
    Some(ParticleKey(*resolved().get()?.names.slots.get(name)?))
}

/// Read the already-resolved provider identity for a retained key.
// hidden: the game's particle conversion boundary consumes the provider index.
#[doc(hidden)]
pub fn particle_kind(key: ParticleKey) -> Option<ParticleKind> {
    resolved().get()?.kinds.get(key.0 as usize).copied()
}

/// Borrow a custom provider's name; no owned string is created.
// hidden: the game's particle conversion boundary sends custom provider names.
#[doc(hidden)]
pub fn particle_name(key: ParticleKey) -> Option<&'static str> {
    resolved().get()?.names.id(key.0)
}

// hidden: the command registry enumerates the game's resolved particle IDs.
#[doc(hidden)]
pub fn particle_ids() -> impl Iterator<Item = &'static str> {
    resolved()
        .get()
        .into_iter()
        .flat_map(|p| p.names.order.iter().filter_map(|&slot| p.names.id(slot)))
}

// hidden: the renderer reads the limiter policy when it resolves a typed request.
#[doc(hidden)]
pub fn particle_override_limiter(key: ParticleKey) -> Option<bool> {
    crate::parts::installed()?
        .keys
        .particle_types
        .get(&key)
        .map(|(flag, _)| *flag)
}

fn particle_codec(key: ParticleKey) -> Option<&'static dyn ParticleCodec> {
    Some(
        crate::parts::installed()?
            .keys
            .particle_types
            .get(&key)?
            .1
            .as_ref(),
    )
}

/// Decode the fields of a named, codec-bearing type. `None` means this name has no typed codec.
// hidden: the game's structured and command dispatch select a registered codec.
#[doc(hidden)]
pub fn decode_particle_map(
    name: &str,
    fields: &Compound,
) -> Option<Result<TypedParticleOptions, String>> {
    let key = particle_named_key(name)?;
    Some(particle_codec(key)?.decode_map(key, fields, &ParticleRegistryView::installed()))
}

/// Encode a structured value, selecting its registered `type` at this boundary.
// hidden: the game's structured dispatcher writes the type field.
#[doc(hidden)]
pub fn encode_particle_map(value: &TypedParticleOptions) -> Result<Compound, String> {
    let key = value.key();
    let (Some(name), Some(codec)) = (particle_name(key), particle_codec(key)) else {
        return Err("unregistered particle type".into());
    };
    let mut fields = codec.encode_map(value, &ParticleRegistryView::installed())?;
    fields.insert("type".into(), crate::Tag::String(name.into()));
    Ok(fields)
}

/// Decode a Sunrise particle envelope. An asset-only type has no stream fields.
// hidden: the play connection dispatches this envelope before spawning.
#[doc(hidden)]
pub fn decode_particle_stream(
    name: &str,
    bytes: &[u8],
) -> Result<Option<TypedParticleOptions>, String> {
    let key = particle_named_key(name).ok_or("unregistered particle type")?;
    if particle_kind(key) != Some(ParticleKind::Custom) {
        return Err("not a custom particle type".into());
    }
    match particle_codec(key) {
        Some(codec) => codec
            .decode_stream(key, bytes, &ParticleRegistryView::installed())
            .map(Some),
        None if bytes.is_empty() => Ok(None),
        None => Err("unexpected particle option bytes for an asset-only type".into()),
    }
}

// hidden: the play connection encodes one registered payload for the packet.
#[doc(hidden)]
pub fn encode_particle_stream(
    value: &TypedParticleOptions,
    limit: usize,
) -> Result<Vec<u8>, String> {
    particle_codec(value.key())
        .ok_or("unregistered particle type")?
        .encode_stream(value, &ParticleRegistryView::installed(), limit)
}
