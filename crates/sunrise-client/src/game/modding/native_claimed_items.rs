//! Main-hand input captured by an item behavior, and the client host's view raycast.
use super::{Game, Screen, api_values::TryIntoApi};
use mod_api::{
    __private::{client_item_use, mod_index_on},
    ClientHost, ClientItemUseBehavior, ItemStack,
    content::crash::{ModCall, enter_native_mod},
};
use std::sync::Arc;

/// The farthest a ray along the view reaches, in blocks.
pub(crate) const RAYCAST_MAX: f32 = 1024.0;

/// Client input captured by a main-hand item behavior, not a living entity's item use.
pub(super) struct ActiveClientItemUse {
    mod_id: &'static str,
    mod_index: usize,
    behavior: Arc<dyn ClientItemUseBehavior>,
    /// Last known live stack of `item_id` in `slot`, rebound on tick and release like
    /// `LivingEntity.useItem`.
    stack: ItemStack,
    item_id: Arc<str>,
    slot: usize,
    ticks: u32,
}

impl Game<'_> {
    /// The first block along the view within `distance` (at most 1024), in loaded chunks only; `None` on a miss.
    pub(super) fn raycast(&self, distance: f32) -> Option<mod_api::Hit> {
        let (d, hit) = self.view_hit()?;
        (d <= distance.clamp(0.0, RAYCAST_MAX)).then_some(hit)
    }
    /// The first block along the view within `RAYCAST_MAX`, in loaded chunks only, and its distance; `None` on a miss.
    pub(super) fn view_hit(&self) -> Option<(f32, mod_api::Hit)> {
        self.player.as_ref()?;
        let eye = self.eye_position()?;
        let dir = super::view_vector(self.yaw, self.pitch);
        let reach = f64::from(RAYCAST_MAX);
        let hit = self.clip_ray(eye, dir, reach)?;
        let (pos, point) = (hit.pos, hit.point);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let distance = (0..3)
            .map(|i| (point[i] - eye[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        Some((
            distance as f32,
            mod_api::Hit {
                block_x: pos[0],
                block_y: pos[1],
                block_z: pos[2],
                x: point[0],
                y: point[1],
                z: point[2],
                block: mod_api::__private::block_key(&self.res.facts().get(state).name)?,
            },
        ))
    }

    fn can_capture_item_use(&self) -> bool {
        self.using.is_none()
            && !self.spectator()
            && self.screen != Screen::Dead
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.health.health() > 0.0)
            && self
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.session.connected())
    }

    /// Can the current client still hold this capture? Do not read the stack's components.
    fn client_item_use_valid(&self, active: &ActiveClientItemUse) -> bool {
        active.slot == self.selected
            && self.can_capture_item_use()
            && self
                .stack_in(active.slot)
                .is_some_and(|stack| stack.item == active.item_id && stack.count != 0)
    }

    /// Read the live hotbar stack at `active.slot` when it still holds the same item kind.
    fn live_client_item_use_stack(&self, active: &ActiveClientItemUse) -> Option<ItemStack> {
        let stack = self.stack_in(active.slot)?;
        if stack.item != active.item_id || stack.count == 0 {
            return None;
        }
        stack.try_into_api().ok()
    }

    /// Give the registered main-hand behavior this click instead of using a target or item.
    pub(in crate::game) fn start_client_item_use(&mut self) -> bool {
        let slot = self.selected;
        let Some(stack) = self.stack_in(slot) else {
            return false;
        };
        let Some((mod_index, mod_id, behavior)) = client_item_use(&stack.item) else {
            return false;
        };
        if !mod_index_on(mod_index)
            || !self.can_capture_item_use()
            || &*stack.item == "minecraft:air"
        {
            return false;
        }
        let Ok(initial) = stack.try_into_api() else {
            return false;
        };
        let item_id = Arc::clone(&stack.item);
        self.client_item_use = Some(ActiveClientItemUse {
            mod_id,
            mod_index,
            behavior: Arc::clone(&behavior),
            stack: initial.clone(),
            item_id,
            slot,
            ticks: 0,
        });
        self.call_client_item_use(mod_id, "use_item", |host| {
            behavior.use_item(host, &initial, slot as u32);
        });
        true
    }

    /// Reset an invalid capture or rebind the live stack and count a held tick. Only input
    /// handling releases it.
    pub(in crate::game) fn tick_client_item_use(&mut self) {
        let Some(active) = self.client_item_use.as_ref() else {
            return;
        };
        let valid = mod_index_on(active.mod_index) && self.client_item_use_valid(active);
        let live = if valid {
            self.live_client_item_use_stack(active)
        } else {
            None
        };
        let held = self.mapped_key_down("key.use");
        if !valid {
            self.reset_client_item_use();
        } else if let Some(active) = self.client_item_use.as_mut() {
            if let Some(live) = live {
                active.stack = live;
            }
            if held {
                active.ticks = active.ticks.saturating_add(1);
            }
        }
    }

    pub(in crate::game) fn reset_client_item_use(&mut self) {
        self.end_client_item_use(false);
    }

    /// Retire the capture before entering mod code. Only a valid input release calls
    /// `release_using`; every other end is a reset. Return whether a capture owned the input.
    pub(in crate::game) fn end_client_item_use(&mut self, release_requested: bool) -> bool {
        let Some(mut active) = self.client_item_use.take() else {
            return false;
        };
        if let Some(live) = self.live_client_item_use_stack(&active) {
            active.stack = live;
        }
        if release_requested
            && mod_index_on(active.mod_index)
            && self.client_item_use_valid(&active)
        {
            self.call_client_item_use(active.mod_id, "release_using", |host| {
                active.behavior.release_using(
                    host,
                    &active.stack,
                    active.slot as u32,
                    active.ticks,
                );
            });
        }
        if mod_index_on(active.mod_index) {
            self.call_client_item_use(active.mod_id, "on_stop_using", |host| {
                active.behavior.on_stop_using(
                    host,
                    &active.stack,
                    active.slot as u32,
                    active.ticks,
                );
            });
        }
        true
    }

    fn call_client_item_use(
        &mut self,
        mod_id: &'static str,
        phase: &'static str,
        invoke: impl FnOnce(&mut dyn ClientHost),
    ) {
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: phase,
            part: Some("ClientItemUseBehavior"),
        });
        invoke(self);
    }
}
