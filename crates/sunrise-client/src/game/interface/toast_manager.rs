//! The client's ordinary toast queue. Now playing is independent of the slots.

use super::{Game, advancements, mod_toasts, tutorial};
use std::{cell::RefCell, collections::VecDeque};
use sunrise_client::{
    hud::HudBatch,
    toasts::{SystemToast, ToastClock},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SystemToastId {
    Narrator,
    PackLoadFailure,
    PackDownload(usize),
    ChunkIo(usize),
    Title,
}

pub(super) enum Toast {
    Advancement(advancements::Toast),
    Tutorial(tutorial::HintToast),
    System(Option<SystemToastId>, SystemToast),
    Diagnostic {
        toast: SystemToast,
        hide_with_f1: bool,
    },
    Mod(mod_toasts::Entry),
}

pub(super) struct VisibleToast {
    pub(super) toast: Toast,
    pub(super) slot: usize,
    // A reset can resize the content, but the instance keeps its original slots.
    slots: u32,
}

#[derive(Default)]
pub(super) struct ToastManager {
    pub(super) pending_mod: RefCell<Vec<mod_toasts::Pending>>,
    pub(super) queued: VecDeque<Toast>,
    pub(super) visible: Vec<VisibleToast>,
    /// The mods holding new toasts back (`set_toasts_held`), as Dynamic FPS makes
    /// `ToastManager.freeSlotCount` 0 while the window is in the background.
    holders: RefCell<Vec<&'static str>>,
}

impl ToastManager {
    /// Mod `id` holds new toasts back, or lets them go. False, changing nothing, with no mod.
    pub(super) fn hold(&self, id: Option<&'static str>, held: bool) -> bool {
        let Some(id) = id else {
            return false;
        };
        let mut holders = self.holders.borrow_mut();
        holders.retain(|holder| *holder != id);
        if held {
            holders.push(id);
        }
        true
    }

    /// Whether an enabled mod holds new toasts back.
    fn held(&self) -> bool {
        self.holders.borrow().iter().any(|id| {
            mod_api::__private::mod_index(id).is_some_and(mod_api::__private::mod_index_on)
        })
    }

    pub(super) fn all_mut(&mut self) -> impl Iterator<Item = &mut Toast> {
        self.visible
            .iter_mut()
            .map(|entry| &mut entry.toast)
            .chain(&mut self.queued)
    }

    /// `ToastManager.getToast` looks in the visible list before the queue.
    pub(super) fn find_mut<T>(
        &mut self,
        get: impl FnMut(&mut Toast) -> Option<&mut T>,
    ) -> Option<&mut T> {
        self.all_mut().find_map(get)
    }

    pub(super) fn system_mut(&mut self, id: SystemToastId) -> Option<&mut SystemToast> {
        self.find_mut(|entry| match entry {
            Toast::System(Some(key), toast) if *key == id => Some(toast),
            _ => None,
        })
    }

    /// `ToastManager.update`: finish/update the visible list before offering
    /// vacancies to the queue, in insertion order. Admission cannot sound a
    /// visibility change until the next update of the promoted instance.
    pub(super) fn update(&mut self, clock: ToastClock, multiplier: f64) -> (Option<bool>, bool) {
        let mut visibility_sound = None;
        self.visible.retain_mut(|entry| {
            let (sound, finished) = match &mut entry.toast {
                Toast::Advancement(t) => t.update(clock, multiplier),
                Toast::Tutorial(t) => t.update(clock),
                Toast::System(_, t) | Toast::Diagnostic { toast: t, .. } => {
                    t.update(clock, multiplier)
                }
                Toast::Mod(t) => t.request.toast.update(clock, multiplier),
            };
            visibility_sound = visibility_sound.or(sound);
            !finished
        });

        // ToastManager.SLOT_COUNT and findFreeSlotsIndex: first consecutive run that fits.
        const SLOT_COUNT: usize = 5;
        let mut used = self
            .visible
            .iter()
            .fold(0u32, |used, entry| used | entry.slots);
        let mut challenge_sound = false;
        // A held queue admits nothing: its toasts wait, as they do for a free slot.
        if !self.held() && used.count_ones() < SLOT_COUNT as u32 {
            for _ in 0..self.queued.len() {
                let Some(toast) = self.queued.pop_front() else {
                    break;
                };
                let count = match &toast {
                    Toast::Tutorial(t) => t.slots(),
                    Toast::Mod(t) => t.layout.slots(),
                    _ => 1,
                };
                let slot = SLOT_COUNT.checked_sub(count).and_then(|last| {
                    let mask = (1 << count) - 1;
                    (0..=last).find(|slot| used & (mask << slot) == 0)
                });
                let Some(slot) = slot else {
                    self.queued.push_back(toast);
                    continue;
                };
                let slots = ((1 << count) - 1) << slot;
                used |= slots;
                challenge_sound |= matches!(&toast, Toast::Advancement(t) if t.is_challenge());
                self.visible.push(VisibleToast { toast, slot, slots });
            }
        }
        (visibility_sound, challenge_sound)
    }
}

impl ToastManager {
    /// `ToastManager.addToast`: the toast waits in the queue for a free slot. Under `SUNRISE_STILL`
    /// (`ToastClock::Still`) it is done at once instead: it never takes a slot or waits in the queue,
    /// which draws the same nothing a toast held at its first frame drew, and a later dismissal or
    /// replacement finds nothing to change.
    pub(super) fn post(&mut self, clock: ToastClock, toast: Toast) {
        if clock == ToastClock::Real {
            self.queued.push_back(toast);
        }
    }
}

impl Game<'_> {
    pub(super) fn add_system_toast(
        &mut self,
        id: Option<SystemToastId>,
        mut toast: SystemToast,
    ) -> bool {
        if !self.admit_toast(
            mod_api::ToastKind::System,
            &mut toast.title,
            &mut toast.message,
        ) {
            return false;
        }
        let clock = ToastClock::of(self.clock.still());
        self.toast_manager.post(clock, Toast::System(id, toast));
        true
    }

    pub(super) fn add_or_update_system_toast(
        &mut self,
        id: SystemToastId,
        new: SystemToast,
    ) -> bool {
        if let Some(toast) = self.toast_manager.system_mut(id) {
            toast.reset_from(new);
            true
        } else {
            self.add_system_toast(Some(id), new)
        }
    }

    /// `ToastManager.extractRenderState`: F1 hides the draw, not the update. The toasts' clock is
    /// read once here, for every toast this draw (`ToastClock::of`).
    pub(super) fn draw_every_toast(&mut self, batch: &mut HudBatch) {
        let clock = ToastClock::of(self.clock.still());
        let replay = self.replay.playing();
        let hidden = self.ui.player_hides_gui || replay;
        let diagnostic_hidden = replay || self.overlay_holds_view();
        self.admit_mod_toasts();
        self.add_server_report_toasts();
        self.add_hook_panic_toasts();
        self.update_tutorial_toasts();
        let multiplier = self.settings.notification_display_time;
        let (visibility_sound, challenge_sound) = self.toast_manager.update(clock, multiplier);
        if let Some(show) = visibility_sound {
            let event = if show {
                "minecraft:ui.toast.in"
            } else {
                "minecraft:ui.toast.out"
            };
            self.play_audio(event, None, 1.0, 1.0);
        }
        if challenge_sound {
            self.play_audio("minecraft:ui.toast.challenge_complete", None, 1.0, 1.0);
        }
        let atlas = &self.gui.atlas;
        for entry in &self.toast_manager.visible {
            let y = entry.slot as f32 * 32.0;
            match &entry.toast {
                Toast::Diagnostic {
                    toast,
                    hide_with_f1,
                } if !diagnostic_hidden && !(*hide_with_f1 && self.ui.player_hides_gui) => {
                    toast.draw(atlas, batch, y)
                }
                Toast::Advancement(t) if !hidden => {
                    self.draw_advancement_toast(batch, t, y, multiplier)
                }
                Toast::Tutorial(t) if !hidden => t.draw(atlas, batch, y),
                Toast::System(_, t) if !hidden => t.draw(atlas, batch, y),
                Toast::Mod(t) if !hidden => self.draw_mod_toast(batch, t, entry.slot),
                _ => {}
            }
        }
        self.draw_controller_notice(batch, clock);
        self.draw_music_toast(batch, hidden, clock);
    }
}

