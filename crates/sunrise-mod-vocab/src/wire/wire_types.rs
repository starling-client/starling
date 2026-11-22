//! Checked wire decoding for opaque API values.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{
    BlockEntityTypeKey, BlockKey, BoolGameRuleKey, DimensionKey, EffectKey, EnchantmentKey,
    EntityTypeKey, FluidKey, IntGameRuleKey, ItemKey, MenuKey, TagKey, UnresolvedFluid,
};

impl Wire for EntityTypeKey {
    layout_via!(u32, "registered-entity-type-slot:v1");
    const MIN: usize = <u32 as Wire>::MIN;
    fn put(&self, out: &mut Writer) {
        out.put(&u32::from(self));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::try_from(input.take::<u32>()?).map_err(|_| WireError::Invalid("EntityTypeKey"))
    }
}

macro_rules! slot_keys {
    (registries; $($key:path => $method:ident, $registry:literal, $resolve:expr;)+) => {
        impl crate::RegistryEntry {
            $(
            #[doc = concat!("Resolve this entry as a `", stringify!($key),
                "` through the current host. None for a different registry or an unknown entry.")]
            pub fn $method(&self, host: &dyn crate::Host) -> Option<$key> {
                if self.registry.namespace() != "minecraft" || self.registry.path() != $registry {
                    return None;
                }
                crate::backend_of(host)?.registry_entry_key_slot(self).map($key)
            }
            )+
        }
        /// Resolve the actual load-owned typed slot after the game checks live membership.
        // hidden: the game's RegistryEntry narrowing boundary, not a protocol-id conversion.
        #[doc(hidden)]
        pub fn registry_entry_key_slot(entry: &crate::RegistryEntry) -> Option<u32> {
            if entry.registry.namespace() != "minecraft" {
                return None;
            }
            match entry.registry.path() {
                $($registry => ($resolve)(&entry.id),)+
                _ => None,
            }
        }
        #[cfg(test)]
        mod registry_entry_narrowing_tests {
            struct Host(std::cell::Cell<usize>);
            impl crate::Host for Host {
                fn backend(&self) -> Option<crate::BackendRef<'_>> {
                    Some(crate::BackendRef::of(self))
                }
            }
            impl crate::Backend for Host {}
            crate::impl_host_areas!(Host);
            crate::impl_backend_areas!(Host, except(keys));
            impl crate::prelude::KeysBackend for Host {
                fn registry_entry_key_slot(&self, entry: &crate::RegistryEntry) -> Option<u32> {
                    self.0.set(self.0.get() + 1);
                    (entry.id.path() == "known").then_some(9123)
                }
            }
            #[test]
            fn every_family_checks_registry_and_preserves_host_issued_identity() {
                let host = Host(std::cell::Cell::new(0));
                $(
                    let entry = crate::RegistryEntry::new(
                        crate::Identifier::from_parts("minecraft", $registry).unwrap(),
                        crate::Identifier::parse("example:known").unwrap(),
                    );
                    assert_eq!(entry.$method(&host).unwrap().0, 9123);
                    let unknown = crate::RegistryEntry::new(
                        entry.registry.clone(),
                        crate::Identifier::parse("example:unknown").unwrap(),
                    );
                    assert_eq!(unknown.$method(&host), None);
                    let before = host.0.get();
                    let wrong_registry = crate::RegistryEntry::new(
                        crate::Identifier::from_parts("other", $registry).unwrap(),
                        entry.id.clone(),
                    );
                    assert_eq!(wrong_registry.$method(&host), None);
                    assert_eq!(host.0.get(), before);
                    let wrong_family = crate::RegistryEntry::new(
                        crate::Identifier::parse("minecraft:not_a_registry").unwrap(),
                        entry.id,
                    );
                    assert_eq!(wrong_family.$method(&host), None);
                    assert_eq!(host.0.get(), before);
                )+
            }
        }
    };
    ($width:ty; $($key:ty),+ $(,)?) => {$ (
        impl Wire for $key {
layout_fields!("slot" => $width);
    const MIN: usize = <$width as Wire>::MIN;
            fn put(&self, out: &mut Writer) { out.put(&self.0); }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                let slot=input.take::<$width>()?;
                Self::from_slot(slot).map_err(WireError::Invalid)
            }
        }
    )+};
}
// `EntityTypeKey` derives its own `Wire`.
slot_keys!(u32; crate::GameEventKey, BlockEntityTypeKey, BlockKey, BoolGameRuleKey, IntGameRuleKey, DimensionKey, EffectKey, EnchantmentKey, FluidKey, ItemKey, MenuKey, crate::client::particle_keys::ParticleKey, TagKey);
// The registry-entry key families: narrowing and host slot resolution share one
// table. Tags and unresolved fluids are not registry-entry key families.
slot_keys!(registries;
    BlockEntityTypeKey => as_block_entity_type_key, "block_entity_type", |id: &crate::Identifier| {
        crate::parts::resolved_block_entity_types().get()?.identifiers.get(id).copied()
    };
    BlockKey => as_block_key, "block", |id: &crate::Identifier| {
        crate::content::registry_keys::block_definitions()?.names.identifiers.get(id).copied()
    };
    BoolGameRuleKey => as_bool_game_rule_key, "game_rule", |id: &crate::Identifier| {
        let slot = *crate::parts::requested_keys().game_rules.identifiers.get(id)?;
        crate::parts::game_rule_bool_slot(BoolGameRuleKey(slot))?;
        Some(slot)
    };
    DimensionKey => as_dimension_key, "dimension", crate::content::registry_keys::existing_dimension_slot;
    EffectKey => as_effect_key, "mob_effect", |id: &crate::Identifier| {
        crate::parts::resolved_effects().get()?.names.identifiers.get(id).copied()
    };
    EnchantmentKey => as_enchantment_key, "enchantment", |id: &crate::Identifier| {
        crate::content::registry_keys::ENCHANTMENTS.read().ok()?.as_ref()?.identifiers.get(id).copied()
    };
    EntityTypeKey => as_entity_type_key, "entity_type", |id: &crate::Identifier| {
        crate::content::registry_keys::resolved_entity_types().get()?.names().identifiers.get(id).copied()
    };
    FluidKey => as_fluid_key, "fluid", |id: &crate::Identifier| {
        crate::parts::resolved_fluids().get()?.identifiers.get(id).copied()
    };
    ItemKey => as_item_key, "item", |id: &crate::Identifier| {
        crate::content::registry_keys::resolved_items().get()?.identifiers.get(id).copied()
    };
    MenuKey => as_menu_key, "menu", |id: &crate::Identifier| {
        crate::content::registry_keys::resolved_menus().read().ok()?.as_ref()?.identifiers.get(id).copied()
    };
    crate::client::particle_keys::ParticleKey => as_particle_key, "particle_type", |id: &crate::Identifier| {
        crate::client::particle_keys::particle_key(id).map(|key| key.0)
    };
);
slot_keys!(u64; UnresolvedFluid);

