//! Compatibility for native packages whose `uses.json` predates the schema 3 hash recipe.
//! The former method/function hashes included a WebAssembly eligibility bit that did not
//! affect native callers. Accept either old value, and defer vanished records and impls with
//! a missing API operand: the old scan could falsely attribute a mod's own names to the API.
//! Deferred entries do not refuse fit. The fit tools list them; the native compile decides
//! whether the mod actually used them. A changed contract still refuses fit.
//! Delete this module and its callers once no published registry package carries a schema 2 or older list.
//!
//! A schema 2 surface hashes its method and function records by the old recipe, and its other
//! records as schema 3 does. [`rehash`] reads one as schema 3: a mod pinned to a build whose
//! SDK carries one can still be checked, repinned and released. That goes with the line's last
//! such build, not with the old lists.

use crate::{
    FunctionContract, MethodContract, SCHEMA, Surface, contract_hash,
    uses::{Entry, UsesTable},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The surface schema of the old recipe.
pub(crate) const OLD_RECIPE_SURFACE: u32 = 2;

/// Gives a schema 2 document's method, function and handle records the hash schema 3's writer
/// gives the same contract, and marks it schema 3. A stored hash the old recipe does not give its
/// contract is kept, so [`Surface::unsealed`] still names it. The document's own `hash` follows its
/// records when it was theirs.
pub(crate) fn rehash(surface: &mut Surface) {
    let sealed = surface.hash == surface.document_hash();
    for method in surface.traits.iter_mut().flat_map(|t| &mut t.methods) {
        if Contract::Method(&method.contract).old_recipe(&method.hash) {
            method.hash = contract_hash(&method.contract);
        }
    }
    for function in surface.functions.iter_mut().chain(&mut surface.handles) {
        if Contract::Function(&function.contract).old_recipe(&function.hash) {
            function.hash = contract_hash(&function.contract);
        }
    }
    surface.schema = SCHEMA;
    if sealed {
        surface.hash = surface.document_hash();
    }
}

pub(crate) struct LegacyFit<'a>(Option<Old<'a>>);

struct Old<'a> {
    hashes: BTreeMap<&'a str, Contract<'a>>,
    known_types: BTreeSet<&'a str>,
    current_types: BTreeSet<&'a str>,
    namespaces: BTreeSet<&'a str>,
    made_for: &'a str,
    deferred: BTreeSet<String>,
}

enum Contract<'a> {
    Method(&'a MethodContract),
    Function(&'a FunctionContract),
}

impl Contract<'_> {
    /// `hash` is one the old recipe gives this contract, with either `wasm` bit.
    fn old_recipe(&self, hash: &str) -> bool {
        // Field order and omission rules are part of the compact JSON being hashed.
        #[derive(Serialize)]
        struct Method<'a> {
            signature: &'a str,
            required: bool,
            wasm: bool,
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            refs: &'a [String],
        }
        #[derive(Serialize)]
        struct Function<'a> {
            signature: &'a str,
            #[serde(skip_serializing_if = "str::is_empty")]
            impl_generics: &'a str,
            wasm: bool,
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            refs: &'a [String],
        }
        [true, false].into_iter().any(|wasm| {
            let old = match self {
                Contract::Method(m) => contract_hash(&Method {
                    signature: &m.signature,
                    required: m.required,
                    wasm,
                    refs: &m.refs,
                }),
                Contract::Function(f) => contract_hash(&Function {
                    signature: &f.signature,
                    impl_generics: &f.impl_generics,
                    wasm,
                    refs: &f.refs,
                }),
            };
            old == hash
        })
    }
}

