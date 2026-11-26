//! Client-host notifications use the ordinary system-toast update and slots.
#[cfg(test)]
#[path = "mod_toasts/tests.rs"]
mod tests;
use super::{Game, toast_manager::Toast};
use mod_api::{AddToastError, Identifier, ModToast, ToastIcon, ToastKind};
use sunrise_client::{
    hud::HudBatch,
    items::ItemStack,
    toasts::{SystemToast, SystemToastLayout},
};
use sunrise_client_lib::api_text::to_engine;

pub(super) struct Pending {
    pub(super) toast: SystemToast,
    kind: Option<Identifier>,
    icon: Option<Icon>,
}

pub(super) struct Entry {
    pub(super) request: Pending,
    pub(super) layout: SystemToastLayout,
}

enum Icon {
    Item(ItemStack),
    Sprite(String),
}

impl Game<'_> {
    /// The calling mod holds new toasts back, or lets them go (`ToastManager.freeSlotCount`).
    pub(in crate::game) fn set_toasts_held(&self, held: bool) -> bool {
        self.toast_manager
            .hold(mod_api::content::crash::current_native_mod(), held)
    }

    /// Queue from a shared ClientHost without dispatching back into a running hook.
    pub(in crate::game) fn add_toast(&self, request: ModToast) -> Result<(), AddToastError> {
        let title = to_engine(&request.title).map_err(AddToastError::Text)?;
        let description = request
            .description
            .as_ref()
            .map(to_engine)
            .transpose()
            .map_err(AddToastError::Text)?;
        let mut toast = SystemToast::new(title, description);
        if let Some(ms) = request.display_ms {
            toast = toast.with_display_time(ms as f64);
        }
        let icon = match request.icon {
            Some(ToastIcon::Item(stack)) => Some(Icon::Item(
                super::api_values::stack_value(&stack).map_err(AddToastError::Icon)?,
            )),
            Some(ToastIcon::Sprite(sprite)) => Some(Icon::Sprite(sprite.to_string())),
            None | Some(_) => None,
        };
        self.toast_manager.pending_mod.borrow_mut().push(Pending {
            toast,
            kind: request.kind,
            icon,
        });
        Ok(())
    }

    pub(super) fn admit_mod_toasts(&mut self) {
        // Take one snapshot before invoking listeners. A request from toast_add is
        // for the next update, not a recursive add or part of this snapshot.
        for mut request in std::mem::take(self.toast_manager.pending_mod.get_mut()) {
            if !self.admit_toast(
                ToastKind::Mod,
                &mut request.toast.title,
                &mut request.toast.message,
            ) {
                continue;
            }
            let layout = request
                .toast
                .layout(&self.gui.atlas, request.icon.is_some());
            if let Some(kind) = &request.kind
                && let Some(entry) = self.toast_manager.find_mut(|entry| match entry {
                    Toast::Mod(entry) if entry.request.kind.as_ref() == Some(kind) => Some(entry),
                    _ => None,
                })
            {
                entry.request.toast.reset_from(request.toast);
                entry.request.icon = request.icon;
                entry.layout = layout;
            } else {
                let clock = sunrise_client::toasts::ToastClock::of(self.clock.still());
                self.toast_manager
                    .post(clock, Toast::Mod(Entry { request, layout }));
            }
        }
    }

    pub(super) fn draw_mod_toast(&self, batch: &mut HudBatch, entry: &Entry, slot: usize) {
        let atlas = &self.gui.atlas;
        let [x, y] = entry
            .request
            .toast
            .draw_wrapped(&entry.layout, atlas, batch, slot);
        // AdvancementToast's icon: 16 pixels square at (8, 8).
        let (x, y) = (x + 8.0, y + 8.0);
        match &entry.request.icon {
            Some(Icon::Item(stack)) => self.draw_item_model_with_owner(batch, stack, x, y, None),
            Some(Icon::Sprite(id)) => batch.sprite_scaled(atlas, id, x, y, 16.0, 16.0, u32::MAX),
            None => {}
        }
    }
}
