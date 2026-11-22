//! `/give`'s handing over of one stack.
use super::{Game, Target};
use crate::game::level_sounds::LevelSound;
use mod_api::SoundSource;
use sunrise_client::items::ItemStack;
use sunrise_core::random::RandomSource;

enum GiveItem {
    Overflow(Option<[i32; 4]>),
    Fake,
}

impl Game<'_> {
    /// `GiveCommand.giveItem`'s handing over of one stack to the acting player:
    /// `Inventory.add`, then what did not fit thrown from the eyes with no pickup delay and
    /// `owner` (the player's UUID) its only taker, or the pickup sound when it all went in.
    /// Effects go to the player's level, even if the turn borrows them into another one.
    /// If their level is unavailable, return what did not fit rather than drop it elsewhere.
    pub(in crate::game) fn give_acting_stack(
        &mut self,
        mut stack: ItemStack,
        owner: Option<[i32; 4]>,
    ) -> Option<ItemStack> {
        let uuid = self.native_acting_id();
        self.journal_player(uuid);
        let left = self.insert_stack_into_inventory(&mut stack);
        // A borrowed player's body is in place but their world may be parked. The shared
        // give effects put that world in place only while replaying the sound or drop.
        let dim = self.target_level(Target::Player);
        if dim != self.dim && self.levels.parked_world(dim).is_none() {
            return left.then_some(stack);
        }
        if left {
            let eye = self.give_eye();
            self.after_commit(crate::game::transactions::After::GiveRest {
                uuid,
                rest: stack,
                owner,
                dim,
                eye,
            });
        } else {
            self.after_commit(crate::game::transactions::After::GivePickup {
                uuid,
                dim,
                prototype: ItemStack { count: 1, ..stack },
            });
        }
        None
    }
    /// `GiveCommand.giveItem` adds the fake pickup before drawing the sound pitch.
    pub(in crate::game) fn give_pickup(&mut self, prototype: ItemStack) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        let velocity = self.player_drop_motion(false);
        self.join_give_item(prototype, eye, velocity, GiveItem::Fake);
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let at = [player.position.x, player.position.y, player.position.z];
        let r = &mut player.random;
        let pitch = ((r.next_float() - r.next_float()) * 0.7 + 1.0) * 2.0;
        self.send_level_sound(
            LevelSound::new("minecraft:entity.item.pickup", at, 0.2, pitch)
                .in_source(SoundSource::Players),
            None,
        );
    }
    /// The receiving player's eyes, where what did not fit is thrown from.
    fn give_eye(&self) -> [f64; 3] {
        let p = self
            .target_position(Target::Player)
            .unwrap_or(self.source_position());
        self.eye_position()
            .unwrap_or([p[0], p[1] + self.target_eye_height(Target::Player), p[2]])
    }
    pub(in crate::game) fn give_remainder(&mut self, rest: ItemStack, owner: Option<[i32; 4]>) {
        let eye = self.give_eye();
        let velocity = self.player_drop_motion(false);
        self.join_give_item(rest, eye, velocity, GiveItem::Overflow(owner));
    }
    /// What did not fit of a `/give` whose player died or left between taking the stack and
    /// the root commit that throws it: it lies where it would have been thrown from, at
    /// `eye` as it was when the stack was taken, still with no pickup delay and `owner` its
    /// only taker. The stack was already taken from its source, so it lands rather than
    /// being lost.
    pub(in crate::game) fn give_remainder_unheld(
        &mut self,
        rest: ItemStack,
        owner: Option<[i32; 4]>,
        eye: [f64; 3],
    ) {
        self.join_give_item(rest, eye, [0.0; 3], GiveItem::Overflow(owner));
    }

    /// `GiveCommand.giveItem` configures the drop before `addFreshEntity`, without `Player.drop`.
    fn join_give_item(
        &mut self,
        stack: ItemStack,
        eye: [f64; 3],
        velocity: [f64; 3],
        kind: GiveItem,
    ) {
        let at = [eye[0], eye[1] - f64::from(0.3f32), eye[2]];
        let mut random = super::super::spill::prepare_item_random(None);
        let delay = match kind {
            GiveItem::Overflow(owner) => {
                random.state.target = owner.map(sunrise_client::lan::playerdata::ints_uuid);
                0
            }
            // ItemEntity.makeFakeItem: never pick up, one tick left before expiry.
            GiveItem::Fake => {
                random.state.age = 5999;
                32767
            }
        };
        self.push_item_entity(stack, at, Some(velocity), delay, Some(random));
        self.join_item_entity(self.next_entity, at);
    }
}
