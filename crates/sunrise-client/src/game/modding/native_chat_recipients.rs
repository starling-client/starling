//! The receivers a mod keeps a public chat line from (`chat_recipient`), asked before the hub
//! broadcasts it.
use super::Game;
use std::cell::RefCell;

thread_local! {
    static FILTERING_CHAT: RefCell<()> = const { RefCell::new(()) };
}

impl Game<'_> {
    /// The placed, connected players a mod keeps a public line from, asked once each in hub order.
    /// Prepare the sender, name and message only when a mod listens. `None` refuses a line
    /// issued during recipient selection: the listener may not reenter or bypass its own veto.
    pub(in crate::game) fn chat_hidden_from<N: AsRef<str>, M: AsRef<str>>(
        &mut self,
        fields: impl FnOnce(&Self) -> (u128, N, M),
    ) -> Option<Vec<u128>> {
        if !mod_api::__private::listened(mod_api::HookId::ChatRecipient) {
            return Some(Vec::new());
        }
        FILTERING_CHAT.with(|active| {
            let _filtering = active.try_borrow_mut().ok()?;
            let Some(host) = self.lan.as_ref() else {
                return Some(Vec::new());
            };
            let receivers: Vec<u128> = host.hub.online_players().map(|g| g.uuid).collect();
            let (sender, name, message) = fields(self);
            let (name, message) = (name.as_ref(), message.as_ref());
            Some(
                receivers
                    .into_iter()
                    .filter(|&recipient| {
                        let mut hidden = false;
                        self.native_chat_recipient(sender, name, message, recipient, &mut hidden);
                        hidden
                    })
                    .collect(),
            )
        })
    }
}
