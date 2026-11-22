//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `data_maps` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call `ItemKey::data`,
// `data_map_value` and `data_map_entries`
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait DataMapsBackend {
    /// Side: both.
    /// Entry `entry` of registry `registry`: its published NBT value in data map `map`, before
    /// typed decoding. The tag borrows from this backend, not the lookup IDs. Clone it only
    /// to retain it beyond the borrow. `Ok(None)` means the map gives this entry no value;
    /// unavailable maps and unknown map types are errors.
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<&Tag>, DataMapError>;
    /// Side: both.
    /// The published NBT entries of data map `map` in `registry`, sorted by entry ID
    /// (`IRegistryExtension.getDataMap`). The slice borrows from this backend; call `to_vec`
    /// only to retain all the rows beyond that borrow. An empty map gives an empty slice.
    fn data_map_entries(&self, registry: &str, map: &str)
    -> Result<&[(String, Tag)], DataMapError>;
}
