//! Client tutorial progression (`net.minecraft.client.tutorial`).

use sunrise_client_gameplay::tutorial::social_hint_progress;
pub(super) use sunrise_client_gameplay::tutorial::{Hint, Previous, Step, Tutorial};

impl super::Game<'_> {
    /// `Tutorial.setStep` persists immediately so it survives leaving a world.
    pub(super) fn persist_tutorial_step(&mut self) {
        self.settings.file.set(
            "tutorialStep",
            sunrise_client::options_file::V::Int(self.tutorial.step as i32),
        );
        self.settings.save(&Self::options_path());
    }
}

impl super::Game<'_> {
    pub(super) fn reset_tutorial(&mut self) {
        self.tutorial = Tutorial::new(Step::from_option(self.settings.file.int("tutorialStep")));
        self.tutorial_toasts.clear();
    }

    fn tutorial_tagged(&self, item: bool, tag: &str, id: &str) -> bool {
        let items = self.res.items();
        let tags = if item {
            &items.item_tags
        } else {
            &items.block_tags
        };
        tags.get(tag).is_some_and(|members| members.contains(id))
    }

    pub(super) fn tutorial_get_item(&mut self, stack: Option<&sunrise_client::items::ItemStack>) {
        // The server thread's settings are a stale copy of the window's: a
        // guest's tutorial is their own client's to record.
        if self.options.dedicated.is_some()
            || self.serving.is_some()
            || self.lan_acting.is_some()
            || self.title.is_some()
            || self.player.is_none()
        {
            return;
        }
        let Some(stack) = stack else { return };
        let id = &*stack.item;
        let tree = self.tutorial_tagged(true, "minecraft:completes_find_tree_tutorial", id);
        let log = self.tutorial_tagged(true, "minecraft:logs", id);
        let plank = self.tutorial_tagged(true, "minecraft:planks", id);
        if self.tutorial.get_item(tree, log, plank) {
            self.persist_tutorial_step();
        }
    }

    pub(super) fn tutorial_destroy(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        progress: f32,
    ) {
        // The server thread's settings are a stale copy of the window's: a
        // guest's tutorial is their own client's to record.
        if self.options.dedicated.is_some()
            || self.serving.is_some()
            || self.lan_acting.is_some()
            || self.title.is_some()
            || self.player.is_none()
        {
            return;
        }
        let name = &self.res.facts().get(state).name;
        let is_log = self.tutorial_tagged(false, "minecraft:logs", name);
        if self.tutorial.destroy(is_log, progress) {
            self.persist_tutorial_step();
        }
    }

    pub(super) fn tick_tutorial(&mut self) {
        use sunrise_client::actions::Action;
        if self.title.is_some()
            || self.player.is_none()
            || self.options.dedicated.is_some()
            || self.serving.is_some()
        {
            return;
        }
        // Minecraft.tick: also show this in a single-player world once it is
        // published to LAN, but not merely because its client uses memory networking.
        if !self.settings.file.bool("joinedFirstServer")
            && (!self.has_singleplayer_server() || self.is_published())
        {
            if let Some(toast) = self.tutorial_toast(Hint::SocialInteractions, Some(0.0)) {
                self.tutorial_toasts.push(toast);
            }
            self.settings.file.set(
                "joinedFirstServer",
                sunrise_client::options_file::V::Bool(true),
            );
            self.settings.save(&Self::options_path());
        }
        if self.screen == super::Screen::Playing
            && ([
                Action::Forward,
                Action::Left,
                Action::Back,
                Action::Right,
                Action::Jump,
            ]
            .into_iter()
            .any(|action| self.held_mapping(action))
                || self
                    .input_move()
                    .is_some_and(|[left, forward]| left.abs() > 0.3 || forward.abs() > 0.3))
        {
            self.tutorial.input();
        }
        let items = self.res.items();
        let in_tag =
            |tag: &str, id: &str| items.item_tags.get(tag).is_some_and(|set| set.contains(id));
        let mut previous = Previous::default();
        if self.tutorial.waiting == 0 && !matches!(self.tutorial.step, Step::Movement | Step::None)
        {
            for stack in self
                .inventory
                .iter()
                .chain(self.armor.iter())
                .flatten()
                .chain(self.offhand.iter())
            {
                previous.tree_item |= in_tag("minecraft:completes_find_tree_tutorial", &stack.item);
                previous.log |= in_tag("minecraft:logs", &stack.item);
                previous.plank |= in_tag("minecraft:planks", &stack.item);
            }
            if let Some(mined) = self.stats.values.get("minecraft:mined")
                && let Some(tree) = items
                    .block_tags
                    .get("minecraft:completes_find_tree_tutorial")
            {
                previous.mined_tree = mined
                    .iter()
                    .any(|(id, count)| *count > 0 && tree.contains(id));
            }
            if let Some(crafted) = self.stats.values.get("minecraft:crafted")
                && let Some(planks) = items.item_tags.get("minecraft:planks")
            {
                previous.crafted_plank = crafted
                    .iter()
                    .any(|(id, count)| *count > 0 && planks.contains(id));
            }
        }
        let survival = self.mode == sunrise_client::command::args::GameMode::Survival;
        if self.tutorial.tick(survival, &previous) {
            self.persist_tutorial_step();
        }
        if self.tutorial.step == Step::FindTree
            && self.pick_entity_hit().is_none()
            && let Some(hit) = self.pick()
        {
            let name = &self
                .res
                .facts()
                .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                .name;
            if self.tutorial_tagged(false, "minecraft:completes_find_tree_tutorial", name)
                && self.tutorial.look_at_tree()
            {
                self.persist_tutorial_step();
            }
        }
    }
}

