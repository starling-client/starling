use super::*;

/// `ChatType`'s two decorations: how a message shows in the chat, and what the narrator says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum Decoration {
    Chat,
    Narration,
}

/// `ChatTypeDecoration.decorate` of a chat type's `chat` or `narration`
/// entry: its translation, with the sender, the target or the message as its
/// parameters, in its style. A chat type the registry lacks decorates as
/// `minecraft:chat` (`chat.type.text`, narrated `chat.type.text.narrate`).
pub(super) fn chat_type_decorate(
    chat_type: Option<&Tag>,
    decoration: Decoration,
    bound: &play::ChatTypeBound,
    content: &serde_json::Value,
) -> serde_json::Value {
    use sunrise_client::scoreboard::tag_to_json;
    let (field, default_key) = match decoration {
        Decoration::Chat => ("chat", "chat.type.text"),
        Decoration::Narration => ("narration", "chat.type.text.narrate"),
    };
    let decoration = chat_type.and_then(|t| t.get(field));
    let key = match decoration.and_then(|d| d.get("translation_key")) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => default_key.into(),
    };
    let parameter = |name: &str| match name {
        "sender" => tag_to_json(&bound.name),
        "target" => bound
            .target
            .as_ref()
            .map_or_else(|| serde_json::Value::from(""), tag_to_json),
        _ => content.clone(),
    };
    let with: Vec<serde_json::Value> = match decoration.and_then(|d| d.get("parameters")) {
        Some(Tag::List(list)) => list
            .iter()
            .map(|p| match p {
                Tag::String(s) => parameter(&s.to_string_lossy()),
                _ => serde_json::Value::from(""),
            })
            .collect(),
        _ => vec![parameter("sender"), parameter("content")],
    };
    let mut component = match decoration.and_then(|d| d.get("style")) {
        Some(style @ Tag::Compound(_)) => tag_to_json(style),
        _ => serde_json::json!({}),
    };
    component["translate"] = key.into();
    component["with"] = with.into();
    component
}

/// `ChatTrustLevel`; a modified message keeps the signed content it was changed from.
pub(in super::super) enum ChatTrust {
    Secure,
    Modified(String),
    NotSecure,
    /// `GuiMessageTag.chatError`: the line `ChatListener.handleChatMessageError` adds.
    Error,
}
impl ChatTrust {
    /// `ChatTrustLevel.createTag`.
    pub(super) fn tag(&self) -> Option<sunrise_client::chat::Tag> {
        match self {
            Self::Secure => None,
            Self::Modified(original) => Some(sunrise_client::chat::Tag::modified(original)),
            Self::NotSecure => Some(sunrise_client::chat::Tag::not_secure()),
            Self::Error => Some(sunrise_client::chat::Tag::chat_error()),
        }
    }
}

