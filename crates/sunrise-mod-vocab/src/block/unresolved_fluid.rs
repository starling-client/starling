//! Fluid names retained from content that is not installed.
use crate::{FluidKey, Identifier, UnresolvedFluid};
/// A registered fluid, or a saved name whose fluid is not registered in this game.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum FluidIdentity {
    /// A registered fluid.
    Resolved(FluidKey),
    /// A name retained verbatim until its declaring mod returns.
    Unresolved(UnresolvedFluid),
}

impl FluidIdentity {
    /// Its registered key, or none when the fluid is unresolved.
    pub fn key(&self) -> Option<FluidKey> {
        match self {
            Self::Resolved(key) => Some(*key),
            Self::Unresolved(_) => None,
        }
    }

    /// The registered key a saved name resolves to, or the name itself when no fluid has it.
    /// `minecraft:empty`, a bare name meaning `minecraft:`, and invalid names follow
    /// [`UnresolvedFluid::new`]; None for the empty fluid or an invalid name.
    pub fn of_name(id: &str) -> Option<Self> {
        let id = Identifier::parse(id).ok()?;
        if id.as_str() == "minecraft:empty" {
            return None;
        }
        Some(
            match crate::content::registry_keys::fluid_key(id.as_str()) {
                Some(key) => Self::Resolved(key),
                None => Self::Unresolved(UnresolvedFluid(id)),
            },
        )
    }
}

impl UnresolvedFluid {
    /// `id` as an unresolved fluid: None when a registered fluid has that name, or for
    /// `minecraft:empty`, which a saved stack never names (NeoForge's `FluidStack.CODEC` refuses
    /// it).
    pub(crate) fn new(id: Identifier) -> Option<Self> {
        (id.as_str() != "minecraft:empty"
            && crate::content::registry_keys::fluid_key(id.as_str()).is_none())
        .then_some(Self(id))
    }

    /// The saved name.
    pub fn id(&self) -> &Identifier {
        &self.0
    }
}

/// Compare an identity with an NBT name without allocating; a bare name means `minecraft:`.
pub fn matches_utf16(identity: &FluidIdentity, id: &[u16]) -> bool {
    match identity {
        FluidIdentity::Resolved(key) => {
            crate::content::registry_keys::fluid_key_utf16(id) == Some(*key)
        }
        FluidIdentity::Unresolved(name) => {
            let spelled = |text: &str| text.encode_utf16().eq(id.iter().copied());
            spelled(name.0.as_str()) || name.0.namespace() == "minecraft" && spelled(name.0.path())
        }
    }
}

/// The saved name of an identity, at an engine-owned boundary.
pub fn identity_name(identity: &FluidIdentity) -> String {
    match identity {
        FluidIdentity::Resolved(key) => crate::content::registry_keys::fluid_id(*key)
            .expect("resolved fluid key")
            .to_owned(),
        FluidIdentity::Unresolved(name) => name.0.as_str().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, WireError, Writer};

    #[test]
    fn an_unresolved_name_crosses_wire_as_its_name_and_empty_is_refused() {
        let FluidIdentity::Unresolved(absent) = FluidIdentity::of_name("absent:fluid").unwrap()
        else {
            panic!("no fluid registry is installed, so the name stays unresolved");
        };
        let mut out = Writer::new();
        out.put(&absent);
        assert_eq!(Reader::decode::<UnresolvedFluid>(out.bytes()), Ok(absent));

        out.clear();
        out.put(&Identifier::parse("minecraft:empty").unwrap());
        assert_eq!(
            Reader::decode::<UnresolvedFluid>(out.bytes()),
            Err(WireError::Invalid(
                "an unresolved fluid names a registered or empty fluid"
            ))
        );
        assert_eq!(FluidIdentity::of_name("empty"), None);
        assert_eq!(FluidIdentity::of_name("bad name"), None);
    }

    #[test]
    fn a_bare_nbt_name_matches_its_minecraft_spelling() {
        let identity = FluidIdentity::of_name("unavailable").unwrap();
        let utf16 = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        assert!(matches_utf16(&identity, &utf16("unavailable")));
        assert!(matches_utf16(&identity, &utf16("minecraft:unavailable")));
        assert!(!matches_utf16(&identity, &utf16("other:unavailable")));
    }
}