impl Wire for crate::Identifier {
    layout_fields!(ordinary_only "validated-identifier:v1"; "namespace" => String, "path" => String);
    const MIN: usize = 8;
    fn put(&self, out: &mut Writer) {
        if out.ordinary_only() {
            out.put_str(self.namespace());
            out.put_str(self.path());
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.ordinary_only()?;
        Self::from_parts(input.take_str()?, input.take_str()?)
            .map_err(|_| WireError::Invalid("invalid identifier"))
    }
}
impl Wire for crate::LevelKey {
    layout_fields!(ordinary_only "interned-level-key:v1"; "text" => String);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        if out.ordinary_only() {
            out.put_str(self.id());
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        // The level key's process-wide interner cannot charge this payload's budget.
        input.ordinary_only()?;
        Self::parse(input.take_str()?).map_err(|_| WireError::Invalid("invalid level key"))
    }
}
impl Wire for crate::ConfigPath {
    layout_fields!("text" => String);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put_str(self.as_str());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let text = input.take::<String>()?;
        if text.is_empty() {
            Ok(Self::root())
        } else {
            Self::try_from(text).map_err(|_| WireError::Invalid("invalid config path"))
        }
    }
}
impl Wire for crate::SpawnerKey {
    layout_fields!(ordinary_only "validated-spawner-key:v1"; "text" => String);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        if out.ordinary_only() {
            out.put_str(self.as_str());
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.ordinary_only()?;
        Self::new(input.take_str()?).ok_or(WireError::Invalid("invalid spawner key"))
    }
}
impl Wire for crate::CustomSpawners {
    layout_fields!(ordinary_only "unique-spawner-entries:v1"; "entries" => Vec<crate::SpawnerEntry>);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        if out.ordinary_only() {
            crate::SpawnerEntry::put_list(self.entries(), out);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.ordinary_only()?;
        Self::try_from(input.take::<Vec<crate::SpawnerEntry>>()?)
            .map_err(|_| WireError::Invalid("duplicate spawner"))
    }
}

