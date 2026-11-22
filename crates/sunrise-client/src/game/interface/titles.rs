//! Titles, the action bar and the selected item name: the state and layout
//! live in `hud_overlays` (`Titles`, `ToolHighlight`); this feeds and draws them.
use super::Game;
pub(super) use sunrise_client::hud_overlays::Titles;
use sunrise_client::{hud::HudBatch, hud_layout, hud_overlays::HeldName, text::Text};

impl Game<'_> {
    /// `ServerPlayer.sendOverlayMessage`: the steady player sees it on this
    /// game's HUD; a server client receives it as a packet.
    pub(super) fn send_overlay_message(&mut self, message: Text) {
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(sunrise_protocol::packets::play::Clientbound::SystemChat(
                    sunrise_protocol::packets::play::SystemChat {
                        content: sunrise_client::lan::data::component_nbt(&message.component()),
                        overlay: true,
                    },
                ));
        } else if self.options.dedicated.is_none() {
            self.titles.set_overlay(message, false);
        }
    }
    /// `Hud.extractOverlayMessage`: the action bar.
    pub(super) fn draw_overlay_message(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.clock.fraction() as f32;
        let background = self.settings.text_background(0.0);
        let font = &self.gui.atlas.font;
        let draws = self.titles.overlay_message(w, h, partial, font, background);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `Hud.extractTitle`.
    pub(super) fn draw_title(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.clock.fraction() as f32;
        let background = self.settings.text_background(0.0);
        let font = &self.gui.atlas.font;
        let draws = self.titles.title(w, h, partial, font, background);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `Hud.extractSavingIndicator`, under the screens: the option, then the
    /// fading "Saving world" text.
    pub(super) fn draw_saving_indicator(&self, batch: &mut HudBatch) {
        if !self.settings.autosave_indicator {
            return;
        }
        let draws = self.saving_indicator.draw(
            batch.width as i32,
            batch.height as i32,
            self.clock.fraction() as f32,
            &self.gui.atlas.font,
            Text::literal(self.translate_with_fallback("menu.savingLevel", "Saving world")),
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// The selected stack as `Hud.tick` compares it for the name highlight.
    fn held_name(&self) -> Option<HeldName> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        let custom = stack.data().is_some_and(|data| data.custom_name.is_some());
        let previous = self
            .tool_highlight
            .last
            .as_ref()
            .map(|last| &last.hover_name);
        Some(HeldName {
            item: stack.item.to_string(),
            hover_name: sunrise_client::stations::hover_name_in_reusing(
                &self.loop_context,
                &self.pack.names,
                stack,
                previous,
            ),
            rarity_color: self.stack_rarity_color(stack),
            custom_name: custom,
        })
    }
    /// The title, action bar and selected-item parts of `Hud.tick`.
    pub(super) fn tick_hud(&mut self) {
        self.titles.tick();
        // Hud.tick: the vignette after the titles, before the held item's name. (A local world,
        // which has no `Gui.tick` here, eases it in its level tick, `tick_particles`.)
        self.tick_vignette();
        self.tick_in_bed_chat();
        // `MinecraftServer.isCurrentlySaving` of single player's server.
        let saving = self
            .integrated
            .as_ref()
            .map_or(self.autosaving, |server| server.is_saving());
        self.saving_indicator.tick(saving);
        let held = self.held_name();
        self.tool_highlight
            .tick(held, self.settings.notification_display_time);
        self.tool_highlight.tip = self.highlight_tip();
    }
    /// `IItemExtension.getHighlightTip` of the selected stack, while its name
    /// shows and a part of its item listens.
    fn highlight_tip(&self) -> Option<Text> {
        let last = self.tool_highlight.last.as_ref()?;
        if self.tool_highlight.timer <= 0 {
            return None;
        }
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        if !mod_api::__private::item_listens(mod_api::HookId::HighlightTip, &stack.item) {
            return None;
        }
        let mut name = last.styled();
        self.native_highlight_tip(stack, &mut name);
        Some(name)
    }
    /// `Hud.extractSelectedItemName`.
    pub(super) fn draw_selected_name(&self, batch: &mut HudBatch, can_hurt: bool) {
        let draws = self.tool_highlight.draw(
            batch.width as i32,
            batch.height as i32,
            can_hurt,
            &self.gui.atlas.font,
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}
