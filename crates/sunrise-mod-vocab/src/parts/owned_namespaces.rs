// exports: none (reached inside the crate only)
use crate::{
    __private::{DeclarationKind, DeclarationRefusal},
    Answered, DeclarationError,
};
use std::{collections::HashMap, sync::Arc};

/// Pre-resolved namespace ownership table derived once from the package inventory at load.
// hidden: engine- and vocab-internal indexed authority, not a mod authoring surface.
// no wire: process-local lookup table derived from `PackageInfo` at load.
#[doc(hidden)]
#[derive(Clone, Default)]
// exhaustive: internal lookup table with private fields.
pub struct NamespaceAuthority {
    /// Non-primary and delegated namespaces -> owning package IDs.
    /// Every package always owns its own ID without needing a map entry.
    owners: HashMap<Box<str>, Box<[Box<str>]>>,
    /// Package ID -> every non-primary namespace it owns (extra manifest namespaces and,
    /// for `PackageKind::Code` hosts, served content namespaces).
    owned_by: HashMap<Box<str>, Box<[Box<str>]>>,
}

impl NamespaceAuthority {
    /// Build the indexed authority table once from a package inventory snapshot.
    pub fn from_packages(packages: &[crate::PackageInfo]) -> Self {
        let mut first_kind: HashMap<&str, crate::PackageKind> =
            HashMap::with_capacity(packages.len());
        let mut owners: HashMap<Box<str>, Vec<Box<str>>> = HashMap::new();
        let mut owned_by: HashMap<Box<str>, Vec<Box<str>>> = HashMap::new();
        let mut add = |owner: &str, ns: &str| {
            if owner == ns {
                return;
            }
            let owner_list = owners.entry(Box::from(ns)).or_default();
            if !owner_list.iter().any(|existing| existing.as_ref() == owner) {
                owner_list.push(Box::from(owner));
            }
            let ns_list = owned_by.entry(Box::from(owner)).or_default();
            if !ns_list.iter().any(|existing| existing.as_ref() == ns) {
                ns_list.push(Box::from(ns));
            }
        };
        for package in packages {
            let id = package.id();
            if first_kind.contains_key(id) {
                continue;
            }
            first_kind.insert(id, package.kind());
            for ns in package.namespaces() {
                add(id, ns);
            }
        }
        for content in packages {
            if content.kind() != crate::PackageKind::Content {
                continue;
            }
            let Some(host) = content.served_by() else {
                continue;
            };
            if first_kind.get(host) != Some(&crate::PackageKind::Code) {
                continue;
            }
            for ns in content.namespaces() {
                if matches!(ns, "minecraft" | "c" | "sunrise") {
                    continue;
                }
                add(host, ns);
            }
        }
        Self {
            owners: owners
                .into_iter()
                .map(|(ns, list)| (ns, list.into_boxed_slice()))
                .collect(),
            owned_by: owned_by
                .into_iter()
                .map(|(owner, list)| (owner, list.into_boxed_slice()))
                .collect(),
        }
    }

    /// Whether `owner` has authority over `namespace`, in O(1) without scanning packages.
    #[inline]
    pub fn owns(&self, owner: &str, namespace: &str) -> bool {
        owner == namespace
            || self
                .owners
                .get(namespace)
                .is_some_and(|list| list.iter().any(|known| known.as_ref() == owner))
    }

    /// Every namespace `owner` has authority over (its own ID first, then owned/served namespaces).
    pub fn namespaces_for<'a>(&'a self, owner: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        std::iter::once(owner).chain(
            self.owned_by
                .get(owner)
                .into_iter()
                .flat_map(|list| list.iter().map(Box::as_ref)),
        )
    }
}

/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID,
/// and a code mod also owns the namespaces of any installed content package
/// whose manifest declares `served_by = "<owner>"`.
pub(crate) fn owns_namespace_in(
    packages: &crate::LoadedMods,
    owner: &str,
    namespace: &str,
) -> bool {
    packages.authority.owns(owner, namespace)
}

/// The frozen indexed namespace authority for the current package inventory.
pub(crate) fn current_namespace_authority() -> Arc<NamespaceAuthority> {
    #[cfg(test)]
    if let Some(authority) = crate::world::phase::test_namespace_authority() {
        return authority;
    }
    crate::dispatch_scope::backend()
        .map(|backend| (backend.namespace_authority)())
        .unwrap_or_default()
}

/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID.
pub fn owns_namespace(owner: &str, namespace: &str) -> bool {
    if owner == namespace {
        return true;
    }
    #[cfg(test)]
    if let Some(owns) = crate::world::phase::test_owns_namespace(owner, namespace) {
        return owns;
    }
    crate::dispatch_scope::backend()
        .is_some_and(|backend| (backend.owns_namespace)(owner, namespace))
}

/// `name` with its namespace: a bare name is in mod `owner`'s own, for a declared id and
/// for the block and item references a declaration names (`block_item`, `pick_item`,
/// `wall_block`, `base_block`, `crafting_remainder`, `Item::block`) alike.
pub fn qualified(owner: &str, name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{owner}:{name}")
    }
}

/// Validate and qualify a declaration using the package namespace inventory.
pub fn owned_declaration_id(owner: &str, name: &str) -> Result<String, String> {
    validate_owned_id(owner, qualified(owner, name))
        .map_err(|error| format!("mod {owner} declares {error}"))
}

