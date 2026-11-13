//! Allocation-free UTF-8 lookup of lossless UTF-16 attachment keys.
use super::index::AttachmentKey;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub(super) enum Units<'a> {
    Saved(std::slice::Iter<'a, u16>),
    Named(std::str::EncodeUtf16<'a>),
}
impl Iterator for Units<'_> {
    type Item = u16;
    fn next(&mut self) -> Option<u16> {
        match self {
            Self::Saved(it) => it.next().copied(),
            Self::Named(it) => it.next(),
        }
    }
}
pub(super) trait Lookup {
    fn units(&self) -> (Units<'_>, Units<'_>);
}
impl Lookup for AttachmentKey {
    fn units(&self) -> (Units<'_>, Units<'_>) {
        (Units::Saved(self.0.0.iter()), Units::Saved(self.1.0.iter()))
    }
}
pub(super) struct Named<'a>(pub &'a str, pub &'a str);
impl Lookup for Named<'_> {
    fn units(&self) -> (Units<'_>, Units<'_>) {
        (
            Units::Named(self.0.encode_utf16()),
            Units::Named(self.1.encode_utf16()),
        )
    }
}
impl PartialEq for dyn Lookup + '_ {
    fn eq(&self, other: &Self) -> bool {
        let (namespace, name) = self.units();
        let (other_namespace, other_name) = other.units();
        namespace.eq(other_namespace) && name.eq(other_name)
    }
}
impl Eq for dyn Lookup + '_ {}
impl Hash for dyn Lookup + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (namespace, name) = self.units();
        // Option tags delimit each component even if it contains zero code units.
        for unit in namespace {
            Some(unit).hash(state);
        }
        None::<u16>.hash(state);
        for unit in name {
            Some(unit).hash(state);
        }
        None::<u16>.hash(state);
    }
}
#[derive(Clone)]
pub(super) struct BucketKey(pub Arc<AttachmentKey>);
impl<'a> Borrow<dyn Lookup + 'a> for BucketKey {
    fn borrow(&self) -> &(dyn Lookup + 'a) {
        self.0.as_ref()
    }
}
impl PartialEq for BucketKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for BucketKey {}
impl Hash for BucketKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.as_ref() as &dyn Lookup).hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_equality_checks_both_components_and_lossless_units() {
        let key: AttachmentKey = ("namespace".into(), "name".into());
        let lookup = &key as &dyn Lookup;
        assert!(lookup == &Named("namespace", "name") as &dyn Lookup);
        assert!(lookup != &Named("namespace", "different") as &dyn Lookup);
        assert!(lookup != &Named("different", "name") as &dyn Lookup);
        let unpaired = (horizon_core::nbt::JavaString(vec![0xd800]), "name".into());
        assert!(&unpaired as &dyn Lookup != &Named("\u{fffd}", "name") as &dyn Lookup);
        let same_units = (horizon_core::nbt::JavaString(vec![0xd800]), "name".into());
        assert!(&unpaired as &dyn Lookup == &same_units as &dyn Lookup);
    }
}
