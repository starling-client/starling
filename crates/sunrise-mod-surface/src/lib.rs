//! `surface.json`: the stable mod API of one game build, with optional engine records for CI and audits.
//!
//! The generator (`sunrise-stage surface`) writes it from the declarations the build parses;
//! nothing in it is kept by hand. This crate holds its record types, their contract hashes, the
//! strict reader and shape hashing. It has no engine dependency.
//!
//! # Ids
//!
//! Every record and every member record has one id, unique in the document. An item's path is
//! a path a mod writes after `mod_api::`: the path of the module that defines it when that path
//! is public (so a re-export added later never changes the id), otherwise its shortest public
//! path (the first in byte order among equals). An item defined in a private module has no
//! public definition path; its public path is the one that survives moving it between internal
//! files, so that path is its id. `paths` lists its public paths (each module through at most 16 routes of re-exports); signatures and `refs` write
//! ids.
//!
//! | Record | Id |
//! |---|---|
//! | hook | `hook:<name>` |
//! | argument or output of a hook | `hook:<name>.<argument>` |
//! | trait | `trait:<path>` |
//! | trait method | `method:<trait path>::<name>` |
//! | associated type or const of a trait | `assoc:<trait path>::<name>` |
//! | inherent method or const of an API type, or of a trait object (`impl dyn Trait`) | `handle:<type or trait path>::<name>` |
//! | free function | `fn:<path>` |
//! | macro | `macro:<path>` |
//! | type | `type:<path>` |
//! | public field or variant of a type | `type:<path>.<field or variant>` (a tuple field by position) |
//! | Part kind | `part:<kind>` |
//! | capability | `capability:<key>` (`registries`, `sunrise:fluids`) |
//! | behaviour | `behaviour:<name>` (`add_to_merging`) |
//! | published loop | `loop:<name>` |
//! | listed internal | `internal:<kind>:<path>` (with ` [<cfg>]` for a guarded row) |
//!
//! # Hashes
//!
//! A record's `hash` is [`contract_hash`] of its contract: every field except `id`, `hash`,
//! `doc`, `paths`, a hook's `engine` and `counterparts`, a type's `shape`, `impls`, `wire` and
//! `wire_form`, a struct's `constructible` and `non_exhaustive` (which matter only to a
//! mod that builds it, judged by the fit test), and its member records. Each
//! member (a method, field or variant) has its own hash, so adding a method to `Host`, adding a
//! hook or adding a field to a type mods cannot build changes no existing record's hash. A
//! container's hash covers its members only where a new member breaks users: the variants of an
//! exhaustive enum, the required methods of a trait mods may implement (any role but `call`; a
//! role the boundary lists cannot tell is `none`, which counts as implementable). Such a
//! trait's supertrait bounds are in its hash too; a trait mods only call is its method set, so
//! gaining or losing a supertrait (a sub-host trait of `Host`) changes no existing record's hash,
//! and a method that goes shows as that method removed. [`Surface::closure`] does not follow a
//! called trait's supertraits; [`Surface::reach`] does. A hook's hash covers what every listener
//! relies on (side, kind, scope, cancellable, context, host, owner, whether it has an output),
//! not its arguments, which are member records, nor its `version`. A struct's field set is never
//! in its hash; its `fields_hash` says whether a full literal still builds. A type's `impls` are a set, derived and
//! hand-written alike, outside its hash: one gained is additive, one lost is a change the fit
//! test names. A type's `shape` is its full wire layout ([`shape`]) and is not hashed into its
//! contract. The document's `hash` is [`Surface::document_hash`] over every (id, hash).
//!
//! A behaviour's hash covers nothing but that the record exists; its `version` is compared on its
//! own, like a hook's, and its `doc` and `tests` are outside the hash.
//!
//! A trait's `role` says whether mods implement it, call it, or neither: the generator takes it
//! from the vocabulary's role lists, resolved to the trait itself, not its name.
//! A trait on neither list has role `none`.
//!
//! Every member record lists its owner (the trait or type) in `refs`, so
//! [`Surface::closure`] from a member reaches its owner.
//!
//! # Reading
//!
//! [`Surface::read`] refuses an unknown `schema` number and malformed or truncated JSON with a
//! typed [`ReadError`], and ignores fields it does not know. It does not recompute record
//! hashes: a newer writer may hash contract fields this reader does not know, and the file's
//! integrity is the signature's job. [`Surface::unsealed`] does, for a release gate that judges a
//! newer writer's file with this reader.
pub mod build_data;
pub mod child;
pub mod diff;
mod legacy_hash;
pub mod records;
pub mod scan;
pub mod shape;
mod syntax;
pub mod uses;

