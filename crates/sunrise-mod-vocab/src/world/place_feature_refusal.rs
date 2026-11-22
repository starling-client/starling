//! Refusals when a mod places a feature (`LevelRef::place_feature` or `LevelRef::place_placed_feature`).

/// Why [`LevelRef::place_feature`](crate::LevelRef::place_feature) or
/// [`LevelRef::place_placed_feature`](crate::LevelRef::place_placed_feature) refused placement.
/// Except for `BiomeFilter`, refusals are checked before placement and change nothing.
/// A feature that ran and reported failure is `Ok(false)`, not a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlaceFeatureRefusal {
    /// The level handle is of an earlier open of the world, or the level is gone.
    Stale,
    /// The call was made on a client; features are placed on the server.
    Client,
    /// The level is not the one in place, the one being ticked; only that one is written.
    NotInPlace,
    /// A chunk of the 3x3 around `pos` is not loaded (`PlaceCommand.checkLoaded`), or `pos` is
    /// outside the level's height, or a write there is held by another operation.
    NotLoaded,
    /// The id has no namespace or is absent from the called method's registry:
    /// `minecraft:worldgen/feature` for `place_feature`,
    /// `minecraft:worldgen/placed_feature` for `place_placed_feature`.
    UnknownFeature,
    /// Direct placement reached a biome filter without a top feature. This call's
    /// feature has not run: all its positions must first pass that filter. A nested
    /// placed feature can still have written before an inner biome filter refuses.
    BiomeFilter,
}

impl std::fmt::Display for PlaceFeatureRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale => "the level handle names no level of this world",
            Self::Client => "features are placed on the server only",
            Self::NotInPlace => "the level is not the one being ticked",
            Self::NotLoaded => "the chunks around the position are not loaded",
            Self::BiomeFilter => "direct placement reached a biome filter without a top feature",
            Self::UnknownFeature => "no feature has that id",
        })
    }
}

impl std::error::Error for PlaceFeatureRefusal {}

impl crate::NoBackend for PlaceFeatureRefusal {
    fn no_backend() -> Self {
        Self::Client
    }
}
