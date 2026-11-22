//! Build typed codec values from streams, then hash their validated persistent form.
use super::{Names, Stack};
use crate::hash_ops::{HashCode, HashOps};
use sunrise_core::nbt::Tag;
use sunrise_world::component_value::Value;

pub(crate) trait Ops: Sized {
    type Value: Clone;
    type Container;
    fn empty_map() -> Self::Value;
    fn create_byte(v: i8) -> Self::Value;
    fn create_int(v: i32) -> Self::Value;
    fn create_f32(v: f32) -> Self::Value;
    fn create_double(v: u64) -> Self::Value;
    fn create_boolean(v: bool) -> Self::Value;
    fn create_string(v: &str) -> Self::Value;
    fn create_string_units(v: &[u16]) -> Self::Value;
    fn create_int_list(v: &[i32]) -> Self::Value;
    fn create_map(v: Vec<(Self::Value, Self::Value)>) -> Self::Value;
    fn create_record(v: Vec<(Self::Value, Self::Value)>) -> Self::Value;
    fn create_list(v: impl IntoIterator<Item = Self::Value>) -> Self::Value;
    fn from_nbt(v: &Tag, bools: &[&str]) -> Option<Self::Value>;
    fn saved_component(name: &str, tag: &Tag) -> Option<Self::Value>;
    fn template(stack: &Stack, names: &dyn Names) -> Option<Self::Value>;
    fn registered_consume_effect(
        effect: sunrise_world::component_value::consume_effects::Admitted,
    ) -> Option<Self::Value> {
        Self::from_nbt(&effect.saved, &[])
    }
    fn special_component(
        _: i32,
        _: &super::Component,
        _: &dyn Names,
    ) -> Option<Option<Self::Value>> {
        None
    }
    fn stream_only(_: Self::Value) -> Option<Self::Value> {
        None
    }
    fn set(v: Vec<Self::Value>) -> Self::Value {
        Self::create_list(v)
    }
    fn container_new(size: usize) -> Self::Container;
    fn container_slot(
        slots: &mut Self::Container,
        slot: usize,
        item: Option<Self::Value>,
    ) -> Option<()>;
    fn container_build(slots: Self::Container) -> Self::Value;
}
pub(crate) struct Values;
impl Values {
    pub(super) fn hash_component(name: &str, value: &Value) -> Option<HashCode> {
        sunrise_world::component_value::source::validate_stream_value(name, value).ok()?;
        Self::hash(value)
    }

    /// The saved codec has already validated this tree. Retain the types that NBT
    /// alone can't represent, notably `Codec.BOOL` versus `Codec.BYTE`.
    fn hash(value: &Value) -> Option<HashCode> {
        sunrise_core::nbt::with_tree_stack(|| Self::hash_with_stack(value))
    }