pub(super) struct HintToast {
    hint: Hint,
    lines: Vec<(sunrise_client::text::Text, bool)>,
    title_lines: usize,
    progress: Option<f32>,
    smooth: f32,
    last_smooth: i64,
    wanted: bool,
    slot: Option<usize>,
    slots: usize,
    slide: sunrise_client::toasts::Slide,
}

impl super::Game<'_> {
    pub(super) fn hide_social_tutorial(&mut self) {
        for toast in &mut self.tutorial_toasts {
            if toast.hint == Hint::SocialInteractions {
                toast.wanted = false;
            }
        }
    }

    pub(super) fn tutorial_toast_slots(&self) -> u32 {
        self.tutorial_toasts
            .iter()
            .filter_map(|t| t.slot.map(|s| ((1u32 << t.slots) - 1) << s))
            .fold(0, |a, b| a | b)
    }

    fn tutorial_toast(&self, hint: Hint, progress: Option<f32>) -> Option<HintToast> {
        use sunrise_client::{keybinds, text::Text};
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping)).plain();
        let (title_key, fallback, description_key, description, icon_args) = match hint {
            Hint::Move => (
                "tutorial.move.title",
                "Move with %1$s, %2$s, %3$s, %4$s",
                "tutorial.move.description",
                "Jump with %1$s",
                vec![
                    key("key.forward"),
                    key("key.left"),
                    key("key.back"),
                    key("key.right"),
                ],
            ),
            Hint::Look => (
                "tutorial.look.title",
                "Look around",
                "tutorial.look.description",
                "Use your mouse to turn",
                vec![],
            ),
            Hint::FindTree => (
                "tutorial.find_tree.title",
                "Find a tree",
                "tutorial.find_tree.description",
                "Punch it to collect wood",
                vec![],
            ),
            Hint::PunchTree => (
                "tutorial.punch_tree.title",
                "Punch the tree",
                "tutorial.punch_tree.description",
                "Hold %1$s to break",
                vec![],
            ),
            Hint::OpenInventory => (
                "tutorial.open_inventory.title",
                "Open inventory",
                "tutorial.open_inventory.description",
                "Press %1$s",
                vec![],
            ),
            Hint::SocialInteractions => (
                "tutorial.socialInteractions.title",
                "Social Interactions",
                "tutorial.socialInteractions.description",
                "Press %1$s to open social interactions",
                vec![],
            ),
            Hint::CraftPlanks => (
                "tutorial.craft_planks.title",
                "Craft wooden planks",
                "tutorial.craft_planks.description",
                "The recipe book can help",
                vec![],
            ),
        };
        let substitute = |mut text: String, values: &[String]| {
            for (i, value) in values.iter().enumerate() {
                text = text.replace(&format!("%{}$s", i + 1), value);
                text = text.replacen("%s", value, 1);
            }
            text
        };
        let mut title = Text::literal(substitute(
            self.translate_with_fallback(title_key, fallback),
            &icon_args,
        ));
        let desc_args = match hint {
            Hint::Move => vec![key("key.jump")],
            Hint::PunchTree => vec![key("key.attack")],
            Hint::OpenInventory => vec![key("key.inventory")],
            Hint::SocialInteractions => vec![key("key.socialInteractions")],
            _ => vec![],
        };
        let mut description = Some(Text::literal(substitute(
            self.translate_with_fallback(description_key, description),
            &desc_args,
        )));
        if !self.add_toast(mod_api::ToastKind::Tutorial, &mut title, &mut description) {
            return None;
        }
        let font = &self.gui.atlas.font;
        let mut lines = font.split_lines(&title, 126.0);
        let title_lines = lines.len();
        if let Some(description) = description {
            lines.extend(font.split_lines(&description, 126.0));
        }
        let height = 10 + 11 * lines.len().max(2);
        let slots = height.div_ceil(32);
        Some(HintToast {
            hint,
            lines,
            title_lines,
            progress,
            smooth: 0.0,
            last_smooth: 0,
            wanted: true,
            slot: None,
            slots,
            slide: Default::default(),
        })
    }

    pub(super) fn draw_tutorial_toasts(
        &mut self,
        batch: &mut sunrise_client::hud::HudBatch,
        hidden: bool,
    ) {
        let wanted = self.tutorial.hint();
        if wanted.map(|(hint, _)| hint) != self.tutorial.cancelled_hint() {
            self.tutorial.set_cancelled_hint(None);
        }
        for t in &mut self.tutorial_toasts {
            if t.hint == Hint::SocialInteractions {
                continue;
            }
            t.wanted = wanted.is_some_and(|(hint, _)| hint == t.hint);
            if t.wanted {
                t.progress = wanted.and_then(|(_, progress)| progress);
            }
        }
        if let Some((hint, progress)) = wanted
            && self.tutorial.cancelled_hint() != Some(hint)
            && !self
                .tutorial_toasts
                .iter()
                .any(|t| t.wanted && t.hint == hint)
        {
            if let Some(toast) = self.tutorial_toast(hint, progress) {
                self.tutorial_toasts.push(toast);
            } else {
                self.tutorial.set_cancelled_hint(Some(hint));
            }
        }
        let mut used = self.toast_slots_used();
        for t in &mut self.tutorial_toasts {
            if t.slot.is_none() && t.wanted && t.slots <= Self::TOAST_SLOTS {
                let mask = (1u32 << t.slots) - 1;
                if let Some(slot) =
                    (0..=Self::TOAST_SLOTS - t.slots).find(|s| used & (mask << s) == 0)
                {
                    t.slot = Some(slot);
                    used |= mask << slot;
                }
            }
        }
        let atlas = &self.gui.atlas;
        let mut i = 0;
        while i < self.tutorial_toasts.len() {
            let t = &mut self.tutorial_toasts[i];
            if t.slot.is_none() {
                if !t.wanted {
                    self.tutorial_toasts.remove(i);
                } else {
                    i += 1;
                }
                continue;
            }
            let (sound, finished) = t.slide.update_now(|fully| {
                t.wanted && (t.hint != Hint::SocialInteractions || social_hint_progress(fully).0)
            });
            if let Some(show) = sound {
                let event = if show { "ui.toast.in" } else { "ui.toast.out" };
                self.audio
                    .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
            }
            if finished {
                self.tutorial_toasts.remove(i);
                continue;
            }
            if t.hint == Hint::SocialInteractions {
                let (_, progress) = social_hint_progress(t.slide.fully_for);
                t.progress = Some(progress);
                t.smooth = progress;
            } else if let Some(progress) = t.progress {
                let delta = ((t.slide.fully_for - t.last_smooth) as f32 / 100.0).clamp(0.0, 1.0);
                t.smooth += (progress - t.smooth) * delta;
                t.last_smooth = t.slide.fully_for;
            }
            if !hidden {
                let height = (10 + 11 * t.lines.len().max(2)) as f32;
                let x = batch.width - 160.0 * t.slide.portion;
                let y = 32.0 * t.slot.unwrap_or_default() as f32;
                sunrise_client::ui::nine_slice(
                    batch,
                    atlas,
                    "toast/tutorial",
                    x,
                    y,
                    160.0,
                    height,
                    u32::MAX,
                );
                let icon = match t.hint {
                    Hint::Move => "movement_keys",
                    Hint::Look => "mouse",
                    Hint::FindTree | Hint::PunchTree => "tree",
                    Hint::OpenInventory => "recipe_book",
                    Hint::CraftPlanks => "wooden_planks",
                    Hint::SocialInteractions => "social_interactions",
                };
                batch.sprite_scaled(
                    atlas,
                    &format!("toast/{icon}"),
                    x + 6.0,
                    y + 6.0,
                    20.0,
                    20.0,
                    u32::MAX,
                );
                let text_y = y + 7.0 + ((t.lines.len().max(2) - t.lines.len()) * 11) as f32 / 2.0;
                for (n, (line, _)) in t.lines.iter().enumerate() {
                    batch.text_styled(
                        atlas,
                        line,
                        x + 30.0,
                        text_y + 11.0 * n as f32,
                        if n < t.title_lines {
                            0xFF50_0050
                        } else {
                            0xFF00_0000
                        },
                        false,
                    );
                }
                if let Some(progress) = t.progress {
                    batch.fill(atlas, x + 3.0, y + height - 4.0, 154.0, 1.0, u32::MAX);
                    let color = if progress >= t.smooth {
                        0xFF00_5500
                    } else {
                        0xFF55_0000
                    };
                    batch.fill(
                        atlas,
                        x + 3.0,
                        y + height - 4.0,
                        154.0 * t.smooth.clamp(0.0, 1.0),
                        1.0,
                        color,
                    );
                }
            }
            i += 1;
        }
    }
}