pub use diff::{Diff, Verdict, diff};
pub use records::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema this crate writes and reads. Adding a field does not change it; removing or
/// redefining one does. Schema 3 records hash only the native contract.
pub const SCHEMA: u32 = 3;

/// The whole document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Surface {
    pub schema: u32,
    pub hash: String,
    #[serde(default)]
    pub hooks: Vec<Hook>,
    #[serde(default)]
    pub traits: Vec<Trait>,
    /// Inherent methods of the API's types, handles and values alike.
    #[serde(default)]
    pub handles: Vec<Function>,
    #[serde(default)]
    pub functions: Vec<Function>,
    #[serde(default)]
    pub macros: Vec<Macro>,
    #[serde(default)]
    pub types: Vec<Type>,
    #[serde(default)]
    pub parts: Vec<Part>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// Promises that are no type or signature. A reader built before they existed ignores the
    /// list, so adding it kept the schema number.
    #[serde(default)]
    pub behaviours: Vec<Behaviour>,
    /// Reserved-namespace tags that packages may extend, declared beside engine readers.
    /// Older readers ignore this additive collection without a schema change.
    #[serde(default)]
    pub extension_tags: Vec<ExtensionTag>,
    /// Native loops and listed internals for CI and local audits. Omitted
    /// from SDK exports and not inferred by the API source-use scanner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engine_items: Vec<EngineItem>,
}

/// A document's top-level values, unread, refusing a key listed twice: a later value would hide
/// an earlier one.
struct TopLevel<'a>(BTreeMap<String, &'a serde_json::value::RawValue>);

impl<'de> Deserialize<'de> for TopLevel<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Keys;
        impl<'de> serde::de::Visitor<'de> for Keys {
            type Value = TopLevel<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    let value = map.next_value()?;
                    if values.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(format!("{key} is listed twice")));
                    }
                }
                Ok(TopLevel(values))
            }
        }
        deserializer.deserialize_map(Keys)
    }
}

/// A record of a list a newer writer added, as this reader knows it: its `id` and `hash` join the
/// document hash.
#[derive(Deserialize)]
struct LaterRecord {
    id: String,
    hash: String,
}

/// Why a document was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Not JSON, truncated, or a field of the wrong type.
    Json(String),
    /// A schema number this reader does not know.
    Schema(u32),
    /// Two records with one id.
    DuplicateId(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "surface.json is not a valid surface: {error}"),
            Self::Schema(schema) => write!(f, "surface schema {schema}, this tool reads {SCHEMA}"),
            Self::DuplicateId(id) => write!(f, "surface.json lists {id} twice"),
        }
    }
}

impl std::error::Error for ReadError {}