impl Game<'_> {
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage). The lines the Chat option hides
    /// are dropped there (its `visibleMessageFilter`).
    pub(in crate::game) fn add_chat_line(
        &mut self,
        content: sunrise_client::text::Text,
        source: sunrise_client::chat::ChatSource,
    ) -> bool {
        self.signed_chat_line(content, source, None, ChatTrust::Secure)
    }
    /// `chat_line` of a player message, keeping its signature for a
    /// deletion (`GuiMessage.signature`).
    pub(super) fn signed_chat_line(
        &mut self,
        content: sunrise_client::text::Text,
        source: sunrise_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
    ) -> bool {
        if !self.ui.chat.shows(source) {
            return false;
        }
        let text = content.plain();
        let focused = self.screen == Screen::Chat;
        if source == sunrise_client::chat::ChatSource::Player {
            self.ui.chat.add_signed(
                &self.gui.atlas,
                content,
                signature,
                trust.tag(),
                self.visual_ticks(),
                focused,
            );
        } else {
            // addServerSystemMessage and addClientSystemMessage both carry
            // GuiMessageTag.systemSinglePlayer: the grey bar at the line's left.
            let indicator = Some(sunrise_client::chat::SYSTEM_INDICATOR);
            self.ui.chat.add(
                &self.gui.atlas,
                content,
                indicator,
                source,
                self.visual_ticks(),
                focused,
            );
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
        true
    }
    /// `ChatListener.handleMessage` for player and disguised chat: held in the
    /// queue while Chat Delay is set, shown at once otherwise. `narration` is
    /// the chat type's narration of it (`decorateNarration`).
    pub(super) fn handle_player_chat(&mut self, chat: QueuedChat) {
        if self.chat_delay_ms() > 0 {
            if let Some(r) = self.remote.as_mut() {
                r.chat_queue.push_back(chat);
            }
        } else {
            self.accept_chat(chat);
        }
    }
    /// `ChatListener.handleChatMessageError`: a message that fails validation shows as the red,
    /// italic `chat.validation_error` in its chat type (queued under Chat Delay as any
    /// message). Its signature counts as processed but not shown, for the acknowledgement the
    /// client sends the server. Vanilla also
    /// skips a blocked sender and one restricted to friends (not kept here either) and
    /// narrates the line.
    pub(super) fn handle_chat_message_error(
        &mut self,
        chat_type: &play::ChatTypeBound,
        signature: Option<&Signature>,
    ) {
        if let Some(signature) = signature {
            self.mark_message_processed(signature, false);
        }
        let error = serde_json::json!({
            "translate": "chat.validation_error",
            "color": "red",
            "italic": true,
        });
        let line = self.decorate_chat(chat_type, &error, Decoration::Chat);
        // saySystemChatQueued(boundChatType.decorateNarration(CHAT_VALIDATION_ERROR)).
        let narration = self
            .decorate_chat(chat_type, &error, Decoration::Narration)
            .plain();
        self.handle_player_chat(QueuedChat {
            line,
            signature: None,
            trust: ChatTrust::Error,
            narration,
            sender: 0,
            chat_type: chat_type.chat_type,
            player: None,
        });
    }
    /// `ChatTrustLevel.evaluate` of a player message, with
    /// `ChatListener.evaluateTrustLevel`: the player's own message on their
    /// own server is secure. `decorated` is the line shown, `unsigned` the
    /// server's replacement for the signed content.
    pub(super) fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        content: &str,
        unsigned: Option<&Tag>,
        decorated: &sunrise_client::text::Text,
    ) -> ChatTrust {
        use sunrise_client::scoreboard::{comp_text, tag_to_json};
        if self.integrated.is_some() && self.remote.as_ref().is_some_and(|r| r.profile == sender) {
            return ChatTrust::Secure;
        }
        // PlayerChatMessage.hasExpiredClient: MESSAGE_EXPIRES_AFTER_CLIENT is
        // the server's five minutes and two more.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let expired = now
            > timestamp.saturating_add(
                sunrise_protocol::secure_chat::EXPIRES_AFTER_SERVER + 2 * 60 * 1000,
            );
        if !signed || expired {
            return ChatTrust::NotSecure;
        }
        // ChatTrustLevel.isModified: the signed content is not in the line, or
        // the replacement uses a font other than the default.
        let font_changed = unsigned.is_some_and(|tag| {
            comp_text(&tag_to_json(tag)).spans.iter().any(|span| {
                span.style
                    .font
                    .as_deref()
                    .is_some_and(|font| font != "minecraft:default")
            })
        });
        if !decorated.plain().contains(content) || font_changed {
            ChatTrust::Modified(content.to_owned())
        } else {
            ChatTrust::Secure
        }
    }
    /// `ClientPacketListener.handleDeleteChat`: the packed signature names a
    /// message; one still held back by Chat Delay never shows
    /// (`ChatListener.removeFromDelayedMessageQueue`), a shown one is
    /// replaced (`ChatComponent.deleteMessage`). A signature the cache
    /// doesn't hold is an invalid packet.
    pub(super) fn handle_delete_chat(&mut self, packed: &(i32, Option<Vec<u8>>)) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let Some(signature) = r.signatures.unpack(packed) else {
            let reason = Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason));
        };
        r.secure_chat.ignore_pending(&signature);
        let queued = r.chat_queue.len();
        r.chat_queue
            .retain(|chat| chat.signature.as_ref() != Some(&signature));
        if r.chat_queue.len() == queued {
            let ticks = self.visual_ticks();
            self.ui
                .chat
                .delete_message(&self.gui.atlas, &signature, ticks);
        }
        Ok(())
    }
    /// A message's `accept`: shown, it is narrated (`narrateChatMessage`) and
    /// starts the Chat Delay wait again.
    pub(super) fn accept_chat(&mut self, chat: QueuedChat) -> bool {
        let signature = chat.signature;
        let shown = self.player_chat_line(chat);
        if let Some(signature) = &signature {
            self.mark_message_processed(signature, shown);
        }
        if shown && let Some(r) = self.remote.as_mut() {
            r.chat_previous = Some(Instant::now());
        }
        shown
    }
    /// `showMessageToPlayer`: the line joins the chat if Only Show Secure
    /// Chat, mods and the Chat option let it through, and the narrator says
    /// it (`sayChatQueued`).
    pub(super) fn player_chat_line(&mut self, chat: QueuedChat) -> bool {
        let QueuedChat {
            mut line,
            signature,
            trust,
            narration,
            sender,
            chat_type,
            player,
        } = chat;
        // ChatListener.showMessageToPlayer: Only Show Secure Chat drops what is not secure.
        if matches!(trust, ChatTrust::NotSecure) && self.settings.file.bool("onlyShowSecureChat") {
            return false;
        }
        // handleChatMessageError says its line as system chat, and mods are not told of it.
        let error = matches!(trust, ChatTrust::Error);
        let (system, kind, player) = (false, Some(chat_type), player.as_ref());
        if !error && !self.chat_received(system, false, sender, kind, player, &mut line) {
            return false;
        }
        let source = sunrise_client::chat::ChatSource::Player;
        let shown = self.signed_chat_line(line, source, signature, trust);
        if shown && error {
            self.say_system_chat_queued(&sunrise_client::text::Text::untrusted(&narration));
        } else if shown {
            self.say_chat_queued(&narration);
        }
        shown
    }
    /// The server's last `n` chat lines, newest first.
    pub(in crate::game) fn remote_chat(&self, n: usize) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        Some(r.chat.iter().rev().take(n).map(|l| l.0.clone()).collect())
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`. Starling: legacy `§` codes in it (which plugins
    /// still send) style the line, as `remote_text`'s do.
    pub(in crate::game) fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> sunrise_client::text::Text {
        use sunrise_client::scoreboard::comp_text;
        let Some(r) = self.remote.as_ref() else {
            return comp_text(content).resolve_legacy();
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content))
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/network/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(in crate::game) fn send_signed_chat_input(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            if self.studio_command(command) {
                return;
            }
            self.send_command(command);
        } else {
            self.send_signed_chat(message);
        }
    }
    /// ChatScreen.handleChatInput fires the client event for chat, not commands.
    pub(in crate::game) fn send_chat_from_screen(&mut self, message: &str) {
        if message.starts_with('/') || !mod_api::__private::listened(mod_api::HookId::ClientChat) {
            self.send_signed_chat_input(message);
        } else if let Some(uuid) = self.remote.as_ref().map(|r| r.profile) {
            let id = self.own_entity_id();
            let sent = chat_after_event(message, |outgoing, cancel| {
                self.native_client_chat(id, uuid, message, outgoing, cancel);
            });
            if let Some(outgoing) = sent {
                self.send_signed_chat(&outgoing);
            }
        }
    }
}
