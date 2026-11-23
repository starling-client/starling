use super::*;

/// The actual read host of a target event, either the game or a scoped mob tick.
pub(in crate::game) struct NativeHookHost<'a> {
    pub(in crate::game) host: &'a dyn mod_api::Host,
}

impl NativeHookHost<'_> {
    /// A mob is about to acquire or clear a target.
    pub(in crate::game) fn native_living_change_target(
        &self,
        call: mod_api::__private::LivingChangeTargetCall<'_>,
    ) {
        self.fire_living_change_target(call);
    }
}
