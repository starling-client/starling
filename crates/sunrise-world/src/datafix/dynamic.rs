//! What DataFixerUpper's `Dynamic` over `NbtOps` does with values: number and
//! string views, maps and lists, field edits. Fixes use these to stay close to
//! the Java they port.
use sunrise_core::nbt::{Compound, JavaString, Tag};

pub fn key(s: &str) -> JavaString {
    JavaString::from(s)
}
pub fn string(s: &str) -> Tag {
    Tag::String(JavaString::from(s))
}
pub fn empty_map() -> Tag {
    Tag::Compound(Compound::new())
}
pub fn boolean(b: bool) -> Tag {
    Tag::Byte(i8::from(b))
}
pub fn int_list(values: &[i32]) -> Tag {
    Tag::IntArray(values.to_vec())
}

/// A Java `Number` from a numeric tag (`Tag.asNumber`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
}
impl Number {
    pub fn int_value(self) -> i32 {
        match self {
            Self::Byte(v) => i32::from(v),
            Self::Short(v) => i32::from(v),
            Self::Int(v) => v,
            Self::Long(v) => v as i32,
            Self::Float(v) => v as i32,
            Self::Double(v) => v as i32,
        }
    }
    pub fn long_value(self) -> i64 {
        match self {
            Self::Byte(v) => i64::from(v),
            Self::Short(v) => i64::from(v),
            Self::Int(v) => i64::from(v),
            Self::Long(v) => v,
            Self::Float(v) => v as i64,
            Self::Double(v) => v as i64,
        }
    }
    pub fn double_value(self) -> f64 {
        match self {
            Self::Byte(v) => f64::from(v),
            Self::Short(v) => f64::from(v),
            Self::Int(v) => f64::from(v),
            Self::Long(v) => v as f64,
            Self::Float(v) => f64::from(v),
            Self::Double(v) => v,
        }
    }
    pub fn float_value(self) -> f32 {
        match self {
            Self::Double(v) => v as f32,
            Self::Long(v) => v as f32,
            other => other.double_value() as f32,
        }
    }
    pub fn byte_value(self) -> i8 {
        match self {
            Self::Byte(v) => v,
            Self::Short(v) => v as i8,
            Self::Int(v) => v as i8,
            Self::Long(v) => v as i8,
            Self::Float(v) => (v as i32) as i8,
            Self::Double(v) => (v as i32) as i8,
        }
    }
    /// `Number.toString` of the boxed value.
    pub fn java_string(self) -> String {
        match self {
            Self::Byte(v) => v.to_string(),
            Self::Short(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Long(v) => v.to_string(),
            Self::Float(v) => sunrise_core::java_number::float_to_string(v),
            Self::Double(v) => sunrise_core::java_number::double_to_string(v),
        }
    }
}

pub fn as_number(tag: &Tag) -> Option<Number> {
    Some(match tag {
        Tag::Byte(v) => Number::Byte(*v),
        Tag::Short(v) => Number::Short(*v),
        Tag::Int(v) => Number::Int(*v),
        Tag::Long(v) => Number::Long(*v),
        Tag::Float(v) => Number::Float(f32::from_bits(*v)),
        Tag::Double(v) => Number::Double(f64::from_bits(*v)),
        _ => return None,
    })
}
pub fn as_int(tag: Option<&Tag>, default: i32) -> i32 {
    tag.and_then(as_number).map_or(default, Number::int_value)
}
pub fn as_long(tag: Option<&Tag>, default: i64) -> i64 {
    tag.and_then(as_number).map_or(default, Number::long_value)
}
pub fn as_double(tag: Option<&Tag>, default: f64) -> f64 {
    tag.and_then(as_number)
        .map_or(default, Number::double_value)
}
pub fn as_float(tag: Option<&Tag>, default: f32) -> f32 {
    tag.and_then(as_number).map_or(default, Number::float_value)
}
/// `NbtOps.getBooleanValue`: a number that is not zero.
pub fn as_bool_opt(tag: Option<&Tag>) -> Option<bool> {
    tag.and_then(as_number).map(|n| n.double_value() != 0.0)
}
pub fn as_bool(tag: Option<&Tag>, default: bool) -> bool {
    as_bool_opt(tag).unwrap_or(default)
}
pub fn as_str(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
pub fn as_string(tag: Option<&Tag>, default: &str) -> String {
    as_str(tag).unwrap_or_else(|| default.to_owned())
}

/// `NbtOps.getStream`: the elements of a list or of a number array.
pub fn stream(tag: Option<&Tag>) -> Vec<Tag> {
    match tag {
        Some(Tag::List(v)) => v.clone(),
        Some(Tag::ByteArray(v)) => v.iter().map(|b| Tag::Byte(*b as i8)).collect(),
        Some(Tag::IntArray(v)) => v.iter().map(|i| Tag::Int(*i)).collect(),
        Some(Tag::LongArray(v)) => v.iter().map(|l| Tag::Long(*l)).collect(),
        _ => Vec::new(),
    }
}
pub fn is_list(tag: Option<&Tag>) -> bool {
    matches!(
        tag,
        Some(Tag::List(_) | Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_))
    )
}

pub fn get<'a>(tag: &'a Tag, field: &str) -> Option<&'a Tag> {
    tag.get(field)
}
pub fn map(tag: &Tag) -> Option<&Compound> {
    match tag {
        Tag::Compound(c) => Some(c),
        _ => None,
    }
}
/// `Dynamic.set` with a string key: `NbtOps.mergeToMap` shallow-copies the
/// input compound before putting the value. End starts a fresh compound.
pub fn set(tag: &mut Tag, field: &str, value: Tag) {
    match tag {
        Tag::End => *tag = Tag::compound([(field, value)]),
        Tag::Compound(c) => {
            *c = std::mem::take(c).into_shallow_copy();
            c.insert(key(field), value);
        }
        _ => {}
    }
}
/// `NbtOps.remove`: size the shallow copy before removing the entry, even if
/// the field is absent. Move the removed value out for the Rust fix's caller.
pub fn remove(tag: &mut Tag, field: &str) -> Option<Tag> {
    let Tag::Compound(c) = tag else { return None };
    *c = std::mem::take(c).into_shallow_copy();
    c.remove(&key(field))
}
/// Put a field directly, for NBT writers and translators outside DynamicOps.
pub fn put(tag: &mut Tag, field: &str, value: Tag) {
    match tag {
        Tag::End => *tag = Tag::compound([(field, value)]),
        Tag::Compound(c) => {
            c.insert(key(field), value);
        }
        _ => {}
    }
}
/// Take a field directly, keeping the compound's table history.
pub fn take(tag: &mut Tag, field: &str) -> Option<Tag> {
    match tag {
        Tag::Compound(c) => c.remove(&key(field)),
        _ => None,
    }
}
pub fn has(tag: &Tag, field: &str) -> bool {
    tag.get(field).is_some()
}
/// `Dynamic.renameField`.
pub fn rename(tag: &mut Tag, from: &str, to: &str) {
    if let Some(v) = remove(tag, from) {
        set(tag, to, v);
    }
}
/// `Dynamic.update`: replace a present field in a shallow copy, retaining
/// the key's position in the copy. A missing field leaves the map untouched.
pub fn update(tag: &mut Tag, field: &str, f: impl FnOnce(Tag) -> Tag) {
    if let Tag::Compound(c) = tag
        && let Some(value) = c.get_mut(&key(field))
    {
        *value = f(std::mem::replace(value, Tag::End));
        *c = std::mem::take(c).into_shallow_copy();
    }
}
/// `Dynamic.renameAndFixField`: the fixed value under the new name, removed when
/// the fix gives nothing.
pub fn rename_and_fix(tag: &mut Tag, from: &str, to: &str, f: impl FnOnce(Tag) -> Option<Tag>) {
    if let Some(v) = remove(tag, from)
        && let Some(v) = f(v)
    {
        set(tag, to, v);
    }
}
/// `Dynamic.copyField`.
pub fn copy_field(from: &Tag, field: &str, to: &mut Tag, to_field: &str) {
    if let Some(v) = from.get(field) {
        set(to, to_field, v.clone());
    }
}
/// `Dynamic.equals(emptyMap())`: the map a record remembers it was read from
/// (see `datafix::mark_read`) is not part of the value.
pub fn is_empty_map(tag: &Tag) -> bool {
    matches!(tag, Tag::Compound(c) if c.keys().all(|k| *k == key(super::LAST_READ)))
}
/// `Dynamic.orElseEmptyMap` of an optional field.
pub fn or_empty_map(tag: Option<&Tag>) -> Tag {
    tag.cloned().unwrap_or_else(empty_map)
}
/// `NamespacedSchema.ensureNamespaced`.
pub fn namespaced(id: &str) -> String {
    match crate::datafix::identifier(id) {
        Some((ns, path)) => format!("{ns}:{path}"),
        None => id.to_owned(),
    }
}

