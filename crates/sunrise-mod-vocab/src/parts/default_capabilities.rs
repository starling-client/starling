//! Type-bound declarations consumed by the game's default capability providers.
use super::*;

/// One registered dynamic item-container callback with its owner and per-registration panic state.
#[derive(Clone)]
pub(crate) struct DefaultItemContainer {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    pub(crate) provider: Arc<dyn crate::ItemContainerProvider>,
    pub(crate) panicked: Arc<std::sync::atomic::AtomicBool>,
}

/// A registered item's dynamic slot count. The item type is bound at registration; this
/// callback runs only for an actual runtime lookup, never to infer whether it is registered.
pub trait ItemContainerProvider: Send + Sync {
    /// The item's slot count now, or none to let the next registration answer.
    fn slots(&self, host: &dyn Host) -> Option<u32>;
}

impl<F: Fn(&dyn Host) -> Option<u32> + Send + Sync> ItemContainerProvider for F {
    fn slots(&self, host: &dyn Host) -> Option<u32> {
        self(host)
    }
}

impl dyn Registrar + '_ {
    /// Register the dynamic default item-container provider for this item type.
    pub fn item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: impl ItemContainerProvider + 'static,
    ) {
        self.register_item_container(item, Arc::new(provider));
    }
}

/// Explicit domains of the game's default item, fluid and energy providers.
pub trait DefaultCapabilitiesRegistrar {
    /// Declare this mod block's item slots and optional menu. Vanilla blocks must use
    /// `block_capability` instead of replacing the game's block entity ownership.
    /// `container.slots` must fit in a nonnegative `i32` (matching NeoForge `ResourceHandler.size`
    /// and `getResource(int)`), every slot index in `faces`, `inputs` and `outputs` must be
    /// `< container.slots`, and `insert_rules.len()` must be `<= container.slots` (omitted rules
    /// mean `Any`); malformed declarations are refused at registration. A supported `menu` whose
    /// slot count matches `container.slots` opens when the block is used; an unsupported or
    /// mismatched menu is ignored without changing `container.slots`. A provider with no slots is
    /// registered.
    fn block_container(&mut self, block: &crate::BlockKey, container: crate::ModContainer);
    /// Declare this block's fluid tanks. Every tank index in `faces`, `inputs` and `outputs` must
    /// be `< fluids.tanks.len()`; out-of-range indices are refused at registration. Capacities are
    /// kept as declared (matching NeoForge `FluidStacksResourceHandler`); a zero-capacity tank
    /// accepts no fluid.
    fn block_fluids(&mut self, block: &crate::BlockKey, fluids: crate::ModFluidTanks);
    /// Declare this block's energy buffer (matching NeoForge `SimpleEnergyHandler`). `capacity`,
    /// `max_insert` and `max_extract` must be nonnegative (`>= 0`); negative values are refused at
    /// registration. Zero capacity and transfer rates larger than `capacity` are kept as declared.
    fn block_energy(&mut self, block: &crate::BlockKey, energy: crate::ModEnergyBuffer);
    /// Register the type-bound callback used by `item_container`.
    fn register_item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: Arc<dyn ItemContainerProvider>,
    );
}

/// Published default stock storage declarations read by the game's default capability providers.
// hidden: the game reads the installed default providers through it; mods declare them through the registrar.
#[doc(hidden)]
pub trait DefaultCapabilitiesCapabilityProviders {
    /// First enabled declaration of this block's item container.
    fn block_container(&self, block: &str) -> Option<crate::ModContainer>;
    /// First enabled declaration of this block's fluid tanks.
    fn block_fluids(&self, block: &str) -> Option<crate::ModFluidTanks>;
    /// First enabled declaration of this block's energy buffer.
    fn block_energy(&self, block: &str) -> Option<crate::ModEnergyBuffer>;
}

impl DefaultCapabilitiesCapabilityProviders for InstalledProviders {
    fn block_container(&self, block: &str) -> Option<crate::ModContainer> {
        declared_block_container(block).cloned()
    }
    fn block_fluids(&self, block: &str) -> Option<crate::ModFluidTanks> {
        first_enabled(installed()?.keys.default_block_fluids.get(block)?).cloned()
    }
    fn block_energy(&self, block: &str) -> Option<crate::ModEnergyBuffer> {
        first_enabled(installed()?.keys.default_block_energy.get(block)?).cloned()
    }
}

pub(super) fn declared_block_container(block: &str) -> Option<&'static crate::ModContainer> {
    first_enabled(installed()?.keys.default_block_containers.get(block)?)
}

fn first_enabled<T>(declarations: &[(usize, T)]) -> Option<&T> {
    declarations
        .iter()
        .find(|(index, _)| crate::mod_index_on(*index))
        .map(|(_, declaration)| declaration)
}
