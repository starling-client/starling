use super::*;
use mod_api::{MovementInput, MovementKeys};
use sunrise_client::sprint::Keys;

impl Game<'_> {
    /// Local player input, after sampling and before auto-jump and sprint decisions.
    fn native_movement_input_update(&self, call: mod_api::__private::MovementInputUpdateCall<'_>) {
        self.fire_movement_input_update(call);
    }

<<<<<<< sync-sunrise-1
impl Game<'_> {
    /// Apply the listener's keys or independent axes before local movement and
    /// packet construction. A key edit wins over all axis edits, including a
    /// controller's old vector; opposing keys remain independently observable.
    pub(in crate::game) fn mod_movement_keys(
        &self,
        mut keys: sunrise_client::sprint::Keys,
    ) -> sunrise_client::sprint::Keys {
        // Starling: the old hook first, for the mods made for it (native_compat.rs).
        keys = self.old_movement_keys(keys);
        if !mod_api::__private::enabled_listened(mod_api::HookId::MovementInput) {
=======
    /// Lend both parts of the sampled input together, without deriving either from edits to the other.
    pub(in crate::game) fn update_movement_input(&self, keys: Keys) -> Keys {
        if !mod_api::__private::enabled_listened(mod_api::HookId::MovementInputUpdate) {
>>>>>>> sunrise/main
            return keys;
        }
        let mut input = MovementInput {
            move_vector: keys.direction(),
            key_presses: MovementKeys {
                forward: keys.forward,
                backward: keys.backward,
                left: keys.left,
                right: keys.right,
                jump: keys.jump,
                sneak: keys.shift,
                sprint: keys.sprint,
            },
        };
        self.native_movement_input_update(mod_api::__private::MovementInputUpdateCall {
            input: &mut input,
        });
        Keys {
            forward: input.key_presses.forward,
            backward: input.key_presses.backward,
            left: input.key_presses.left,
            right: input.key_presses.right,
            jump: input.key_presses.jump,
            shift: input.key_presses.sneak,
            sprint: input.key_presses.sprint,
            analog: Some(input.move_vector),
        }
    }

    #[cfg(test)]
    pub(in crate::game) fn movement_dispatch_fixture(&self, fire: bool) -> f32 {
        let mut input = MovementInput {
            move_vector: [0.0, 1.0],
            ..MovementInput::default()
        };
        let call = mod_api::__private::MovementInputUpdateCall { input: &mut input };
        if fire {
            self.native_movement_input_update(call);
        } else {
            mod_api::dispatch_movement_input_update(self, call);
        }
        input.move_vector[1]
    }
}