/// First 128 bits of SHA-256, formatted as 32 lowercase hex digits.
pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let hash = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(32);
    for byte in &hash[..16] {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The contract hash of a record: the first 128 bits, as 32 lowercase hex digits, of the SHA-256
/// of the contract's compact JSON (fields in declaration order, absent optional fields left out).
pub fn contract_hash<T: Serialize>(contract: &T) -> String {
    let text = serde_json::to_string(contract).expect("contracts serialize");
    digest(&text)
}

/// The identifier a mod writes for an id: the last path segment, or the field or variant name.
/// A capability's is its key; its constant is in [`Surface::identifiers`].
pub fn leaf(id: &str) -> &str {
    let Some((kind, rest)) = id.split_once(':') else {
        return id;
    };
    match kind {
        "capability" | "part" | "behaviour" => rest,
        "hook" => rest.rsplit_once('.').map_or(rest, |(_, member)| member),
        "type" => match rest.rsplit_once('.') {
            Some((_, member)) => member,
            None => rest.rsplit("::").next().unwrap_or(rest),
        },
        _ => rest.rsplit("::").next().unwrap_or(rest),
    }
}

macro_rules! sealed {
    ($($record:ident($contract:ty) { $($extra:ident),* }),* $(,)?) => {$(
        impl $record {
            /// The record, with the hash of `contract`; the fields outside the hash are empty.
            pub fn new(id: impl Into<String>, doc: impl Into<String>, contract: $contract) -> Self {
                Self {
                    id: id.into(),
                    hash: contract_hash(&contract),
                    doc: doc.into(),
                    contract,
                    $($extra: Default::default(),)*
                }
            }
        }
    )*};
}

sealed!(
    Method(MethodContract) {},
    Function(FunctionContract) { paths },
    Macro(MacroContract) { paths },
    Field(FieldContract) {},
    Variant(VariantContract) {},
    Capability(CapabilityContract) {},
    Associated(AssociatedContract) {},
);

impl Hook {
    /// The hook, hashed over what every listener relies on whatever arguments it reads; its
    /// arguments and output become member records.
    pub fn new(
        id: impl Into<String>,
        doc: impl Into<String>,
        engine: HookEngine,
        counterparts: Counterparts,
        contract: HookContract,
    ) -> Self {
        #[derive(Serialize)]
        struct Own<'a> {
            side: Side,
            contract: HookKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            scope: &'a Option<Scope>,
            #[serde(skip_serializing_if = "Option::is_none")]
            cancellable: Option<bool>,
            #[serde(skip_serializing_if = "Option::is_none")]
            context: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            host: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            owner: &'a Option<String>,
            answered_by_mod: bool,
            output: bool,
        }
        let id = id.into();
        let hash = contract_hash(&Own {
            side: contract.side,
            contract: contract.contract,
            scope: &contract.scope,
            cancellable: contract.cancellable,
            context: &contract.context,
            host: &contract.host,
            owner: &contract.owner,
            answered_by_mod: contract.answered_by_mod,
            output: contract.output.is_some(),
        });
        let fields = contract
            .arguments
            .iter()
            .chain(&contract.output)
            .map(|argument| {
                let mut refs: Vec<String> = contract
                    .refs
                    .iter()
                    .filter(|r| names(&argument.ty, r))
                    .cloned()
                    .collect();
                refs.push(id.clone());
                refs.sort();
                Field::new(
                    format!("{id}.{}", argument.name),
                    "",
                    FieldContract {
                        ty: argument.ty.clone(),
                        refs,
                    },
                )
            })
            .collect();
        Self {
            id,
            hash,
            doc: doc.into(),
            engine,
            counterparts,
            contract,
            fields,
        }
    }
}

/// Whether the normalised type `ty` names the item `id` by its full path: `a::Item` and
/// `b::Item` are different items.
fn names(ty: &str, id: &str) -> bool {
    let path = id.split_once(':').map_or(id, |(_, path)| path);
    ty.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .any(|word| word == path)
}

/// The hash of a struct's field set: each public field's `name: type`, sorted. A literal or a
/// pattern names fields in any order, so moving a field changes nothing; a tuple struct's
/// field names are its positions, so moving one of its fields still changes the set.
pub fn fields_hash(fields: &[Field]) -> String {
    fields_hash_of(
        fields
            .iter()
            .map(|field| (leaf(&field.id), field.contract.ty.as_str())),
    )
}

/// [`fields_hash`] of `(name, type)` pairs: the exporter's, over every public field a literal
/// must name, `#[doc(hidden)]` ones included.
pub fn fields_hash_of<'a>(fields: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut lines: Vec<String> = fields
        .into_iter()
        .map(|(name, ty)| format!("{name}: {ty}\n"))
        .collect();
    lines.sort();
    digest(&lines.concat())
}