impl Wire for crate::BlockStateRange {
    layout_fields!("first" => crate::BlockState, "len" => u32);
    const MIN: usize = 10;
    fn put(&self, out: &mut Writer) {
        out.put(&self.first());
        out.put(&self.len());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::new(input.take()?, input.take()?)
            .map_err(|_| WireError::Invalid("invalid block state range"))
    }
}

/// An NBT key: UTF-16 units, in the bytes of a `Vec<u16>` but written and read in one pass.
impl Wire for sunrise_core::nbt::JavaString {
    layout_fields!("units" => Vec<u16>);
    const MIN: usize = <Vec<u16> as Wire>::MIN;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.record(|input| input.take::<Vec<u16>>().map(Self))
    }
}

impl Wire for crate::ItemStack {
    layout_fields!("item" => crate::ItemKey, "count" => u32, "damage" => Option<u32>, "components" => Option<crate::Components>);
    const MIN: usize = 10;
    fn put(&self, out: &mut Writer) {
        out.put(&self.item());
        out.put(&self.count());
        out.put(&self.damage_component());
        let components = self.components();
        if components.is_none() || out.charge_with(|budget| budget.arc::<crate::Components>(1)) {
            out.put_opt(components);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let mut stack = Self::new(input.take()?, input.take()?)
            .ok_or(WireError::Invalid("empty item stack"))?;
        if let Some(damage) = input.take::<Option<u32>>()? {
            stack.set_damage(damage);
        }
        if let Some(components) = input.take::<Option<crate::Components>>()? {
            input.charge_with(|budget| budget.arc::<crate::Components>(1))?;
            stack
                .set_components(components.into_nbt())
                .map_err(|_| WireError::Invalid("invalid item components"))?;
        }
        Ok(stack)
    }
}

impl Wire for crate::ModelRef {
    layout_fields!("generation" => u64, "slot" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
        out.put(&self.1);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::from_parts(input.take()?, input.take()?).map_err(WireError::Invalid)
    }
}
impl Wire for crate::BakedModelKey {
    layout_fields!("generation" => u64, "slot" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
        out.put(&self.1);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::from_parts(input.take()?, input.take()?).map_err(WireError::Invalid)
    }
}
impl Wire for crate::RenderPipelineRef {
    layout_fields!("generation" => u64, "slot" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
        out.put(&self.1);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::from_parts(input.take()?, input.take()?).map_err(WireError::Invalid)
    }
}
impl Wire for crate::InstanceGroupRef {
    layout_fields!("generation" => u64, "slot" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
        out.put(&self.1);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Self::from_parts(input.take()?, input.take()?).map_err(WireError::Invalid)
    }
}

impl Wire for crate::BlockUpdateFlags {
    layout_fields!("0" => u32);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.put(&self.bits());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(Self(input.take()?))
    }
}

