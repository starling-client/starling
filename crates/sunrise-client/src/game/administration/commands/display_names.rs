//! The two names a command reads from an entity: `Entity.getDisplayName` for
//! command replies (the team-formatted name with `show_entity` hover and UUID
//! insertion, with click events removed from the name and its siblings) and
//! `Nameable.getPlainTextName` for the selector's `name=`.
use super::super::Game;
use super::Target;
use sunrise_client::text::Text;

impl Game<'_> {
    /// Decoded `CustomName` with `Entity.removeAction` applied, or `None` when
    /// absent or invalid under vanilla's `ComponentSerialization.CODEC`.
    fn custom_name_of(&self, id: u32) -> Option<Text> {
        let raw = self.persist_raw(id)?.get("CustomName")?;
        let decoded = sunrise_client::entity_nbt::decode_custom_name_text(raw)?;
        Some(sunrise_client::scoreboard::remove_click_events(&decoded))
    }
    /// `Entity.getName`: the decoded `CustomName` with click actions removed,
    /// else the entity's type name, free of team formatting, hover or insertion.
    pub(super) fn target_base_name(&self, t: Target) -> Text {
        match t {
            Target::Player => Text::literal(self.acting_player_name()),
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or_else(Text::empty, |g| Text::literal(&g.name)),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(|| Text::translate("entity.", &[]), |m| m.command_name()),
            // ItemEntity.getName: its CustomName, else the stack's item name
            // (`getItemName`: a mod item's Part, else the item's own `getName`).
            Target::Item(id) => {
                if let Some(name) = self.custom_name_of(id) {
                    return name;
                }
                match self.dropped.iter().find(|i| i.id == id) {
                    Some(item) => self.stack_item_name_text(&item.stack),
                    None => Text::literal(self.pack.names.display_item("")),
                }
            }
            // FallingBlockEntity.getTypeName: `entity.minecraft.falling_block_type`
            // with the block's name.
            Target::Falling(id) => {
                let custom = self
                    .falling
                    .iter()
                    .find(|f| f.id == id)
                    .and_then(|f| f.saved.as_ref())
                    .and_then(|saved| saved.1.get("CustomName"))
                    .or_else(|| self.persist_raw(id).and_then(|r| r.get("CustomName")))
                    .and_then(sunrise_client::entity_nbt::decode_custom_name_text)
                    .map(|text| sunrise_client::scoreboard::remove_click_events(&text));
                if let Some(name) = custom {
                    return name;
                }
                let block = self
                    .falling
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| self.res.facts().get(f.state).name.replace(':', "."))
                    .unwrap_or_default();
                let block = Text::translate(&format!("block.{block}"), &[]);
                Text::translate("entity.minecraft.falling_block_type", &[block])
            }
            Target::Bolt(id) => self.bolt_plain_name(id),
            // A general row keeps its name on the row, not in a saved compound.
            Target::General(id) => {
                match self
                    .resident_entities
                    .general
                    .row(id)
                    .and_then(|row| row.custom_name.as_ref())
                {
                    Some(name) => sunrise_client::scoreboard::remove_click_events(name),
                    None => Text::translate(
                        &format!("entity.{}", self.target_type(t).replace(':', ".")),
                        &[],
                    ),
                }
            }
            other => {
                if let Some(name) = super::super::native_entities::target_id(other)
                    .and_then(|id| self.custom_name_of(id))
                {
                    return name;
                }
                let ty = self.target_type(other);
                Text::translate(&format!("entity.{}", ty.replace(':', ".")), &[])
            }
        }
    }
    /// `Nameable.getPlainTextName`: `getName().getString()`, the name without
    /// the team's prefix and suffix that `getDisplayName` adds.
    pub(in crate::game) fn target_plain_name(&self, t: Target) -> String {
        match t {
            Target::Player => self.acting_player_name().to_owned(),
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or_else(String::new, |g| g.name.clone()),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| m.command_name().plain()),
            other => self.target_base_name(other).plain(),
        }
    }
    pub(in crate::game) fn bolt_plain_name(&self, id: u32) -> Text {
        if let Some(name) = self
            .bolts
            .iter()
            .find(|b| b.id == id)
            .and_then(|b| b.custom_name.as_ref())
            .and_then(sunrise_client::entity_nbt::decode_custom_name_text)
        {
            return sunrise_client::scoreboard::remove_click_events(&name);
        }
        Text::translate("entity.minecraft.lightning_bolt", &[])
    }
    /// Shared `Entity.getDisplayName` decoration: `PlayerTeam.formatNameForTeam`
    /// of `getName()`, then `withHoverEvent(createHoverEvent())` (a `ShowEntity`
    /// of the type, UUID and name) and `withInsertion(getStringUUID())`.
    fn decorate_entity_display_name(
        &self,
        id: u32,
        kind: &str,
        team: Option<&sunrise_client::scoreboard::Team>,
        name: Text,
    ) -> Text {
        Self::decorate_entity_display_name_with_uuid(kind, self.persist.uuid_of(id), team, name)
    }
    fn decorate_entity_display_name_with_uuid(
        kind: &str,
        uuid: Option<[i32; 4]>,
        team: Option<&sunrise_client::scoreboard::Team>,
        name: Text,
    ) -> Text {
        let shown = match team {
            Some(team) => team.formatted_name(name.clone()),
            None => name.clone(),
        };
        let Some(uuid) = uuid else {
            return shown;
        };
        let uuid = sunrise_client::save::uuid_text(uuid);
        let kind = sunrise_client::command::args::normalize(kind);
        shown.set_component_fields(vec![
            (
                "hover_event".into(),
                serde_json::json!({
                    "action": "show_entity",
                    "id": kind,
                    "uuid": uuid,
                    "name": name.component(),
                }),
            ),
            ("insertion".into(), uuid.into()),
        ])
    }
    /// `Entity.getDisplayName` for the mob `id`.
    pub(in crate::game) fn mob_command_name(&self, id: u32) -> Text {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return Text::translate("entity.", &[]);
        };
        self.decorate_entity_display_name(id, &m.kind.id, self.mob_team_of(m), m.command_name())
    }
    /// `Entity.getDisplayName` for any non-player, non-mob target.
    pub(super) fn entity_display_name(&self, t: Target) -> Text {
        let name = self.target_base_name(t);
        let Some(id) = super::super::native_entities::target_id(t) else {
            return name;
        };
        let kind = self.target_type(t);
        self.decorate_entity_display_name(id, &kind, self.entity_team(id), name)
    }
    /// Capture enough of `t` before `/kill` removes its storage to build
    /// `Entity.getDisplayName` afterwards (`KillCommand` calls `getDisplayName`
    /// after `entity.kill`, when `Scoreboard.entityRemoved` has already left
    /// any team for removed non-player entities).
    pub(super) fn capture_kill_display(&self, t: Target) -> KillDisplay {
        match t {
            Target::Player | Target::Guest(_) | Target::Mob(_) => {
                KillDisplay::Ready(self.target_name(t))
            }
            other => KillDisplay::Entity {
                kind: self.target_type(other),
                uuid: super::super::native_entities::target_id(other)
                    .and_then(|id| self.persist.uuid_of(id)),
                name: self.target_base_name(other),
            },
        }
    }
    /// Finish `Entity.getDisplayName` for a killed target against the post-kill
    /// scoreboard team state.
    pub(super) fn finish_kill_display(&self, captured: KillDisplay) -> Text {
        match captured {
            KillDisplay::Ready(text) => text,
            KillDisplay::Entity { kind, uuid, name } => {
                let holder = uuid.map(sunrise_client::save::uuid_text);
                let team = holder
                    .as_deref()
                    .and_then(|holder| self.score.board.players_team(holder));
                Self::decorate_entity_display_name_with_uuid(&kind, uuid, team, name)
            }
        }
    }
}

/// Pre-kill identity snapshot so `KillCommand` can format `Entity.getDisplayName`
/// after `entity.kill` removes the entity from its storage and team.
pub(super) enum KillDisplay {
    Ready(Text),
    Entity {
        kind: String,
        uuid: Option<[i32; 4]>,
        name: Text,
    },
}
