//! `ConsumeEffect.Type` dispatch through the connection's synchronized registry.
use crate::{Error, Result, buf::Reader, packets::configuration::RegistryEntry};
use std::{collections::HashMap, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_world::component_value::consume_effects::{Admitted, by_name, entries};

pub type AdmissionLookup<'a> = dyn Fn(&Arc<str>, &[u8]) -> Option<Admitted> + 'a;

pub const REGISTRY: &str = "minecraft:consume_effect_type";
const BUILTINS: [&str; 5] = [
    "minecraft:apply_effects",
    "minecraft:remove_effects",
    "minecraft:clear_all_effects",
    "minecraft:teleport_randomly",
    "minecraft:play_sound",
];

pub fn builtin_id(name: &str) -> Option<i32> {
    let (namespace, path) = name.split_once(':')?;
    builtin_id_parts(namespace, path)
}

pub(crate) fn builtin_id_parts(namespace: &str, path: &str) -> Option<i32> {
    if namespace != "minecraft" {
        return None;
    }
    BUILTINS
        .iter()
        .position(|key| key.strip_prefix("minecraft:") == Some(path))
        .map(|i| i as i32)
}

pub fn builtin_name(id: i32) -> Option<&'static str> {
    BUILTINS.get(usize::try_from(id).ok()?).copied()
}

pub fn has_custom_types() -> bool {
    entries().next().is_some()
}

/// Registry rows in the server's protocol ID order. Custom rows carry a Wire schema.
pub fn configuration() -> impl Iterator<Item = (&'static str, u64)> {
    BUILTINS
        .into_iter()
        .map(|name| (name, 0))
        .chain(entries().map(|(name, ty)| (name.as_ref(), ty.schema)))
}

/// A snapshot's numeric IDs and inverse index. Both directions are derived
/// from the same advertised order, never from local registration order.
#[derive(Clone, Debug)]
pub struct Snapshot {
    identity: Arc<()>,
    kinds: Vec<Kind>,
    ids: HashMap<&'static str, HashMap<&'static str, i32>>,
}

impl Snapshot {
    pub(crate) fn resolve(entries: &[RegistryEntry]) -> Result<Self> {
        let mut snapshot = Self {
            identity: Arc::new(()),
            kinds: Vec::with_capacity(entries.len()),
            ids: HashMap::new(),
        };
        for (index, entry) in entries.iter().enumerate() {
            let (kind, name, expected) = if let Some(id) = builtin_id(&entry.id) {
                (Kind::Builtin(id), BUILTINS[id as usize], 0)
            } else if let Some((name, ty)) = by_name(&entry.id) {
                (Kind::Native(name, ty), name.as_ref(), ty.schema)
            } else {
                return Err(Error::new(format!("unknown consume effect {}", entry.id)));
            };
            let (namespace, path) = name.split_once(':').expect("registered effect key");
            let id =
                i32::try_from(index).map_err(|_| Error::new("consume effect ID out of range"))?;
            if !matches!(&entry.data, Some(Tag::Long(schema)) if *schema as u64 == expected)
                || snapshot
                    .ids
                    .entry(namespace)
                    .or_default()
                    .insert(path, id)
                    .is_some()
            {
                return Err(Error::new(format!(
                    "duplicate or incompatible consume effect {name}"
                )));
            }
            snapshot.kinds.push(kind);
        }
        if BUILTINS.iter().any(|name| {
            let path = name.strip_prefix("minecraft:").expect("built-in namespace");
            snapshot.id("minecraft", path).is_none()
        }) {
            return Err(Error::new("missing built-in consume effect type"));
        }
        Ok(snapshot)
    }

    /// A connection-local identity for retained bytes that use this mapping.
    pub fn identity(&self) -> &Arc<()> {
        &self.identity
    }

    pub(crate) fn kind(&self, id: i32) -> Option<Kind> {
        self.kinds.get(usize::try_from(id).ok()?).copied()
    }

    pub(crate) fn id(&self, namespace: &str, path: &str) -> Option<i32> {
        self.ids.get(namespace)?.get(path).copied()
    }

    pub(crate) fn owned_heap_bytes(&self) -> usize {
        use crate::retained::fresh_hash_table_bytes;

        let kind_bytes = self
            .kinds
            .capacity()
            .saturating_mul(size_of::<Kind>())
            .saturating_add(2 * size_of::<usize>()); // Arc allocation's reference counts
        self.ids.values().fold(
            kind_bytes.saturating_add(fresh_hash_table_bytes::<(&str, HashMap<&str, i32>)>(
                self.ids.capacity(),
            )),
            |size, ids| size.saturating_add(fresh_hash_table_bytes::<(&str, i32)>(ids.capacity())),
        )
    }
}

/// Stream discriminant after resolving the connection's ID to its registered type.
#[derive(Clone, Copy)]
pub enum Kind {
    Builtin(i32),
    Native(
        &'static std::sync::Arc<str>,
        &'static sunrise_world::component_value::consume_effects::Type,
    ),
}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin(id) => f.debug_tuple("Builtin").field(id).finish(),
            Self::Native(name, _) => f.debug_tuple("Native").field(name).finish(),
        }
    }
}

pub fn read_kind(reader: &mut Reader<'_>) -> Result<Kind> {
    let id = reader.varint()?;
    reader
        .consume_effect_kind(id)
        .ok_or_else(|| Error::new(format!("unknown consume effect type {id}")))
}

pub(super) fn skip_native(
    ty: &sunrise_world::component_value::consume_effects::Type,
    reader: &mut Reader<'_>,
) -> Result<()> {
    let (size, _) = (ty.read)(reader.unread(), false).map_err(Error::new)?;
    reader.bytes(size)?;
    Ok(())
}

/// Read one structural value and project it onto the persistent map. A mod whose
/// save callback fails loses only that effect; its Wire framing is still usable.
pub fn read_native(
    name: &std::sync::Arc<str>,
    ty: &sunrise_world::component_value::consume_effects::Type,
    reader: &mut Reader<'_>,
) -> Result<Option<Admitted>> {
    let unread = reader.unread();
    if let Some(admitted) = reader.consume_effect_admissions() {
        let (size, _) = (ty.read)(unread, false).map_err(Error::new)?;
        let bytes = unread
            .get(..size)
            .ok_or_else(|| Error::new("consume effect extends beyond its patch"))?;
        if let Some(effect) = admitted(name, bytes) {
            reader.bytes(size)?;
            return Ok(Some(effect));
        }
    }
    let (size, saved) = (ty.read)(unread, true).map_err(Error::new)?;
    let bytes = reader.bytes(size)?;
    Ok(saved.map(|saved| Admitted::new(name, saved, bytes.into())))
}