impl Wire for crate::Reduction {
    layout_fields!("tag" => u8, "0:Invulnerability" => (), "1:Armor" => (), "2:Enchantments" => (), "3:MobEffects" => (), "4:Absorption" => (), "5:InnateResistance" => ());
    const MIN: usize = 1;
    fn put(&self, out: &mut Writer) {
        out.put(
            &(match self {
                Self::Invulnerability => 0u8,
                Self::Armor => 1,
                Self::Enchantments => 2,
                Self::MobEffects => 3,
                Self::Absorption => 4,
                Self::InnateResistance => 5,
            }),
        );
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(match input.take::<u8>()? {
            0 => Self::Invulnerability,
            1 => Self::Armor,
            2 => Self::Enchantments,
            3 => Self::MobEffects,
            4 => Self::Absorption,
            5 => Self::InnateResistance,
            _ => return Err(WireError::Invalid("invalid reduction kind")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn round<T: Wire + std::fmt::Debug + PartialEq>(value: T) {
        let mut out = Writer::new();
        out.put(&value);
        assert_eq!(Reader::decode::<T>(out.bytes()).unwrap(), value);
        for n in 0..out.bytes().len() {
            assert!(Reader::decode::<T>(&out.bytes()[..n]).is_err());
        }
    }

    #[test]
    fn wire_checked_values_and_garbage() {
        round(crate::ConfigPath::new("example/options.toml").unwrap());
        round(crate::ConfigPath::root());
        round(crate::SpawnerKey::new("example:spawner").unwrap());
        round(crate::CustomSpawners::vanilla());
        round(crate::Identifier::parse("example:thing").unwrap());
        let mut out = Writer::new();
        out.put_str("../escape");
        assert!(Reader::decode::<crate::ConfigPath>(out.bytes()).is_err());
        out.clear();
        out.put_str("not namespaced");
        assert!(Reader::decode::<crate::SpawnerKey>(out.bytes()).is_err());
        out.clear();
        out.put(&vec![
            crate::SpawnerEntry::Builtin(crate::SpawnerKind::Cat),
            crate::SpawnerEntry::Builtin(crate::SpawnerKind::Cat),
        ]);
        assert!(Reader::decode::<crate::CustomSpawners>(out.bytes()).is_err());
        for len in 0..64 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + len * 11) as u8).collect();
            let _ = Reader::decode::<crate::ItemStack>(&bytes);
        }
    }
    #[test]
    fn wire_dimension_key_checked() {
        let mut out = Writer::new();
        out.put(&DimensionKey::OVERWORLD);
        assert_eq!(
            Reader::decode::<DimensionKey>(out.bytes()),
            Ok(DimensionKey::OVERWORLD)
        );
        assert!(Reader::decode::<DimensionKey>(&u32::MAX.to_le_bytes()).is_err());
        assert!(Reader::decode::<DimensionKey>(&[]).is_err());
    }
}

#[cfg(test)]
mod java_string_tests {
    use super::*;
    use sunrise_core::nbt::JavaString;

    fn units() -> Vec<Vec<u16>> {
        vec![
            vec![],
            vec![0],
            vec![0xD800],
            vec![0xDC00, 0x0041, 0xFFFF, 0x1234],
            (0..=1000u16).map(|n| n.wrapping_mul(97)).collect(),
        ]
    }

    /// The bytes are those of a `Vec<u16>`, so the host's reader and writer need no change.
    #[test]
    fn a_key_is_the_bytes_of_its_units_both_ways() {
        for units in units() {
            let mut expected = Writer::new();
            expected.put(&units);
            let mut got = Writer::new();
            got.put(&JavaString(units.clone()));
            assert_eq!(got.bytes(), expected.bytes());
            let key: JavaString = Reader::decode(expected.bytes()).unwrap();
            assert_eq!(key.0, units);
            assert_eq!(Reader::decode::<Vec<u16>>(got.bytes()).unwrap(), units);
            for end in 0..expected.bytes().len() {
                let cut = &expected.bytes()[..end];
                assert_eq!(
                    Reader::decode::<JavaString>(cut).map(|k| k.0),
                    Reader::decode::<Vec<u16>>(cut)
                );
            }
        }
    }

