//! Typed partial block predicates for prepared bulk edits.
use crate::{BlockKey, BlockStateError, StateConstraint, TagKey, content::registry_keys};

/// A block or tag filter, validated on construction, with unspecified properties left as
/// wildcards. An unknown or missing property on a tag member never matches.
// no wire: a native filter over process-local block and tag keys.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StatePredicate {
    target: Target,
    constraints: Vec<StateConstraint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Block(BlockKey),
    Tag(TagKey),
}
impl StatePredicate {
    /// A filter validated against the game's resolved definition for `block`.
    /// Unknown or duplicate properties and illegal values are refused. Omitted properties
    /// are wildcards, so a partial filter need not form a complete state with defaults.
    /// Returns [`BlockStateError::Unavailable`] before block definitions are published.
    pub fn block(
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Result<Self, BlockStateError> {
        let blocks = registry_keys::block_definitions().ok_or(BlockStateError::Unavailable)?;
        let definition = blocks
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?;
        let properties: Vec<_> = constraints
            .iter()
            .map(|constraint| (constraint.name.as_str(), constraint.value.as_str()))
            .collect();
        definition.validate_properties(&properties)?;
        Ok(Self {
            target: Target::Block(block),
            constraints: constraints.to_vec(),
        })
    }
    /// A tag filter; duplicate properties are refused, but arbitrary properties and empty tags are valid.
    /// Membership is evaluated against authoritative tags at application, not frozen here.
    pub fn tag(tag: TagKey, constraints: &[StateConstraint]) -> Result<Self, BlockStateError> {
        for (input, constraint) in constraints.iter().enumerate() {
            if constraints[..input]
                .iter()
                .any(|previous| previous.name == constraint.name)
            {
                return Err(BlockStateError::DuplicateProperty(input as u32));
            }
        }
        Ok(Self {
            target: Target::Tag(tag),
            constraints: constraints.to_vec(),
        })
    }
    /// The exact block this predicate selects, if it is not a tag filter.
    pub fn block_key(&self) -> Option<BlockKey> {
        match self.target {
            Target::Block(key) => Some(key),
            Target::Tag(_) => None,
        }
    }
    /// The tag this predicate selects, if it is not an exact block filter.
    pub fn tag_key(&self) -> Option<TagKey> {
        match self.target {
            Target::Tag(key) => Some(key),
            Target::Block(_) => None,
        }
    }
    /// Explicit property requirements; all must match, but omitted properties are unrestricted.
    pub fn constraints(&self) -> &[StateConstraint] {
        &self.constraints
    }
}

#[cfg(test)]
#[path = "state_predicate/tests.rs"]
mod tests;
