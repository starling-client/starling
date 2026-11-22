//! The client host builds hashed container clicks with its connection registries.
use super::Game;
use mod_api::{ContainerClickRequest, PacketBuildError, ServerboundPlayPacket};

impl Game<'_> {
    pub(super) fn container_click(
        &self,
        request: &ContainerClickRequest,
    ) -> Result<ServerboundPlayPacket, PacketBuildError> {
        ServerboundPlayPacket::validate_container_click_limits(
            request.container_id,
            request.state_id,
            request.slot,
            request.changed_slots.len() as u64,
        )?;
        let remote = self
            .remote
            .as_ref()
            .ok_or_else(|| PacketBuildError::Unavailable {
                packet: "container_click".into(),
                field: "connection".into(),
            })?;
        sunrise_client::mod_packet_builders::encode_container_click(
            request,
            &remote.builtin,
            &remote.registries,
            |stack| super::api_values::stack_value(stack).ok(),
        )
    }
}

#[cfg(test)]
#[path = "native_packet_builders/tests.rs"]
mod tests;