impl<'a> LegacyFit<'a> {
    pub(crate) fn new(
        table: Option<&'a UsesTable>,
        target: &'a Surface,
        entries: &[&'a Entry],
    ) -> Self {
        Self(table.filter(|table| table.schema <= 2).map(|table| {
            let methods = target.traits.iter().flat_map(|trait_| &trait_.methods);
            let functions = target.functions.iter().chain(&target.handles);
            let current_types: BTreeSet<&str> = target
                .types
                .iter()
                .flat_map(|ty| {
                    ty.id
                        .strip_prefix("type:")
                        .into_iter()
                        .chain(ty.paths.iter().map(String::as_str))
                })
                .collect();
            let known_types = entries
                .iter()
                .filter_map(|e| e.id.strip_prefix("type:"))
                .filter(|name| !name.contains('.'))
                .collect::<BTreeSet<_>>();
            let namespaces = current_types
                .iter()
                .chain(&known_types)
                .filter_map(|name| name.rsplit_once("::").map(|(module, _)| module))
                .collect();
            Old {
                hashes: methods
                    .map(|m| (m.id.as_str(), Contract::Method(&m.contract)))
                    .chain(
                        // The old recipe cannot certify a newly versioned behaviour.
                        functions
                            .filter(|f| f.contract.behaviour_versions.is_empty())
                            .map(|f| (f.id.as_str(), Contract::Function(&f.contract))),
                    )
                    .collect(),
                known_types,
                current_types,
                namespaces,
                made_for: &table.made_for,
                deferred: BTreeSet::new(),
            }
        }))
    }

    pub(crate) fn check_missing_record(&mut self, id: &str) -> bool {
        let Some(old) = &mut self.0 else { return true };
        old.deferred.insert(id.to_owned());
        false
    }

    pub(crate) fn check_missing_impl(&mut self, id: &str, name: &str) -> bool {
        let Some(old) = &mut self.0 else { return true };
        let Some((_, operands)) = name.split_once('<') else {
            return true;
        };
        // Impl text is the surface exporter's canonical Rust spelling. Only treat a missing
        // operand as an API type when the old table names it as one or it uses an API namespace.
        // Bare foreign traits and standard types are not evidence of an API deletion.
        let defer = operands
            .split(|c: char| !(c.is_alphanumeric() || matches!(c, ':' | '_' | '#')))
            .any(|ty| {
                !old.current_types.contains(ty)
                    && (old.known_types.contains(ty)
                        || ty.rsplit_once("::").is_some_and(|(module, name)| {
                            !name.is_empty() && old.namespaces.contains(module)
                        }))
            });
        if defer {
            old.deferred.insert(id.to_owned());
        }
        !defer
    }

    pub(crate) fn deferred_note(self) -> Option<String> {
        let old = self.0?;
        if old.deferred.is_empty() {
            return None;
        }
        let ids = old
            .deferred
            .iter()
            .map(|id| format!("`{}`", id.escape_debug()))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "not checked: {ids}; list written for {} before schema 3; the compile decides",
            old.made_for.escape_debug()
        ))
    }

    pub(crate) fn matches(&self, id: &str, new_hash: &str, old_hash: &str) -> bool {
        let Some(contract) = self.0.as_ref().and_then(|old| old.hashes.get(id)) else {
            return new_hash == old_hash;
        };
        contract.old_recipe(old_hash)
    }
}

/// A schema-1 table did not record all hook versions. Recover only those numbers from the
/// made-for SDK; never construct a `Surface` or compare old surface documents here.
pub(crate) fn pin_hook_versions(
    table: &mut crate::uses::UsesTable,
    made_for: &[u8],
) -> Result<(), crate::ReadError> {
    if table.schema > 2 {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct Hook<'a> {
        id: &'a str,
        version: u64,
    }
    #[derive(Deserialize)]
    struct HookVersions<'a> {
        schema: u32,
        #[serde(borrow)]
        hooks: Vec<Hook<'a>>,
    }
    let baseline: HookVersions<'_> =
        serde_json::from_slice(made_for).map_err(|e| crate::ReadError::Json(e.to_string()))?;
    if !matches!(baseline.schema, 2 | crate::SCHEMA) {
        return Err(crate::ReadError::Schema(baseline.schema));
    }
    let mut versions = BTreeMap::new();
    for hook in baseline.hooks {
        if versions.insert(hook.id, hook.version).is_some() {
            return Err(crate::ReadError::DuplicateId(hook.id.into()));
        }
    }
    for entry in &mut table.entries {
        entry.version = entry
            .version
            .or_else(|| versions.get(entry.id.as_str()).copied());
    }
    Ok(())
}