    #[test]
    fn a_key_longer_than_its_bytes_or_the_limit_is_refused_before_it_grows() {
        let mut huge = Writer::new();
        huge.put(&u32::MAX);
        assert_eq!(
            Reader::decode::<JavaString>(huge.bytes()).map(|k| k.0),
            Reader::decode::<Vec<u16>>(huge.bytes())
        );
        let key = JavaString(vec![7; 10]);
        for limit in 0..30 {
            let (mut a, mut b) = (Writer::new(), Writer::new());
            a.limit(limit);
            b.limit(limit);
            a.put(&key);
            b.put(&key.0);
            assert_eq!(a.error(), b.error(), "limit {limit}");
            if a.error().is_none() {
                assert_eq!(a.bytes(), b.bytes());
            }
        }
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;

    /// Vanilla's semantic bound on a tag is 512 nested lists or compounds with any leaf inside the
    /// innermost one, mixed-type lists included: the wire's nesting budget accepts all of it.
    #[test]
    fn a_tag_at_the_semantic_depth_limit_crosses() {
        use sunrise_core::nbt::{Compound, Tag};
        let mixed = Tag::List(vec![Tag::Byte(1), Tag::Short(128)]);
        let wrap_list = |inner: Tag| Tag::List(vec![inner]);
        let wrap_compound = |inner: Tag| {
            let mut compound = Compound::default();
            compound.insert("k".into(), inner);
            Tag::Compound(compound)
        };
        for wrap in [&wrap_list as &dyn Fn(Tag) -> Tag, &wrap_compound] {
            // `mixed` is itself one list: 511 wrappers make 512 nested containers.
            let mut tag = mixed.clone();
            for _ in 0..511 {
                tag = wrap(tag);
            }
            let mut out = Writer::new();
            out.put(&tag);
            let mut reader = Reader::new(out.bytes());
            assert!(reader.take::<Tag>().is_ok_and(|read| read == tag));
        }
    }
}

// Compound is defined in sunrise-core and cannot carry the vocabulary's Wire
// derive there. Encode the public map (length, then key/value pairs in traversal
// order), not the private hash table's storage. The typed decoder charges for
// compound storage, not BTreeMap nodes.
impl Wire for sunrise_core::nbt::Compound {
    layout_via!(
        std::collections::BTreeMap<sunrise_core::nbt::JavaString, crate::Tag>,
        "nbt-compound:u32-le-length:v1"
    );
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        out.payload_nested(|out| {
            if !out.charge_with(|budget| budget.compound(self.len())) {
                return;
            }
            out.put_len(self.len());
            for (key, value) in self {
                out.put(key);
                out.put(value);
            }
        });
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.payload_nested(|input| {
            let len = input.take_len(sunrise_core::nbt::JavaString::MIN + crate::Tag::MIN)?;
            input.charge_with(|budget| budget.compound(len))?;
            let mut map = Self::new();
            for _ in 0..len {
                let key = input.take()?;
                map.insert(key, input.take()?);
            }
            Ok(map)
        })
    }
}

// Re-exported types cannot carry a local derive. Keep their encoding explicit here.
impl crate::wire::Wire for crate::Tag {
    layout_fields!("tag" => u32, "0:End" => (), "1:Byte" => i8, "2:Short" => i16, "3:Int" => i32, "4:Long" => i64, "5:Float" => f32, "6:Double" => f64, "7:ByteArray" => Vec<u8>, "8:String" => sunrise_core::nbt::JavaString, "9:List" => Vec<crate::Tag>, "10:Compound" => sunrise_core::nbt::Compound, "11:IntArray" => Vec<i32>, "12:LongArray" => Vec<i64>);
    const MIN: usize = 4;
    fn put(&self, out: &mut Writer) {
        super::tag_tree::put(self, out);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        super::tag_tree::take(input)
    }
}
impl crate::wire::Wire for serde_json::Value {
    layout_via!(String, "validated-json-value:v1", ordinary_only);
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        if !out.ordinary_only() {
            return;
        }
        if !super::json_tree::value_depth([self], 0) {
            out.fail(WireError::Depth);
            return;
        }
        match serde_json::to_string(self) {
            Ok(text) => out.put_str(&text),
            Err(_) => out.fail(WireError::Invalid("not valid JSON")),
        }
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        super::json_tree::take(input)
    }
}
impl crate::wire::Wire for crate::Direction {
    layout_fields!("tag" => u32, "0:Down" => (), "1:Up" => (), "2:North" => (), "3:South" => (), "4:West" => (), "5:East" => ());
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&match self {
            Self::Down => 0u32,
            Self::Up => 1u32,
            Self::North => 2u32,
            Self::South => 3u32,
            Self::West => 4u32,
            Self::East => 5u32,
        });
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        Ok(match input.take::<u32>()? {
            0 => Self::Down,
            1 => Self::Up,
            2 => Self::North,
            3 => Self::South,
            4 => Self::West,
            5 => Self::East,
            tag => return Err(crate::wire::WireError::Tag(tag)),
        })
    }
}
impl crate::wire::Wire for crate::BlockPos {
    layout_fields!("x" => i32, "y" => i32, "z" => i32);
    const MIN: usize = <i32 as crate::wire::Wire>::MIN
        + <i32 as crate::wire::Wire>::MIN
        + <i32 as crate::wire::Wire>::MIN;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&self.x);
        out.put(&self.y);
        out.put(&self.z);
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.record(|input| {
            Ok(Self {
                x: input.take()?,
                y: input.take()?,
                z: input.take()?,
            })
        })
    }
}
impl crate::wire::Wire for crate::EquipmentSlot {
    layout_fields!("tag" => u32, "0:MainHand" => (), "1:OffHand" => (), "2:Feet" => (), "3:Legs" => (), "4:Chest" => (), "5:Head" => (), "6:Body" => (), "7:Saddle" => ());
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&match self {
            Self::MainHand => 0u32,
            Self::OffHand => 1u32,
            Self::Feet => 2u32,
            Self::Legs => 3u32,
            Self::Chest => 4u32,
            Self::Head => 5u32,
            Self::Body => 6u32,
            Self::Saddle => 7u32,
        });
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        Ok(match input.take::<u32>()? {
            0 => Self::MainHand,
            1 => Self::OffHand,
            2 => Self::Feet,
            3 => Self::Legs,
            4 => Self::Chest,
            5 => Self::Head,
            6 => Self::Body,
            7 => Self::Saddle,
            tag => return Err(crate::wire::WireError::Tag(tag)),
        })
    }
}
impl crate::wire::Wire for crate::GameMode {
    layout_fields!("tag" => u32, "0:Survival" => (), "1:Creative" => (), "2:Adventure" => (), "3:Spectator" => ());
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&match self {
            Self::Survival => 0u32,
            Self::Creative => 1u32,
            Self::Adventure => 2u32,
            Self::Spectator => 3u32,
        });
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        Ok(match input.take::<u32>()? {
            0 => Self::Survival,
            1 => Self::Creative,
            2 => Self::Adventure,
            3 => Self::Spectator,
            tag => return Err(crate::wire::WireError::Tag(tag)),
        })
    }
}
impl crate::wire::Wire for crate::NeighbourUpdate {
    layout_fields!("tag" => u32, "0:OutputSignal" => (), "1:Neighbours" => (), "2:NeighboursExcept" => crate::Direction);
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        match self {
            Self::OutputSignal => out.put(&0u32),
            Self::Neighbours => out.put(&1u32),
            Self::NeighboursExcept(f0) => {
                out.put(&2u32);
                out.put(f0);
            }
        }
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.record(|input| {
            Ok(match input.take::<u32>()? {
                0 => Self::OutputSignal,
                1 => Self::Neighbours,
                2 => Self::NeighboursExcept(input.take()?),
                tag => return Err(crate::wire::WireError::Tag(tag)),
            })
        })
    }
}
impl crate::wire::Wire for crate::ChunkPos {
    layout_fields!("x" => i32, "z" => i32);
    const MIN: usize = <i32 as crate::wire::Wire>::MIN + <i32 as crate::wire::Wire>::MIN;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&self.x);
        out.put(&self.z);
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.record(|input| {
            Ok(Self {
                x: input.take()?,
                z: input.take()?,
            })
        })
    }
}
impl crate::wire::Wire for crate::BoundingBox {
    layout_fields!("min" => [i32;3], "max" => [i32;3]);
    const MIN: usize = <[i32; 3] as crate::wire::Wire>::MIN + <[i32; 3] as crate::wire::Wire>::MIN;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&self.min);
        out.put(&self.max);
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.record(|input| {
            Ok(Self {
                min: input.take()?,
                max: input.take()?,
            })
        })
    }
}
impl crate::wire::Wire for crate::LevelDimension {
    layout_fields!(ordinary_only "level-dimension:v1"; "tag" => u32, "0:Overworld" => (), "1:Nether" => (), "2:End" => (), "3:Custom" => crate::LevelKey);
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        if !out.ordinary_only() {
            return;
        }
        match self {
            Self::Overworld => out.put(&0u32),
            Self::Nether => out.put(&1u32),
            Self::End => out.put(&2u32),
            Self::Custom(f0) => {
                out.put(&3u32);
                out.put(f0);
            }
        }
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.ordinary_only()?;
        input.record(|input| {
            Ok(match input.take::<u32>()? {
                0 => Self::Overworld,
                1 => Self::Nether,
                2 => Self::End,
                3 => Self::Custom(input.take()?),
                tag => return Err(crate::wire::WireError::Tag(tag)),
            })
        })
    }
}
impl crate::wire::Wire for serde_json::Map<String, serde_json::Value> {
    layout_via!(String, "validated-json-object:v1", ordinary_only);
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        if !out.ordinary_only() {
            return;
        }
        if !super::json_tree::value_depth(self.values(), 1) {
            out.fail(WireError::Depth);
            return;
        }
        match serde_json::to_string(self) {
            Ok(text) => out.put_str(&text),
            Err(_) => out.fail(WireError::Invalid("not valid JSON")),
        }
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        super::json_tree::take(input)
    }
}
impl crate::wire::Wire for crate::UseAnimation {
    layout_fields!("tag" => u32, "0:None" => (), "1:Eat" => (), "2:Drink" => (), "3:Block" => (), "4:Bow" => (), "5:Trident" => (), "6:Crossbow" => (), "7:Spyglass" => (), "8:TootHorn" => (), "9:Brush" => (), "10:Bundle" => (), "11:Spear" => ());
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&match self {
            Self::None => 0u32,
            Self::Eat => 1u32,
            Self::Drink => 2u32,
            Self::Block => 3u32,
            Self::Bow => 4u32,
            Self::Trident => 5u32,
            Self::Crossbow => 6u32,
            Self::Spyglass => 7u32,
            Self::TootHorn => 8u32,
            Self::Brush => 9u32,
            Self::Bundle => 10u32,
            Self::Spear => 11u32,
        });
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        Ok(match input.take::<u32>()? {
            0 => Self::None,
            1 => Self::Eat,
            2 => Self::Drink,
            3 => Self::Block,
            4 => Self::Bow,
            5 => Self::Trident,
            6 => Self::Crossbow,
            7 => Self::Spyglass,
            8 => Self::TootHorn,
            9 => Self::Brush,
            10 => Self::Bundle,
            11 => Self::Spear,
            tag => return Err(crate::wire::WireError::Tag(tag)),
        })
    }
}
impl crate::wire::Wire for crate::ConnectionKind {
    layout_fields!("tag" => u32, "0:Fence" => (), "1:Wall" => (), "2:Bars" => ());
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        out.put(&match self {
            Self::Fence => 0u32,
            Self::Wall => 1u32,
            Self::Bars => 2u32,
            _ => {
                out.fail(crate::wire::WireError::Invalid("unknown ConnectionKind"));
                return;
            }
        });
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        Ok(match input.take::<u32>()? {
            0 => Self::Fence,
            1 => Self::Wall,
            2 => Self::Bars,
            tag => return Err(crate::wire::WireError::Tag(tag)),
        })
    }
}

