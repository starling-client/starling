use super::*;

impl mod_api::PackDiscoveryContext for Game<'_> {
    fn live_host(&mut self) -> Option<mod_api::HostSide<'_>> {
        Some(super::super::execution_role::host_side(self))
    }

    fn client_text(&self) -> Option<&dyn mod_api::prelude::ClientTextClientHost> {
        if self.is_logical_server() {
            None
        } else {
            Some(self)
        }
    }
}

impl Game<'_> {
    /// Register sources when a resource or data pack repository is discovered.
    pub(in crate::game) fn native_add_pack_finders(
        &mut self,
        packs: &mut crate::game::pack_finders::Registry,
    ) {
        let mods = self.mods.clone();
        Self::fire_add_pack_finders(
            self,
            |listen| packs.register_listener(listen),
            |id| mods.code_owner_enabled(id),
        );
    }
}