#[cfg(test)]
mod still_tests {
    use crate::game::{in_own_process, native_entity_refs::tests::with_client_game};
    use sunrise_client::{clock::FixedStep, hud::HudBatch, text::Text, toasts::SystemToast};

    /// One draw of every toast: its vertices' bytes.
    fn frame(game: &mut crate::game::Game<'_>) -> Vec<u8> {
        let mut batch = HudBatch::scratch([640, 480]);
        game.draw_every_toast(&mut batch);
        bytemuck::cast_slice(&batch.vertices).to_vec()
    }

    /// Under `SUNRISE_STILL` a posted toast is done at once: 1000 of them (system and mod toasts),
    /// through a level change, `SetTime` and the title screen, leave the slots and the queue empty
    /// and draw the frame a game with none draws. Normal play queues and shows them as before.
    #[test]
    #[ignore = "daily"]
    fn a_still_game_posts_toasts_into_nothing() {
        in_own_process(
            "game::toast_manager::still_tests::a_still_game_posts_toasts_into_nothing",
            || {
                with_client_game(|game| {
                    game.clock = FixedStep::still_for_tests();
                    let empty = frame(game);
                    for i in 0..1000 {
                        let title = Text::literal(format!("Saved {i}"));
                        if i % 2 == 0 {
                            assert!(game.add_system_toast(None, SystemToast::new(title, None)));
                        } else {
                            game.add_toast(mod_api::ModToast::new(mod_api::Text::literal(
                                format!("Mod {i}"),
                            )))
                            .unwrap();
                        }
                        match i {
                            300 => {
                                game.world =
                                    crate::game::Game::world_for_mods(game.res.facts().air, 9)
                            }
                            600 => {
                                game.world.game_time = 90_000;
                                game.world.day_time = 90_000;
                            }
                            900 => game.title = Some(game.new_title(game.save.root.clone())),
                            _ => {}
                        }
                        if i % 100 == 99 {
                            assert_eq!(frame(game), empty, "after {} toasts", i + 1);
                        }
                    }
                    assert_eq!(frame(game), empty);
                    assert!(game.toast_manager.visible.is_empty());
                    assert!(game.toast_manager.queued.is_empty());

                    // Normal play: five slots fill, the sixth waits.
                    game.clock = FixedStep::default();
                    assert!(!game.clock.still());
                    for i in 0..6 {
                        let title = Text::literal(format!("Real {i}"));
                        assert!(game.add_system_toast(None, SystemToast::new(title, None)));
                    }
                    frame(game);
                    assert_eq!(game.toast_manager.visible.len(), 5);
                    assert_eq!(game.toast_manager.queued.len(), 1);
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(manager: &mut ToastManager, title: &str) {
        manager
            .queued
            .push_back(Toast::System(None, SystemToast::new(title.into(), None)));
    }

    /// While an enabled mod holds them, new toasts wait in the queue, those already shown go on,
    /// and they show once the last holder lets go; a call with no mod holds nothing.
    #[test]
    fn held_toasts_wait_in_the_queue_until_released() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        let mods = vec![
            mod_api::ModEntry::new("first", Box::new(Quiet)),
            mod_api::ModEntry::new("second", Box::new(Quiet)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            let mut manager = ToastManager::default();
            notice(&mut manager, "shown");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!(manager.visible.len(), 1);

            assert!(!manager.hold(None, true));
            assert!(manager.hold(Some("first"), true));
            assert!(manager.hold(Some("second"), true));
            notice(&mut manager, "waits");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (1, 1));

            assert!(manager.hold(Some("first"), false));
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (1, 1));

            assert!(manager.hold(Some("second"), false));
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (2, 0));

            // An unknown (or disabled) mod's hold holds nothing.
            assert!(manager.hold(Some("gone"), true));
            notice(&mut manager, "unheld");
            manager.update(ToastClock::Real, 1.0);
            assert_eq!((manager.visible.len(), manager.queued.len()), (3, 0));
        });
    }
}
