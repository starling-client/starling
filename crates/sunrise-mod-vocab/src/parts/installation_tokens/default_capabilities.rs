//! Engine access to the same type-bound default providers registration queries observe.
/// This block's first enabled item-container declaration.
pub fn block_container(block: &str) -> Option<&'static crate::ModContainer> {
    crate::parts::default_capabilities::declared_block_container(block)
}
/// This block's first enabled fluid-tank declaration.
pub fn block_fluids(block: &str) -> Option<crate::ModFluidTanks> {
    crate::parts::DefaultCapabilitiesCapabilityProviders::block_fluids(
        &crate::InstalledProviders,
        block,
    )
}
/// This block's first enabled energy-buffer declaration.
pub fn block_energy(block: &str) -> Option<crate::ModEnergyBuffer> {
    crate::parts::DefaultCapabilitiesCapabilityProviders::block_energy(
        &crate::InstalledProviders,
        block,
    )
}
/// Run only this item's registered dynamic callbacks at the normal runtime lookup moment.
pub fn item_container_slots(host: &dyn crate::Host, item: &str) -> Option<u32> {
    let providers = crate::parts::installed()?
        .keys
        .default_item_containers
        .get(item)?;
    for part in providers {
        if !crate::callback_owner::enabled(part.mod_index, &part.panicked) {
            continue;
        }
        if let Ok(Some(slots)) = crate::callback_owner::call(
            part.mod_id,
            part.mod_index,
            &part.panicked,
            "item container provider",
            "item_container_slots",
            || part.provider.slots(host),
        ) {
            return Some(slots);
        }
    }
    None
}
