#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UseAnimation {
    #[default]
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Bundle,
    Spear,
}

/// `ItemUseAnimation.BY_ID`: names in network ID order.
pub const NAMES: [&str; 12] = [
    "none",
    "eat",
    "drink",
    "block",
    "bow",
    "trident",
    "crossbow",
    "spyglass",
    "toot_horn",
    "brush",
    "bundle",
    "spear",
];
