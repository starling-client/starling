//! A schema walk over the admitted corpus. Unlike `content_golden`, this test
//! uses the current reader's schema, not the previous release's.

#[path = "support/content_corpus.rs"]
mod corpus;
#[path = "content_keys/forms.rs"]
mod forms;
#[path = "content_keys/source_guard.rs"]
mod source_guard;

use serde::de::{
    Deserialize, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, VariantAccess,
    Visitor,
    value::{MapDeserializer, SeqDeserializer},
};
use serde_json::Value;
use std::{cell::RefCell, collections::BTreeSet, fs, io::Read, path::Path};
use sunrise_mod_package::{
    Content, Manifest, Package,
    content::{DECLARATION_KINDS, DYE_COLORS, ENTITY_TEMPLATES, probe_declaration},
    extension_tags,
    manifest::MANIFEST,
};

/// The corpus and the reader sources are named from the crate root.
fn at_crate_root() {
    std::env::set_current_dir(env!("CARGO_MANIFEST_DIR")).unwrap();
}

/// The same two checks for the daily run, whose green a stable release needs (`verify` runs
/// the plain ones on every rollup and main push).
#[test]
#[ignore = "daily"]
fn daily_content_keys() {
    content_keys();
    source_guard::serde_blind_spots_are_reviewed();
}

#[test]
fn content_keys() {
    at_crate_root();
    let coverage = RefCell::new(Coverage::default());
    let empty = Value::Object(Default::default());
    // The callbacks publish the top-level fields even when the empty object fails
    // to decode. A declaration kind's absence must not hide its entire schema.
    for &kind in DECLARATION_KINDS {
        drop(probe_declaration(kind, Probe::new(kind, &empty, &coverage)));
    }
    drop(Manifest::deserialize(Probe::new(
        "manifest", &empty, &coverage,
    )));

    let mut block_components = BTreeSet::new();
    let mut item_removal = false;
    let mut extension_files = BTreeSet::new();
    for (entry, path) in corpus::entries() {
        if entry.starts_with("refused_") {
            continue;
        }
        let path = Path::new(&path);
        let package = Package::read(path).unwrap();
        let manifest_text = if path.is_dir() {
            fs::read_to_string(path.join(MANIFEST)).unwrap()
        } else {
            // The probe needs the original input, not a reserialized Manifest with defaults.
            let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
            let mut text = String::new();
            zip.by_name(MANIFEST)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            text
        };
        let mut toml: toml::Value = toml::from_str(&manifest_text).unwrap();
        // The probe replays the manifest as JSON, which has no `nan` or `inf`: the form is a float
        // either way, and the golden file holds the value.
        if let Some(properties) = toml
            .get_mut("properties")
            .and_then(toml::Value::as_table_mut)
        {
            for (_, value) in properties.iter_mut() {
                if value.as_float().is_some_and(|number| !number.is_finite()) {
                    *value = toml::Value::Float(0.0);
                }
            }
        }
        let value = serde_json::to_value(toml).unwrap();
        Manifest::deserialize(Probe::new("manifest", &value, &coverage))
            .unwrap_or_else(|e| panic!("{} ({MANIFEST}): {e}", path.display()));

        let content = Content::read(path, &package.manifest).unwrap();
        assert_eq!(content.server_only_refusal(&package.manifest), None);
        let reader = content.reader();
        for file in content.files.iter() {
            let mut parts = file.splitn(5, '/');
            if parts.next() != Some("data") {
                continue;
            }
            let ns = parts.next().unwrap();
            if parts.next() != Some("sunrise")
                || !package.manifest.owned_namespaces().any(|owned| owned == ns)
            {
                continue;
            }
            let kind = parts.next().unwrap();
            let bytes = reader.read(file).unwrap();
            let value = serde_json::from_slice(&bytes).unwrap();
            probe_declaration(kind, Probe::new(kind, &value, &coverage))
                .unwrap_or_else(|e| panic!("{file}: {e}"));
        }

        // These finite string vocabularies are not Serde enums. Use the reader's constants.
        let mut evidence = coverage.borrow_mut();
        evidence.variants(
            "block.properties.instrument",
            mod_vocab::NoteInstrument::ALL.iter().map(|i| i.name()),
            content
                .blocks
                .iter()
                .filter_map(|block| block.properties.instrument.map(|i| i.name())),
        );
        evidence.variants(
            "block.sign_text_color",
            DYE_COLORS.iter().copied(),
            content
                .blocks
                .iter()
                .filter_map(|block| block.sign_text_color.as_deref()),
        );
        evidence.variants(
            "entity_type.template",
            ENTITY_TEMPLATES.iter().copied(),
            content
                .entity_types
                .iter()
                .map(|entity| entity.template.as_str()),
        );

        for block in content.blocks {
            block_components.extend(block.components.into_keys());
        }
        item_removal |= content
            .items
            .iter()
            .any(|item| item.components.keys().any(|k| k.starts_with('!')));
        extension_files.extend(
            content
                .files
                .iter()
                .filter(|p| p.starts_with("data/sunrise/tags/"))
                .cloned(),
        );
    }

    let coverage = coverage.into_inner();
    let missing: Vec<_> = coverage
        .expected
        .difference(&coverage.seen)
        .map(String::as_str)
        // These three fields belong to exact-build native mods, not content mods.
        .filter(|key| {
            !matches!(
                *key,
                "manifest.native" | "manifest.runtime" | "manifest.internals"
            )
        })
        .collect();
    assert!(
        !coverage.expected.is_empty(),
        "no declaration schema was probed"
    );
    assert!(
        missing.is_empty(),
        "no admitted fixture for:\n{}",
        missing.join("\n")
    );
    for key in ["minecraft:max_stack_size", "minecraft:rarity"] {
        assert!(
            block_components.contains(key),
            "no admitted block component {key}"
        );
    }
    assert!(
        block_components.iter().any(|key| key.starts_with('!')),
        "no admitted block component removal"
    );
    assert!(item_removal, "no admitted item component removal");
    for tag in extension_tags::ALL {
        let path = format!(
            "data/sunrise/tags/{}/{}.json",
            tag.registry,
            tag.id.strip_prefix("sunrise:").unwrap()
        );
        assert!(
            extension_files.contains(&path),
            "no admitted extension tag fixture for {path}"
        );
    }
}