    fn hash_with_stack(value: &Value) -> Option<HashCode> {
        use Value::*;
        Some(match value {
            Empty => HashOps::empty(),
            Byte(v) => HashOps::create_byte(*v),
            Short(v) => HashOps::create_short(*v),
            Int(v) => HashOps::create_int(*v),
            Long(v) => HashOps::create_long(*v),
            Float(v) => HashOps::create_float(*v),
            Double(v) => HashOps::create_double(*v),
            Bool(v) => HashOps::create_boolean(*v),
            String(v) => HashOps::create_string_units(v),
            ByteArray(v) => HashOps::create_byte_list(v),
            IntArray(v) => HashOps::create_int_list(v),
            LongArray(v) => HashOps::create_long_list(v),
            List(v) | Set(v) => {
                HashOps::create_list(v.iter().map(Self::hash).collect::<Option<Vec<_>>>()?)
            }
            Map(v) => HashOps::create_map(
                v.iter()
                    .map(|(k, v)| Some((Self::hash(k)?, Self::hash(v)?)))
                    .collect::<Option<_>>()?,
            ),
            RegisteredConsumeEffect(effect) => HashOps::from_nbt(&effect.saved, &[]),
            Template(t) => Self::template_hash(t)?,
            Container(slots) => HashOps::create_list(
                slots
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, item)| Some((slot, item.as_ref()?)))
                    .map(|(slot, item)| {
                        Some(HashOps::create_map(vec![
                            (
                                HashOps::create_string("slot"),
                                HashOps::create_int(slot as i32),
                            ),
                            (HashOps::create_string("item"), Self::template_hash(item)?),
                        ]))
                    })
                    .collect::<Option<Vec<_>>>()?,
            ),
            _ => return None,
        })
    }

    fn template_hash(template: &sunrise_world::component_value::Template) -> Option<HashCode> {
        use sunrise_world::component_value::{is_persistent, source::validate_template_count};
        validate_template_count(template.count).ok()?;
        let mut record =
            crate::hash_ops::RecordHash::new().field("id", HashOps::create_string(&template.item));
        if template.count != 1 {
            record = record.field("count", HashOps::create_int(template.count));
        }
        // Both typed stream patch conversion and saved source have
        // already retained exactly one entry per name.
        let entries: Vec<_> = template
            .patch
            .iter()
            .filter(|(name, _)| is_persistent(name))
            .map(|(name, value)| {
                Some(match value {
                    Some(value) => (
                        HashOps::create_string(name),
                        Self::hash_component(name, value)?,
                    ),
                    None => (
                        HashOps::create_string(&format!("!{name}")),
                        HashOps::empty_map(),
                    ),
                })
            })
            .collect::<Option<_>>()?;
        if !entries.is_empty() {
            record = record.field("components", HashOps::create_map(entries));
        }
        Some(record.build())
    }
}
impl Ops for Values {
    type Value = Value;
    type Container = Vec<Option<sunrise_world::component_value::Template>>;
    fn empty_map() -> Value {
        Value::Map(Vec::new())
    }
    fn create_byte(v: i8) -> Value {
        Value::Byte(v)
    }
    fn create_int(v: i32) -> Value {
        Value::Int(v)
    }
    fn create_f32(v: f32) -> Value {
        Value::Float(v.to_bits())
    }
    fn create_double(v: u64) -> Value {
        Value::Double(v)
    }
    fn create_boolean(v: bool) -> Value {
        Value::Bool(v)
    }
    fn create_string(v: &str) -> Value {
        Value::string(v)
    }
    fn create_string_units(v: &[u16]) -> Value {
        Value::String(v.to_vec())
    }
    fn create_int_list(v: &[i32]) -> Value {
        Value::IntArray(v.to_vec())
    }
    fn create_map(mut v: Vec<(Value, Value)>) -> Value {
        sunrise_world::component_value::normalize_map(&mut v, |(key, _)| {
            let Value::String(units) = key else {
                unreachable!("all stream codec maps have string keys");
            };
            units.as_slice()
        });
        Value::Map(v)
    }
    fn create_record(fields: Vec<(Value, Value)>) -> Value {
        Value::Map(fields)
    }
    fn create_list(v: impl IntoIterator<Item = Value>) -> Value {
        Value::List(v.into_iter().collect())
    }
    fn from_nbt(v: &Tag, bools: &[&str]) -> Option<Value> {
        Some(Value::from_nbt(v, bools))
    }
    fn registered_consume_effect(
        effect: sunrise_world::component_value::consume_effects::Admitted,
    ) -> Option<Value> {
        Some(Value::RegisteredConsumeEffect(effect))
    }
    fn saved_component(name: &str, tag: &Tag) -> Option<Value> {
        sunrise_world::component_value::source::from_component_tag_with(
            name,
            tag,
            crate::item::persistent::is_native_component,
        )
        .ok()
    }
    fn template(stack: &Stack, names: &dyn Names) -> Option<Value> {
        crate::item::persistent::template_value(stack, names)
            .ok()
            .map(|v| Value::Template(Box::new(v)))
    }
    fn special_component(
        kind: i32,
        value: &super::Component,
        names: &dyn Names,
    ) -> Option<Option<Value>> {
        crate::item::persistent::special_value(kind, value, names)
    }
    fn set(v: Vec<Value>) -> Value {
        Value::Set(v)
    }
    fn stream_only(v: Value) -> Option<Value> {
        Some(Value::StreamOnly(Box::new(v)))
    }
    fn container_new(size: usize) -> Self::Container {
        Vec::with_capacity(size)
    }
    fn container_slot(slots: &mut Self::Container, _: usize, item: Option<Value>) -> Option<()> {
        slots.push(match item {
            Some(item) => Some(item.into_template()?),
            None => None,
        });
        Some(())
    }
    fn container_build(slots: Self::Container) -> Value {
        Value::Container(slots)
    }
}
pub(crate) struct Record<O: Ops>(Vec<(O::Value, O::Value)>);
impl<O: Ops> Record<O> {
    pub(crate) fn new() -> Self {
        Self(Vec::new())
    }
    pub(crate) fn field(mut self, name: &str, value: O::Value) -> Self {
        self.0.push((O::create_string(name), value));
        self
    }
    pub(crate) fn optional(self, name: &str, value: Option<O::Value>) -> Self {
        match value {
            Some(v) => self.field(name, v),
            None => self,
        }
    }
    pub(crate) fn build(self) -> O::Value {
        O::create_record(self.0)
    }
}