#[allow(unreachable_patterns, unused_variables)]
impl crate::wire::Wire for crate::TicketOwner {
    layout_fields!("tag" => u32, "0:Block" => [i32;3], "1:Entity" => u128);
    const MIN: usize = 4;
    fn put(&self, out: &mut crate::wire::Writer) {
        match self {
            Self::Block(f0) => {
                out.put(&0u32);
                out.put(f0);
            }
            Self::Entity(f0) => {
                out.put(&1u32);
                out.put(f0);
            }
        }
    }
    fn take(
        input: &mut crate::wire::Reader<'_>,
    ) -> ::std::result::Result<Self, crate::wire::WireError> {
        input.record(|input| {
            Ok(match input.take::<u32>()? {
                0 => Self::Block(input.take()?),
                1 => Self::Entity(input.take()?),
                tag => return Err(crate::wire::WireError::Tag(tag)),
            })
        })
    }
}

// Biome keys carry a slot and an engine-owned generation. Validate on the receiving host.
impl Wire for crate::BiomeKey {
    layout_fields!("slot" => u32, "generation" => u64);
    const MIN: usize = <u32 as Wire>::MIN + <u64 as Wire>::MIN;
    fn put(&self, out: &mut Writer) {
        out.put(&self.0);
        out.put(&self.1);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(Self(input.take()?, input.take()?))
    }
}

#[cfg(test)]
#[path = "wire_layout_tests.rs"]
mod layout_tests;