#[derive(Default)]
struct Coverage {
    expected: BTreeSet<String>,
    seen: BTreeSet<String>,
}

impl Coverage {
    fn collection(&mut self, path: &str, segment: &str, nonempty: bool) {
        let elements = format!("{path}{segment}");
        self.expected.insert(elements.clone());
        if nonempty {
            self.seen.insert(elements);
        }
    }

    fn keys(&mut self, path: &str, keys: &[&str], value: &serde_json::Map<String, Value>) {
        for key in keys {
            let field = format!("{path}.{key}");
            self.expected.insert(field.clone());
            // An empty container supplies no evidence about its elements. Keep looking
            // for a fixture that can actually drive their deserialize callbacks.
            if value.get(*key).is_some_and(|v| match v {
                Value::Null => false,
                Value::Array(values) => !values.is_empty(),
                Value::Object(values) => !values.is_empty(),
                _ => true,
            }) {
                self.seen.insert(field);
            }
        }
    }

    fn variants<'a, 'b>(
        &mut self,
        path: &str,
        variants: impl IntoIterator<Item = &'a str>,
        seen: impl IntoIterator<Item = &'b str>,
    ) {
        self.expected.extend(
            variants
                .into_iter()
                .map(|variant| format!("{path}::{variant}")),
        );
        self.seen
            .extend(seen.into_iter().map(|variant| format!("{path}::{variant}")));
    }
}

/// Replay fixture values through the real Serde visitors. Struct fields and
/// enum payloads add named path segments; collections use element/key/value
/// segments so keys and values cannot satisfy each other's schema.
struct Probe<'a> {
    input: Input<'a>,
    path: String,
    coverage: &'a RefCell<Coverage>,
}

