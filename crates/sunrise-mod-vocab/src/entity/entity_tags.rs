use std::fmt;
use sunrise_core::nbt::Tag;

// no wire: borrows the live entity's saved tags from the host.
/// A borrowed view of an entity's scoreboard tags (`Entity.entityTags()`).
/// Equality compares set membership by exact UTF-16 code units, ignoring order and
/// duplicates. Debug output lists the string tags in saved order.
///
/// For an ordered comparison, use `a.iter_utf16().eq(b.iter_utf16())`. To match
/// the old `Vec<String>` equality, map both iterators through
/// [`String::from_utf16_lossy`] before comparing them.
// exhaustive: its field is private
#[derive(Clone, Copy)]
pub struct EntityTags<'a>(&'a [Tag]);

impl<'a> EntityTags<'a> {
    /// Visit string tags in saved order as their exact UTF-16 code units.
    /// Non-string entries are skipped. An unpaired surrogate is not replaced.
    pub fn iter_utf16(self) -> impl Iterator<Item = &'a [u16]> {
        self.0.iter().filter_map(|tag| match tag {
            Tag::String(text) => Some(text.0.as_slice()),
            _ => None,
        })
    }

    /// Whether the view contains no string tags.
    pub fn is_empty(&self) -> bool {
        self.iter_utf16().next().is_none()
    }

    /// Compare a Rust string's UTF-16 code units with the saved tags without
    /// allocating or replacing unpaired surrogates.
    pub fn contains(&self, tag: &str) -> bool {
        self.iter_utf16()
            .any(|units| units.iter().copied().eq(tag.encode_utf16()))
    }
}

impl fmt::Debug for EntityTags<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter_utf16()).finish()
    }
}

impl PartialEq for EntityTags<'_> {
    fn eq(&self, other: &Self) -> bool {
        let is_subset = |left: Self, right: Self| {
            left.iter_utf16()
                .all(|tag| right.iter_utf16().any(|candidate| candidate == tag))
        };
        is_subset(*self, *other) && is_subset(*other, *self)
    }
}

impl Eq for EntityTags<'_> {}

/// Lend the game's saved tag list without exposing NBT to mods.
pub fn borrow_entity_tags(tags: &[Tag]) -> EntityTags<'_> {
    EntityTags(tags)
}

#[cfg(test)]
#[test]
fn entity_tag_view_preserves_saved_order_and_exact_strings() {
    use sunrise_core::nbt::JavaString;

    let saved = [
        Tag::String(JavaString(vec![0xD800])),
        Tag::Byte(7),
        Tag::String("x".into()),
    ];
    let tags = borrow_entity_tags(&saved);
    assert!(!tags.is_empty());
    assert!(!borrow_entity_tags(&[Tag::String("".into())]).is_empty());
    assert!(borrow_entity_tags(&[Tag::Byte(3)]).is_empty());
    let expected: [&[u16]; 2] = [&[0xD800], &[0x78]];
    assert!(tags.iter_utf16().eq(expected));
    assert!(!tags.contains("\u{fffd}"));
    assert!(
        tags.iter_utf16()
            .any(|units| String::from_utf16_lossy(units) == "\u{fffd}")
    );
    assert_eq!(format!("{tags:?}"), "[[55296], [120]]");
}

#[cfg(test)]
#[test]
fn entity_tag_view_equality_uses_exact_utf16_set_membership() {
    use sunrise_core::nbt::JavaString;

    let surrogate = Tag::String(JavaString(vec![0xD800]));
    let x = Tag::String("x".into());
    let original = [surrogate.clone(), Tag::Byte(7), x.clone()];
    let tags = borrow_entity_tags(&original);
    let reordered = [x.clone(), surrogate, x.clone()];
    let reordered_tags = borrow_entity_tags(&reordered);
    assert_eq!(tags, reordered_tags);
    assert_eq!(reordered_tags, tags);
    let expected: [&[u16]; 3] = [&[0x78], &[0xD800], &[0x78]];
    assert!(reordered_tags.iter_utf16().eq(expected));

    let subset = borrow_entity_tags(std::slice::from_ref(&x));
    assert_ne!(tags, subset);
    assert_ne!(subset, tags);
    assert_ne!(
        tags,
        borrow_entity_tags(&[Tag::String(JavaString(vec![0xD801])), x.clone()])
    );
    assert_ne!(
        tags,
        borrow_entity_tags(&[Tag::String("\u{fffd}".into()), x])
    );
}
