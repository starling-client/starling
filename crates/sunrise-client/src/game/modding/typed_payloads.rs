//! The host's typed payload token check: what every typed send and reply runs first.
use super::Game;
use mod_api::{PayloadSendError, PayloadToken};

impl Game<'_> {
    /// Check `token` against this load's frozen typed declarations.
    pub(super) fn check_payload_token(&self, token: &PayloadToken) -> Result<(), PayloadSendError> {
        mod_api::__private::game::check_payload_token(token)
    }
}

impl Game<'_> {
    /// The drop counts of `Droppable` channel `id`.
    pub(super) fn payload_drop_counts(
        &self,
        id: &mod_api::PayloadId,
    ) -> Result<Option<mod_api::PayloadDropCounts>, mod_api::LevelReadError> {
        Ok(mod_api::__private::game::payload_drop_counts(id))
    }
}