fn validate_owned_id(owner: &str, id: String) -> Result<String, DeclarationError> {
    let Some(parsed) = crate::Identifier::parse_namespaced(&id) else {
        return Err(DeclarationError::InvalidId { id });
    };
    let namespace = parsed.namespace();
    if !owns_namespace(owner, namespace) {
        return Err(DeclarationError::UnownedNamespace {
            id,
            namespace: namespace.to_owned(),
        });
    }
    Ok(id)
}

/// A declaration a mod's `blocks` or `items` hook adds: its id, and the names of other
/// content it references.
pub(crate) trait Declaration {
    const KIND: DeclarationKind;

    /// The declared id and its optional references.
    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>);
}

impl Declaration for crate::Block {
    const KIND: DeclarationKind = DeclarationKind::Block;

    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [
                &mut self.block_item,
                &mut self.pick_item,
                &mut self.wall_block,
                &mut self.base_block,
            ],
        )
    }
}

impl Declaration for crate::Item {
    const KIND: DeclarationKind = DeclarationKind::Item;

    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [&mut self.crafting_remainder, &mut self.block],
        )
    }
}

// no wire: in-process closures that stage one listener's declarations.
/// The owner validates each submission immediately and publishes only an accepted batch.
// hidden: registrars use this callback to validate or commit an owner’s staged batch.
#[doc(hidden)]
pub type DeclarationStage<'a, D> =
    dyn FnMut(&mut dyn FnMut(&mut DeclarationSink<'_, D>) -> bool) + 'a;

// no wire: an in-process closure borrowing the current declaration batch.
/// The validated submission callback for one listener.
// hidden: only declaration registrars use this validated ingress callback.
#[doc(hidden)]
pub type DeclarationSink<'a, D> = dyn FnMut(Answered<D>) -> Result<(), DeclarationError> + 'a;

/// Validate at the call site and keep the dispatch owner with the declaration.
pub(crate) fn submit_declaration<D: Declaration>(
    refusals: &mut Vec<DeclarationRefusal>,
    mod_id: &'static str,
    submit: &mut DeclarationSink<'_, D>,
    mut declaration: D,
) -> Result<(), DeclarationError> {
    if !crate::registration_open() {
        let error = DeclarationError::RegistrationClosed {
            id: declaration.names().0.clone(),
        };
        log::warn!("mod {mod_id} {:?} declaration refused: {error}", D::KIND);
        return Err(error);
    }
    let result = (|| {
        let (id, references) = declaration.names();
        *id = validate_owned_id(mod_id, qualified(mod_id, id))?;
        for reference in references.into_iter().flatten() {
            *reference = qualified(mod_id, reference);
        }
        submit(Answered {
            answer: declaration,
            mod_id,
        })
    })();
    result.inspect_err(|error| {
        refusals.push(DeclarationRefusal::new(mod_id, D::KIND, error.clone()));
    })
}

/// Commit the diagnostics only for a listener whose declaration proposal was accepted.
pub(crate) fn declaration_listener(
    context: &crate::RegistrationContext,
    run: impl FnOnce(&mut Vec<DeclarationRefusal>) -> bool,
) -> bool {
    let mut refusals = Vec::new();
    let accepted = run(&mut refusals);
    if accepted && !refusals.is_empty() {
        context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(refusals);
    }
    accepted
}

/// The engine consumes block/item declaration ownership refusals before loading
/// resources.
// hidden: engine admission plumbing, re-exported through __private for the loader.
#[doc(hidden)]
pub fn take_declaration_refusals(context: &crate::RegistrationContext) -> Vec<DeclarationRefusal> {
    std::mem::take(
        &mut *context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use crate::{PackageInfo, PackageKind, Side};

    #[test]
    fn namespace_authority_indexes_code_and_served_namespaces_with_reserved_guard() {
        let packages = [
            PackageInfo::new(
                "horizon_java".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["horizon_java".into(), "horizon_extra".into()],
                vec![],
            ),
            PackageInfo::new(
                "runes".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec![
                    "runes".into(),
                    "runes_extra".into(),
                    "minecraft".into(),
                    "c".into(),
                    "sunrise".into(),
                ],
                vec![],
            )
            .with_served_by(Some("horizon_java".into())),
            PackageInfo::new(
                "plain_host".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["plain_host".into()],
                vec![],
            ),
            PackageInfo::new(
                "bad_served".into(),
                "1".into(),
                PackageKind::Content,
                Side::Both,
                vec!["bad_served".into()],
                vec![],
            )
            .with_served_by(Some("plain_host".into())),
        ];
        let authority = NamespaceAuthority::from_packages(&packages);
        assert!(authority.owns("horizon_java", "horizon_java"));
        assert!(authority.owns("horizon_java", "horizon_extra"));
        assert!(authority.owns("horizon_java", "runes"));
        assert!(authority.owns("horizon_java", "runes_extra"));
        for reserved in ["minecraft", "c", "sunrise"] {
            assert!(!authority.owns("horizon_java", reserved), "{reserved}");
        }
        assert!(!authority.owns("plain_host", "bad_served"));
        assert!(!authority.owns("third", "runes"));
        assert!(authority.owns("third", "third"));
        let namespaces: Vec<_> = authority.namespaces_for("horizon_java").collect();
        assert_eq!(
            namespaces,
            ["horizon_java", "horizon_extra", "runes", "runes_extra"]
        );
    }
}