/// `Dynamic.asLongStreamOpt` over `NbtOps`: a long array, or a list or array
/// whose elements are all numbers.
pub fn as_long_stream(tag: Option<&Tag>) -> Option<Vec<i64>> {
    match tag? {
        Tag::LongArray(v) => Some(v.clone()),
        t if is_list(Some(t)) => stream(Some(t))
            .iter()
            .map(|e| as_number(e).map(Number::long_value))
            .collect(),
        _ => None,
    }
}
/// `Dynamic.asIntStreamOpt` over `NbtOps`.
pub fn as_int_stream(tag: Option<&Tag>) -> Option<Vec<i32>> {
    match tag? {
        Tag::IntArray(v) => Some(v.clone()),
        t if is_list(Some(t)) => stream(Some(t))
            .iter()
            .map(|e| as_number(e).map(Number::int_value))
            .collect(),
        _ => None,
    }
}
/// `Dynamic.asByteBufferOpt` over `NbtOps`.
pub fn as_byte_buffer(tag: Option<&Tag>) -> Option<Vec<u8>> {
    match tag? {
        Tag::ByteArray(v) => Some(v.clone()),
        t if is_list(Some(t)) => stream(Some(t))
            .iter()
            .map(|e| as_number(e).map(|n| n.byte_value() as u8))
            .collect(),
        _ => None,
    }
}
/// `ChunkHeightAndBiomeFix.ceillog2` (`Math.ceil(Math.log(n) / Math.log(2))`,
/// which is exact for every palette size).
pub fn ceillog2(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(tag: &Tag) -> Vec<String> {
        map(tag)
            .unwrap()
            .keys()
            .map(JavaString::to_string_lossy)
            .collect()
    }

    #[test]
    fn merge_and_update_size_the_copy_from_the_input_and_only_reorder_the_outer_table() {
        let initial = Tag::compound([
            ("zeta", Tag::Int(0)),
            ("alpha", Tag::Int(1)),
            (
                "mid",
                Tag::compound([
                    ("zeta", Tag::Int(2)),
                    ("alpha", Tag::Int(3)),
                    ("mid", Tag::Int(4)),
                ]),
            ),
        ]);
        let mut merged = initial.clone();
        set(&mut merged, "new", Tag::Int(5));
        // Size 3 gives 4 buckets. The new key grows it to 8, where `mid`
        // and `new` share a bucket. The nested map still has 16 buckets.
        assert_eq!(keys(&merged), ["zeta", "mid", "new", "alpha"]);
        assert_eq!(keys(merged.get("mid").unwrap()), ["zeta", "alpha", "mid"]);

        let mut updated = initial;
        update(&mut updated, "alpha", |_| Tag::Int(6));
        assert_eq!(keys(&updated), ["zeta", "mid", "alpha"]);
        assert_eq!(updated.get("alpha"), Some(&Tag::Int(6)));
    }

    #[test]
    fn remove_sizes_the_copy_before_removal_even_for_a_missing_key() {
        let initial = Tag::compound([
            ("zeta", Tag::Int(0)),
            ("alpha", Tag::Int(1)),
            ("mid", Tag::Int(2)),
            ("d", Tag::Int(3)),
        ]);
        let mut unchanged = initial.clone();
        assert_eq!(remove(&mut unchanged, "missing"), None);
        assert_eq!(keys(&unchanged), ["zeta", "mid", "d", "alpha"]);
        // Removing first and sizing for 3 keys would place `d` before `mid`.
        let mut pruned = initial;
        assert_eq!(remove(&mut pruned, "zeta"), Some(Tag::Int(0)));
        assert_eq!(keys(&pruned), ["mid", "d", "alpha"]);
    }
}