impl Trait {
    pub fn new(
        id: impl Into<String>,
        doc: impl Into<String>,
        contract: TraitContract,
        methods: Vec<Method>,
        associated: Vec<Associated>,
    ) -> Self {
        // A trait mods only call is its method set: a supertrait it gains or loses shows through
        // the methods themselves, so its bounds and their references stay out of its hash. Any
        // other role (`none` included, where the boundary lists cannot tell) may be implemented.
        let hash = if contract.role != Role::Call {
            contract_hash(&contract)
        } else {
            contract_hash(&TraitContract {
                bounds: Vec::new(),
                refs: Vec::new(),
                ..contract.clone()
            })
        };
        Self {
            id: id.into(),
            hash,
            doc: doc.into(),
            paths: Vec::new(),
            contract,
            methods,
            associated,
        }
    }
}

impl Type {
    pub fn new(
        id: impl Into<String>,
        doc: impl Into<String>,
        shape: Option<String>,
        contract: TypeContract,
        fields: Vec<Field>,
        variants: Vec<Variant>,
    ) -> Self {
        // `shape`, `wire` and `wire_form` describe encoding, not the Rust contract hashed here.
        // Whether a struct can
        // be built with a literal matters only to a mod that builds it, which the fit test judges
        // from `constructible` itself.
        let struct_like = matches!(
            contract.kind,
            TypeKind::Struct | TypeKind::TupleStruct | TypeKind::UnitStruct | TypeKind::Union
        );
        let hash = contract_hash(&TypeContract {
            wire: false,
            wire_form: None,
            constructible: false,
            non_exhaustive: contract.non_exhaustive && !struct_like,
            ..contract.clone()
        });
        Self {
            id: id.into(),
            hash,
            doc: doc.into(),
            shape,
            paths: Vec::new(),
            impls: Vec::new(),
            fields_hash: struct_like.then(|| fields_hash(&fields)),
            contract,
            fields,
            variants,
        }
    }
}

impl Behaviour {
    /// The behaviour, hashed over nothing but its existence: its `version` is compared on its
    /// own, and its `doc` and `tests` are outside the hash.
    pub fn new(
        id: impl Into<String>,
        doc: impl Into<String>,
        tests: Vec<String>,
        contract: BehaviourContract,
    ) -> Self {
        #[derive(Serialize)]
        struct Own {}
        Self {
            id: id.into(),
            hash: contract_hash(&Own {}),
            doc: doc.into(),
            tests,
            contract,
        }
    }
}

impl Part {
    pub fn new(
        id: impl Into<String>,
        doc: impl Into<String>,
        hooks: Vec<String>,
        contract: PartContract,
    ) -> Self {
        Self {
            id: id.into(),
            hash: contract_hash(&contract),
            doc: doc.into(),
            hooks,
            contract,
        }
    }
}

/// Every id reached from `ids` along `refs` ([`Surface::refs`]), `ids` included. A caller that
/// walks from many starts builds `refs` once.
pub(crate) fn walk_refs<'a>(
    refs: &BTreeMap<&str, Vec<&str>>,
    ids: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut pending: Vec<String> = ids.into_iter().map(str::to_owned).collect();
    while let Some(id) = pending.pop() {
        if let Some(next) = refs.get(id.as_str()) {
            pending.extend(
                next.iter()
                    .filter(|r| !out.contains(**r))
                    .map(|r| (*r).to_owned()),
            );
        }
        out.insert(id);
    }
    out
}

impl Surface {
    /// The document: each list sorted by id, a trait's members by id, a type's fields and
    /// variants in declaration order, and its hash.
    pub fn new(
        mut hooks: Vec<Hook>,
        mut traits: Vec<Trait>,
        mut handles: Vec<Function>,
        mut functions: Vec<Function>,
        mut macros: Vec<Macro>,
        mut types: Vec<Type>,
        mut parts: Vec<Part>,
        mut capabilities: Vec<Capability>,
    ) -> Self {
        hooks.sort_by(|a, b| a.id.cmp(&b.id));
        traits.sort_by(|a, b| a.id.cmp(&b.id));
        for item in &mut traits {
            item.methods.sort_by(|a, b| a.id.cmp(&b.id));
            item.associated.sort_by(|a, b| a.id.cmp(&b.id));
        }
        handles.sort_by(|a, b| a.id.cmp(&b.id));
        functions.sort_by(|a, b| a.id.cmp(&b.id));
        macros.sort_by(|a, b| a.id.cmp(&b.id));
        types.sort_by(|a, b| a.id.cmp(&b.id));
        parts.sort_by(|a, b| a.id.cmp(&b.id));
        capabilities.sort_by(|a, b| a.id.cmp(&b.id));
        let mut surface = Self {
            schema: SCHEMA,
            hash: String::new(),
            hooks,
            traits,
            handles,
            functions,
            macros,
            types,
            parts,
            capabilities,
            behaviours: Vec::new(),
            extension_tags: Vec::new(),
            engine_items: Vec::new(),
        };
        surface.hash = surface.document_hash();
        surface
    }

