//! The persistent `CustomData.CODEC`, shared by recipes and live stack boundaries.
use std::borrow::Cow;
use sunrise_core::nbt::{
    Compound, JavaString, Tag, discard_tag, parse_snbt_utf16, snbt_whitespace,
};

/// `CustomData.COMPOUND_TAG_CODEC`: compounds or fully parsed flattened SNBT.
/// Compound payloads are opaque NBT, including unknown keys and Java UTF-16.
pub fn decode(value: Tag) -> Result<Tag, String> {
    match value {
        value @ Tag::Compound(_) => Ok(value),
        Tag::String(text) => flattened(&text).map(Tag::Compound),
        _ => Err("Not a compound tag or SNBT string".into()),
    }
}

/// Validate a borrowed payload without copying opaque compound trees.
pub fn validate(value: &Tag) -> Result<(), String> {
    if let Cow::Owned(compound) = borrow(value)? {
        discard_tag(Tag::Compound(compound));
    }
    Ok(())
}

/// Borrow an ordinary compound, decoding only the flattened SNBT representation.
pub fn borrow(value: &Tag) -> Result<Cow<'_, Compound>, String> {
    match value {
        Tag::Compound(compound) => Ok(Cow::Borrowed(compound)),
        Tag::String(text) => flattened(text).map(Cow::Owned),
        _ => Err("Not a compound tag or SNBT string".into()),
    }
}

fn flattened(text: &JavaString) -> Result<Compound, String> {
    let (value, used) = parse_snbt_utf16(&text.0).map_err(|error| error.to_string())?;
    if !text.0[used..]
        .iter()
        .copied()
        .all(|unit| snbt_whitespace(u32::from(unit)))
    {
        discard_tag(value);
        return Err("argument.nbt.trailing".into());
    }
    match value {
        Tag::Compound(value) => Ok(value),
        invalid => {
            discard_tag(invalid);
            Err("Expected compound tag".into())
        }
    }
}