#[derive(Clone, Copy)]
enum Input<'a> {
    Value(&'a Value),
    Key(&'a str),
}

impl<'a> Probe<'a> {
    fn new(path: &str, value: &'a Value, coverage: &'a RefCell<Coverage>) -> Self {
        Self {
            input: Input::Value(value),
            path: path.into(),
            coverage,
        }
    }

    fn at(&self, segment: &str, input: Input<'a>) -> Self {
        Self {
            input,
            path: format!("{}{segment}", self.path),
            coverage: self.coverage,
        }
    }
}

impl<'de> IntoDeserializer<'de, serde_json::Error> for Probe<'de> {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self {
        self
    }
}

impl<'de> Deserializer<'de> for Probe<'de> {
    type Error = serde_json::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let value = match self.input {
            Input::Value(value) => value,
            Input::Key(key) => return visitor.visit_borrowed_str(key),
        };
        forms::probe(&self, value)?;
        match value {
            Value::Object(map) => {
                visitor.visit_map(MapDeserializer::new(map.iter().map(|(key, value)| {
                    (
                        self.at("{key}", Input::Key(key)),
                        self.at("{value}", Input::Value(value)),
                    )
                })))
            }
            Value::Array(seq) => visitor.visit_seq(SeqDeserializer::new(
                seq.iter().map(|value| self.at("[]", Input::Value(value))),
            )),
            value => value.deserialize_any(visitor),
        }
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if let Input::Value(Value::Object(map)) = self.input {
            self.coverage
                .borrow_mut()
                .collection(&self.path, "{entries}", !map.is_empty());
        }
        self.deserialize_any(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if let Input::Value(Value::Array(seq)) = self.input {
            self.coverage
                .borrow_mut()
                .collection(&self.path, "[]", !seq.is_empty());
        }
        self.deserialize_any(visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let Input::Value(Value::Object(map)) = self.input else {
            return self.deserialize_any(visitor);
        };
        self.coverage.borrow_mut().keys(&self.path, fields, map);
        visitor.visit_map(MapDeserializer::new(map.iter().map(|(key, value)| {
            (
                key.as_str(),
                self.at(&format!(".{key}"), Input::Value(value)),
            )
        })))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let input = match self.input {
            Input::Value(Value::String(variant)) => Input::Key(variant),
            input => input,
        };
        match input {
            Input::Key(variant) => {
                self.coverage.borrow_mut().variants(
                    &self.path,
                    variants.iter().copied(),
                    [variant],
                );
                visitor.visit_enum(variant.into_deserializer())
            }
            Input::Value(Value::Object(map)) if map.len() == 1 => {
                let (variant, value) = map.iter().next().unwrap();
                let variant = variants
                    .iter()
                    .copied()
                    .find(|v| *v == variant)
                    .ok_or_else(|| serde::de::Error::unknown_variant(variant, variants))?;
                self.coverage.borrow_mut().variants(
                    &self.path,
                    variants.iter().copied(),
                    [variant],
                );
                visitor.visit_enum(Tagged {
                    variant,
                    payload: self.at(&format!("::{variant}"), Input::Value(value)),
                })
            }
            Input::Value(value) => value.deserialize_enum(name, variants, visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if matches!(self.input, Input::Value(Value::Null)) {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        unit unit_struct tuple tuple_struct identifier ignored_any
    }
}

struct Tagged<'de> {
    variant: &'static str,
    payload: Probe<'de>,
}

impl<'de> EnumAccess<'de> for Tagged<'de> {
    type Error = serde_json::Error;
    type Variant = Self;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self::Variant), Self::Error> {
        Ok((seed.deserialize(self.variant.into_deserializer())?, self))
    }
}

impl<'de> VariantAccess<'de> for Tagged<'de> {
    type Error = serde_json::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        <()>::deserialize(self.payload)
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        seed.deserialize(self.payload)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.payload.deserialize_tuple(len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.payload
            .deserialize_struct(self.variant, fields, visitor)
    }
}