    /// The document with `behaviours` (sorted by id) added, and its hash.
    pub fn with_behaviours(mut self, mut behaviours: Vec<Behaviour>) -> Self {
        behaviours.sort_by(|a, b| a.id.cmp(&b.id));
        self.behaviours = behaviours;
        self.hash = self.document_hash();
        self
    }

    /// Add the declared extension tags and update the document hash.
    pub fn with_extension_tags(mut self, mut tags: Vec<ExtensionTag>) -> Self {
        tags.sort_by(|a, b| (&a.registry, &a.id).cmp(&(&b.registry, &b.id)));
        self.extension_tags = tags;
        self.hash = self.document_hash();
        self
    }

    /// Add the ordered engine catalog records and update the document hash.
    pub fn with_engine_items(mut self, mut items: Vec<EngineItem>) -> Self {
        items.sort_by(|a, b| a.id.cmp(&b.id));
        self.engine_items = items;
        self.hash = self.document_hash();
        self
    }

    /// Every named API record and member record as (id, hash), in no promised order.
    /// Engine items and extension tags are not API source uses.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        let hooks = self.hooks.iter().flat_map(|r| {
            std::iter::once((r.id.as_str(), r.hash.as_str()))
                .chain(r.fields.iter().map(|f| (f.id.as_str(), f.hash.as_str())))
        });
        let traits = self.traits.iter().flat_map(|r| {
            std::iter::once((r.id.as_str(), r.hash.as_str()))
                .chain(r.methods.iter().map(|m| (m.id.as_str(), m.hash.as_str())))
                .chain(
                    r.associated
                        .iter()
                        .map(|a| (a.id.as_str(), a.hash.as_str())),
                )
        });
        let functions = self
            .handles
            .iter()
            .chain(&self.functions)
            .map(|r| (r.id.as_str(), r.hash.as_str()));
        let macros = self.macros.iter().map(|r| (r.id.as_str(), r.hash.as_str()));
        let types = self.types.iter().flat_map(|r| {
            std::iter::once((r.id.as_str(), r.hash.as_str()))
                .chain(r.fields.iter().map(|f| (f.id.as_str(), f.hash.as_str())))
                .chain(r.variants.iter().map(|v| (v.id.as_str(), v.hash.as_str())))
        });
        let parts = self.parts.iter().map(|r| (r.id.as_str(), r.hash.as_str()));
        let capabilities = self
            .capabilities
            .iter()
            .map(|r| (r.id.as_str(), r.hash.as_str()));
        let behaviours = self
            .behaviours
            .iter()
            .map(|r| (r.id.as_str(), r.hash.as_str()));
        hooks
            .chain(traits)
            .chain(functions)
            .chain(macros)
            .chain(types)
            .chain(parts)
            .chain(capabilities)
            .chain(behaviours)
    }

    fn entries_with_engine(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries().chain(
            self.engine_items
                .iter()
                .map(|r| (r.id.as_str(), r.hash.as_str())),
        )
    }

    /// The SHA-256 (first 128 bits, hex) of `<id> <hash>\n` for every entry, in byte order of id.
    pub fn document_hash(&self) -> String {
        self.document_hash_with(&[])
    }

    /// [`Self::document_hash`] with `later`'s entries counted too: the records of lists a newer
    /// writer adds, which this reader knows only by their `id` and `hash`.
    fn document_hash_with(&self, later: &[LaterRecord]) -> String {
        let tags: Vec<_> = self
            .extension_tags
            .iter()
            .map(|tag| {
                (
                    format!("extension_tag:{}/{}", tag.registry, tag.id),
                    contract_hash(&(&tag.registry, &tag.id)),
                )
            })
            .collect();
        let mut entries: Vec<(&str, &str)> = self
            .entries_with_engine()
            .chain(tags.iter().map(|(id, hash)| (id.as_str(), hash.as_str())))
            .chain(later.iter().map(|r| (r.id.as_str(), r.hash.as_str())))
            .collect();
        entries.sort_unstable();
        let mut text = String::new();
        for (id, hash) in entries {
            text.push_str(id);
            text.push(' ');
            text.push_str(hash);
            text.push('\n');
        }
        digest(&text)
    }

    /// The ids whose stored hash is not the one this reader's constructors give their contract: a
    /// hook's argument records are compared whole with the ones its contract gives. A newer writer that hashes contract fields this reader doesn't
    /// know fails here too, so a release judged by an older tool keeps that tool's hashes.
    /// (`fields_hash` is outside: it also covers fields no record lists.)
    pub fn unsealed(&self) -> Vec<String> {
        fn sealed<'a>(
            out: &mut Vec<String>,
            records: impl IntoIterator<Item = (&'a str, &'a str, String)>,
        ) {
            for (id, stored, hash) in records {
                if stored != hash {
                    out.push(id.to_owned());
                }
            }
        }
        let mut out = Vec::new();
        for hook in &self.hooks {
            let fresh = Hook::new(
                hook.id.clone(),
                "",
                hook.engine.clone(),
                hook.counterparts.clone(),
                hook.contract.clone(),
            );
            sealed(
                &mut out,
                [(hook.id.as_str(), hook.hash.as_str(), fresh.hash)],
            );
            let fields = |fields: &[Field]| {
                fields
                    .iter()
                    .map(|f| (f.id.clone(), f.hash.clone(), f.contract.clone()))
                    .collect::<Vec<_>>()
            };
            if fields(&hook.fields) != fields(&fresh.fields) {
                out.push(format!("{} (its argument records)", hook.id));
            }
        }
        for item in &self.traits {
            let fresh = Trait::new(
                item.id.clone(),
                "",
                item.contract.clone(),
                Vec::new(),
                Vec::new(),
            );
            sealed(
                &mut out,
                [(item.id.as_str(), item.hash.as_str(), fresh.hash)],
            );
            sealed(
                &mut out,
                item.methods.iter().map(|m| {
                    let hash = Method::new(m.id.clone(), "", m.contract.clone()).hash;
                    (m.id.as_str(), m.hash.as_str(), hash)
                }),
            );
            sealed(
                &mut out,
                item.associated.iter().map(|a| {
                    let hash = Associated::new(a.id.clone(), "", a.contract.clone()).hash;
                    (a.id.as_str(), a.hash.as_str(), hash)
                }),
            );
        }
        sealed(
            &mut out,
            self.handles.iter().chain(&self.functions).map(|f| {
                let hash = Function::new(f.id.clone(), "", f.contract.clone()).hash;
                (f.id.as_str(), f.hash.as_str(), hash)
            }),
        );
        sealed(
            &mut out,
            self.macros.iter().map(|m| {
                let hash = Macro::new(m.id.clone(), "", m.contract.clone()).hash;
                (m.id.as_str(), m.hash.as_str(), hash)
            }),
        );
        for item in &self.types {
            let hash = Type::new(
                item.id.clone(),
                "",
                None,
                item.contract.clone(),
                Vec::new(),
                Vec::new(),
            )
            .hash;
            sealed(&mut out, [(item.id.as_str(), item.hash.as_str(), hash)]);
            sealed(
                &mut out,
                item.fields.iter().map(|f| {
                    let hash = Field::new(f.id.clone(), "", f.contract.clone()).hash;
                    (f.id.as_str(), f.hash.as_str(), hash)
                }),
            );
            sealed(
                &mut out,
                item.variants.iter().map(|v| {
                    let hash = Variant::new(v.id.clone(), "", v.contract.clone()).hash;
                    (v.id.as_str(), v.hash.as_str(), hash)
                }),
            );
        }
        sealed(
            &mut out,
            self.parts.iter().map(|p| {
                let hash = Part::new(p.id.clone(), "", Vec::new(), p.contract.clone()).hash;
                (p.id.as_str(), p.hash.as_str(), hash)
            }),
        );
        sealed(
            &mut out,
            self.capabilities.iter().map(|c| {
                let hash = Capability::new(c.id.clone(), "", c.contract.clone()).hash;
                (c.id.as_str(), c.hash.as_str(), hash)
            }),
        );
        sealed(
            &mut out,
            self.behaviours.iter().map(|b| {
                let hash = Behaviour::new(b.id.clone(), "", Vec::new(), b.contract.clone()).hash;
                (b.id.as_str(), b.hash.as_str(), hash)
            }),
        );
        sealed(
            &mut out,
            self.engine_items.iter().map(|e| {
                let hash = EngineItem::new(e.id.clone(), e.contract.clone()).hash;
                (e.id.as_str(), e.hash.as_str(), hash)
            }),
        );
        out
    }

    /// [`Self::read`], and the ids it finds unsealed: [`Self::unsealed`]'s records, and `surface`
    /// when the document's own hash is not [`Self::document_hash`] with every list this reader
    /// doesn't know counted in: this reader recomputes the document hash, [`Self::read`] does not.
    /// Such a list is how a newer writer adds a collection: JSON objects with a non-empty `id` (no
    /// whitespace or control characters) and a `hash` (32 lowercase hex digits), whose other
    /// fields this reader skips, and it refuses any other unknown list. Other
    /// unknown values are outside the document hash. Every id, an extension tag's
    /// `extension_tag:<registry>/<id>` included, is listed once in the whole document, and every
    /// top-level key once.
    pub fn read_sealed(bytes: &[u8]) -> Result<(Self, Vec<String>), ReadError> {
        let surface = Self::read(bytes)?;
        let mut unsealed = surface.unsealed();
        let TopLevel(raw) =
            serde_json::from_slice(bytes).map_err(|e| ReadError::Json(e.to_string()))?;
        let known = serde_json::to_value(&surface).expect("a surface serializes");
        let mut later = Vec::new();
        for (key, value) in raw {
            if key == "engine_items" || known.get(&key).is_some() || !value.get().starts_with('[') {
                continue;
            }
            let refused = |why: String| ReadError::Json(format!("{key}: {why}"));
            let records: Vec<&serde_json::value::RawValue> =
                serde_json::from_str(value.get()).map_err(|e| refused(e.to_string()))?;
            for record in records {
                if !record.get().starts_with('{') {
                    return Err(refused(format!("{} is no record", record.get())));
                }
                let record: LaterRecord =
                    serde_json::from_str(record.get()).map_err(|e| refused(e.to_string()))?;
                if record.id.is_empty()
                    || record
                        .id
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control())
                {
                    return Err(refused(format!("{:?} is no id", record.id)));
                }
                if record.hash.len() != 32
                    || !record
                        .hash
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(refused(format!("{:?} is no hash", record.hash)));
                }
                later.push(record);
            }
        }
        let tags: Vec<String> = surface
            .extension_tags
            .iter()
            .map(|tag| format!("extension_tag:{}/{}", tag.registry, tag.id))
            .collect();
        let mut seen = BTreeSet::new();
        let ids = surface.entries_with_engine().map(|(id, _)| id);
        for id in ids
            .chain(tags.iter().map(String::as_str))
            .chain(later.iter().map(|r| r.id.as_str()))
        {
            if !seen.insert(id) {
                return Err(ReadError::DuplicateId(id.to_owned()));
            }
        }
        if surface.hash != surface.document_hash_with(&later) {
            unsealed.push("surface".to_owned());
        }
        Ok((surface, unsealed))
    }

    /// The identifiers a mod's source may contain for each id: its [`leaf`], and a capability
    /// key's constant. A behaviour has none: no mod's code uses one.
    pub fn identifiers(&self) -> impl Iterator<Item = (&str, &str)> {
        let constants = self.capabilities.iter().filter_map(|c| {
            c.contract.constant.as_deref().map(|constant| {
                (
                    constant.rsplit("::").next().unwrap_or(constant),
                    c.id.as_str(),
                )
            })
        });
        self.entries()
            .filter(|(id, _)| !id.starts_with("behaviour:"))
            .map(|(id, _)| (leaf(id), id))
            .chain(constants)
    }

    /// `ids` with every API type or trait their signatures reference, transitively. A type's
    /// references are those of its fields, variants and alias target; a trait's, those of its
    /// supertraits when mods implement it, and none otherwise. Unknown ids are kept and lead nowhere.
    pub fn closure<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
        self.walk(ids, false)
    }

    /// [`Surface::closure`], also through the supertraits of traits mods only call: what a mod
    /// can reach, as when it calls a sub-host trait's method on `&mut dyn ServerHost`.
    pub fn reach<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
        self.walk(ids, true)
    }

    fn walk<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a str>,
        supertraits: bool,
    ) -> BTreeSet<String> {
        walk_refs(&self.refs(supertraits), ids)
    }

    /// Each record's direct references, the edges [`Surface::closure`] follows.
    pub(crate) fn refs(&self, supertraits: bool) -> BTreeMap<&str, Vec<&str>> {
        let mut refs: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for hook in &self.hooks {
            refs.insert(
                &hook.id,
                hook.contract.refs.iter().map(String::as_str).collect(),
            );
            for field in &hook.fields {
                refs.insert(
                    &field.id,
                    field.contract.refs.iter().map(String::as_str).collect(),
                );
            }
        }
        for item in &self.traits {
            // A trait mods may implement leads to its supertraits: implementing it means
            // implementing them. A called trait's lead there only with `supertraits`.
            let own = if supertraits || item.contract.role != Role::Call {
                item.contract.refs.iter().map(String::as_str).collect()
            } else {
                Vec::new()
            };
            refs.insert(&item.id, own);
            for associated in &item.associated {
                refs.insert(
                    &associated.id,
                    associated
                        .contract
                        .refs
                        .iter()
                        .map(String::as_str)
                        .collect(),
                );
            }
            for method in &item.methods {
                refs.insert(
                    &method.id,
                    method.contract.refs.iter().map(String::as_str).collect(),
                );
            }
        }
        for function in self.handles.iter().chain(&self.functions) {
            refs.insert(
                &function.id,
                function.contract.refs.iter().map(String::as_str).collect(),
            );
        }
        for item in &self.types {
            let mut all: Vec<&str> = item.contract.refs.iter().map(String::as_str).collect();
            for field in &item.fields {
                let own: Vec<&str> = field.contract.refs.iter().map(String::as_str).collect();
                all.extend(&own);
                refs.insert(&field.id, own);
            }
            for variant in &item.variants {
                let own: Vec<&str> = variant.contract.refs.iter().map(String::as_str).collect();
                all.extend(&own);
                refs.insert(&variant.id, own);
            }
            refs.insert(&item.id, all);
        }
        for capability in &self.capabilities {
            refs.insert(
                &capability.id,
                capability
                    .contract
                    .refs
                    .iter()
                    .map(String::as_str)
                    .collect(),
            );
        }
        refs
    }

    /// Reads a document: refuses an unknown schema, malformed JSON and repeated ids.
    pub fn read(bytes: &[u8]) -> Result<Self, ReadError> {
        #[derive(Deserialize)]
        struct Header {
            schema: u32,
        }
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| ReadError::Json(e.to_string()))?;
        if header.schema != SCHEMA {
            return Err(ReadError::Schema(header.schema));
        }
        let surface: Self =
            serde_json::from_slice(bytes).map_err(|e| ReadError::Json(e.to_string()))?;
        let mut seen = BTreeSet::new();
        for (id, _) in surface.entries_with_engine() {
            if !seen.insert(id) {
                return Err(ReadError::DuplicateId(id.to_owned()));
            }
        }
        let mut tags = BTreeSet::new();
        for tag in &surface.extension_tags {
            if !tags.insert((&tag.registry, &tag.id)) {
                return Err(ReadError::DuplicateId(format!(
                    "extension_tag:{}/{}",
                    tag.registry, tag.id
                )));
            }
        }
        Ok(surface)
    }

    /// The document as written: pretty JSON, ending in a newline.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a surface serializes");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod counterexamples_tests;
#[cfg(test)]
mod scan_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod uses_tests;
