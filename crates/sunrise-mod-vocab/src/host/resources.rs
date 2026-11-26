//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The host's methods both sides share of the `resources` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ResourcesHost {
    /// Side: both.
    /// Open the original file from the winning pack, as `ResourceManager.getResource`
    /// and `Resource.open` do, not the bytes merged for a renderer or reload listener.
    /// In priority order, each pack can supply its own file or block lower files
    /// with `filter.block`. Client resources come from the selected asset packs
    /// over the game's assets; server data comes from the selected world data packs
    /// over the game's and mods' data, with `minecraft:vanilla` kept unless disabled.
    /// `identifier` is the resource ID (e.g. `minecraft:models/block/stone.json`
    /// or `mymod:machines/furnace.json`), without the pack's directory prefix.
    ///
    /// `Ok(None)` means the resource or stack is absent, including a filtered-out
    /// resource. Lookup or open errors are returned as [`ResourceOpenError`]; I/O
    /// and archive decoding can also fail while reading. A failure never substitutes
    /// a lower pack's bytes. The reader borrows this context's published generation.
    /// It does not reload packs, impose a resource size limit, or read the whole
    /// file on open. Use `std::io::Read::read_to_end` to explicitly keep owned bytes.
    fn open_resource(
        &self,
        pack_type: PackType,
        identifier: &Identifier,
    ) -> Result<Option<Box<dyn std::io::Read + '_>>, ResourceOpenError>;

    /// Side: both.
    /// Every selected pack's copy of resource `identifier` in `pack_type`'s stack, lowest first
    /// and the highest pack's last, as `ResourceManager.getResourceStack` answers it; empty where
    /// no pack supplies it. The stack and the names are those of the selected packs;
    /// [`open_resource`](Self::open_resource) opens the highest pack's own file. A copy that
    /// can't be read is left out, so the last entry can be a lower pack's copy than the one
    /// `open_resource` opens. This read never reloads packs.
    fn resource_stack(&self, pack_type: PackType, identifier: &Identifier) -> Vec<Vec<u8>>;

    /// Side: both.
    /// The resources of `pack_type`'s stack under folder `directory` of every namespace whose
    /// path ends in `suffix`, one each, in identifier order, as `ResourceManager.listResources`
    /// answers it: `list_resources(PackType::ClientResources, "textures/item", ".png")` gives
    /// `minecraft:textures/item/apple.png` and the rest. An empty `directory` lists a whole
    /// namespace. Metadata files (`.mcmeta`) are never listed, and a name that isn't a valid
    /// identifier is skipped. A side without that stack, or a `directory` that leaves its
    /// namespace, lists nothing. This read never reloads packs.
    fn list_resources(&self, pack_type: PackType, directory: &str, suffix: &str)
    -> Vec<Identifier>;
}
