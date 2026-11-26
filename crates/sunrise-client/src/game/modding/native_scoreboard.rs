//! Scoreboard objective handles for native server mods.
//!
//! Every write is made on the same `Scoreboard` the `/scoreboard` command writes, with the same
//! methods: the per-tick sync to clients (`lan/scoreboard.rs`) and the save (`save_scoreboard`)
//! need to know nothing of a mod. An objective's handle holds the world's epoch and the identity
//! the board gave the objective when it was added, so a removed objective stays stale when its
//! name is added again.
use super::Game;
use core::ops::ControlFlow;
use mod_api::__private::game::*;
use mod_api::{
    DisplaySlot, NumberFormat, ObjectiveError, ObjectiveReadError, ObjectiveRef,
    ObjectiveRenderType, ScoreEntry, ScoreHolder, ServerReadError, TextFormError, WorldEpoch,
};
use sunrise_client::{
    scoreboard::{self as sb, Comp, comp_text, literal},
    text::Text as EngineText,
};

fn engine_render_type(render: ObjectiveRenderType) -> sb::RenderType {
    match render {
        ObjectiveRenderType::Hearts => sb::RenderType::Hearts,
        _ => sb::RenderType::Integer,
    }
}

pub(super) fn api_render_type(render: sb::RenderType) -> ObjectiveRenderType {
    match render {
        sb::RenderType::Integer => ObjectiveRenderType::Integer,
        sb::RenderType::Hearts => ObjectiveRenderType::Hearts,
    }
}

/// A text as the API holds it; one too deep for the API is refused by name, never flattened.
pub(super) fn api_text(text: &EngineText) -> Result<mod_api::Text, TextFormError> {
    use super::api_values::TryIntoApi;
    text.try_into_api()
}

fn engine_text(text: &mod_api::Text) -> Result<EngineText, TextFormError> {
    sunrise_client_lib::api_text::to_serialized_engine(text)
}

fn engine_comp(text: &mod_api::Text) -> Result<Comp, TextFormError> {
    Ok(engine_text(text)?.component())
}

/// The board's number format for the API's (`NumberFormatTypes`).
fn engine_number_format(format: &NumberFormat) -> Result<sb::NumberFormat, TextFormError> {
    Ok(match format {
        NumberFormat::Fixed(text) => sb::NumberFormat::Fixed(engine_comp(text)?),
        NumberFormat::Styled(style) => {
            // The style's JSON is a component's, less its content.
            let mut json = engine_comp(&mod_api::Text::literal("").with_style(style.clone()))?;
            if let Comp::Object(fields) = &mut json {
                fields.remove("text");
            } else {
                json = Comp::Object(Default::default());
            }
            sb::NumberFormat::Styled(json)
        }
        _ => sb::NumberFormat::Blank,
    })
}

pub(super) fn api_number_format(format: &sb::NumberFormat) -> Result<NumberFormat, TextFormError> {
    Ok(match format {
        sb::NumberFormat::Blank => NumberFormat::Blank,
        sb::NumberFormat::Fixed(comp) => NumberFormat::Fixed(api_text(&comp_text(comp))?),
        sb::NumberFormat::Styled(style) => {
            let mut json = style.clone();
            if let Comp::Object(fields) = &mut json {
                fields.insert("text".into(), Comp::String(String::new()));
            }
            NumberFormat::Styled(std::mem::take(&mut api_text(&comp_text(&json))?.style))
        }
    })
}

pub(super) fn objective_slots<'a>(
    board: &'a sb::Scoreboard,
    name: &'a str,
) -> impl Iterator<Item = DisplaySlot> + 'a {
    DisplaySlot::all().filter(move |slot| {
        board
            .display
            .get(slot.name())
            .is_some_and(|objective| objective.as_str() == name)
    })
}

impl Game<'_> {
    /// The objective a handle names: None for a stale handle, one of an earlier open, or on a
    /// client.
    fn objective_state(&self, objective: ObjectiveRef) -> Option<&sb::Objective> {
        if self.remote.is_some() || self.native_lifecycle.epoch() != Some(objective.fields().epoch)
        {
            return None;
        }
        self.score
            .board
            .objective_by_identity(objective.fields().id)
    }

    fn objective_visit_state(
        &self,
        objective: ObjectiveRef,
    ) -> Result<&sb::Objective, ObjectiveReadError> {
        if !self.is_logical_server() {
            return Err(ObjectiveReadError::Unavailable);
        }
        self.objective_state(objective)
            .ok_or(ObjectiveReadError::Stale)
    }

    pub(super) fn scoreboard_visit_epoch(&self) -> Result<WorldEpoch, ServerReadError> {
        if !self.is_logical_server() || self.remote.is_some() {
            return Err(ServerReadError::Unavailable);
        }
        self.native_lifecycle
            .epoch()
            .ok_or(ServerReadError::Unavailable)
    }

    fn objective_handle(&self, objective: &sb::Objective) -> Option<ObjectiveRef> {
        Some(objective_ref(
            self.native_lifecycle.epoch()?,
            objective.identity.0,
        ))
    }

    /// The name of the objective `objective` names, for a write: the board is loaded, and the
    /// savepoint of an open transaction is kept before anything changes.
    fn objective_for_write(&mut self, objective: ObjectiveRef) -> Result<String, ObjectiveError> {
        self.load_scoreboard();
        let name = self
            .objective_state(objective)
            .ok_or(ObjectiveError::Stale)?
            .name
            .clone();
        self.journal_scoreboard();
        Ok(name)
    }

    /// The key a holder has on the board, for a read: a player's name, a non-player entity's
    /// UUID if it has one yet (no score can exist before), or the name as given.
    pub(super) fn score_holder_key(&self, holder: &ScoreHolder) -> Option<String> {
        match holder {
            ScoreHolder::Name(name) => Some(name.clone()),
            ScoreHolder::Player(player) => self.player_name(*player).map(str::to_owned),
            ScoreHolder::Entity(entity) => {
                self.existing_holder_name(self.native_entity_target(*entity)?)
            }
        }
    }

    /// The key and the display name of a holder, for a write; a non-player entity that has no
    /// UUID yet gets one, as its name is asked for.
    pub(super) fn score_holder_for_write(
        &mut self,
        holder: &ScoreHolder,
    ) -> Option<(String, Option<Comp>)> {
        let target = match holder {
            ScoreHolder::Name(name) => return Some((name.clone(), Some(literal(name)))),
            ScoreHolder::Player(player) => {
                self.player_name(*player)?;
                self.native_entity_target(player.entity())?
            }
            ScoreHolder::Entity(entity) => self.native_entity_target(*entity)?,
        };
        let holder = self.holder_for(target)?;
        Some((holder.name, holder.display))
    }

    /// The objective, the holder's key and display for a score write: refused for a stale
    /// objective, a read-only criteria, or a holder that does not resolve.
    fn score_write(
        &mut self,
        objective: ObjectiveRef,
        holder: &ScoreHolder,
    ) -> Result<(String, String, Option<Comp>), ObjectiveError> {
        let name = self.objective_for_write(objective)?;
        let criteria = &self
            .objective_state(objective)
            .ok_or(ObjectiveError::Stale)?
            .criteria;
        if sb::criteria_read_only(criteria) {
            return Err(ObjectiveError::ReadOnlyCriteria);
        }
        let (key, display) = self
            .score_holder_for_write(holder)
            .ok_or(ObjectiveError::UnknownHolder)?;
        Ok((name, key, display))
    }

    /// `Scoreboard.getObjective`.
    pub(super) fn objective(&self, name: &str) -> Option<ObjectiveRef> {
        if self.remote.is_some() {
            return None;
        }
        self.objective_handle(self.score.board.objective(name)?)
    }

    /// `Scoreboard.getObjectives`.
    pub(super) fn visit_objectives(
        &self,
        visit: &mut dyn FnMut(ObjectiveRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        let epoch = self.scoreboard_visit_epoch()?;
        Ok(self
            .score
            .board
            .objectives
            .iter()
            .map(|o| objective_ref(epoch, o.identity.0))
            .try_for_each(visit))
    }

    /// `Scoreboard.addObjective`.
    pub(super) fn add_objective(
        &mut self,
        name: &str,
        criteria: &str,
        display_name: EngineText,
        render_type: ObjectiveRenderType,
    ) -> Result<ObjectiveRef, ObjectiveError> {
        if self.remote.is_some() || self.native_lifecycle.epoch().is_none() {
            return Err(ObjectiveError::Stale);
        }
        self.load_scoreboard();
        self.journal_scoreboard();
        if self.score.board.objective(name).is_some() {
            return Err(ObjectiveError::NameTaken);
        }
        let Ok(Some(criteria)) = sb::criteria_by_name(criteria, &self.ui.registries) else {
            return Err(ObjectiveError::UnknownCriteria);
        };
        self.score.board.add_objective(
            name,
            &criteria,
            display_name.component(),
            engine_render_type(render_type),
            false,
            None,
        );
        self.sync_score_registries();
        self.score
            .board
            .objective(name)
            .and_then(|o| self.objective_handle(o))
            .ok_or(ObjectiveError::Stale)
    }

    /// `Scoreboard.getDisplayObjective`.
    pub(super) fn display_slot_objective(&self, slot: DisplaySlot) -> Option<ObjectiveRef> {
        if self.remote.is_some() {
            return None;
        }
        self.objective_handle(self.score.board.display_objective(slot.name())?)
    }

    /// `Scoreboard.setDisplayObjective`.
    pub(super) fn set_display_slot(
        &mut self,
        slot: DisplaySlot,
        objective: Option<ObjectiveRef>,
    ) -> bool {
        let name = match objective {
            Some(objective) => match self.objective_for_write(objective) {
                Ok(name) => Some(name),
                Err(_) => return false,
            },
            None => {
                self.load_scoreboard();
                if self.remote.is_some() {
                    return false;
                }
                self.journal_scoreboard();
                None
            }
        };
        self.score
            .board
            .set_display_objective(slot.name(), name.as_deref());
        true
    }

    /// `Objective.getName`.
    pub(super) fn objective_name(&self, objective: ObjectiveRef) -> Option<&str> {
        self.objective_state(objective).map(|o| o.name.as_str())
    }

    /// `Objective.getCriteria`.
    pub(super) fn objective_criteria(&self, objective: ObjectiveRef) -> Option<&str> {
        self.objective_state(objective).map(|o| o.criteria.as_str())
    }

    /// `Objective.getDisplayName`.
    pub(super) fn objective_display_name(
        &self,
        objective: ObjectiveRef,
    ) -> Result<EngineText, ObjectiveError> {
        self.objective_state(objective)
            .map(|o| o.display_text())
            .ok_or(ObjectiveError::Stale)
    }

    /// `Objective.getRenderType`.
    pub(super) fn objective_render_type(
        &self,
        objective: ObjectiveRef,
    ) -> Option<ObjectiveRenderType> {
        self.objective_state(objective)
            .map(|o| api_render_type(o.render_type))
    }

    /// `Objective.numberFormat`.
    pub(super) fn objective_number_format(
        &self,
        objective: ObjectiveRef,
    ) -> Result<Option<NumberFormat>, ObjectiveError> {
        let state = self
            .objective_state(objective)
            .ok_or(ObjectiveError::Stale)?;
        Ok(state
            .number_format
            .as_ref()
            .map(api_number_format)
            .transpose()?)
    }

    /// `Objective.displayAutoUpdate`.
    pub(super) fn objective_display_auto_update(&self, objective: ObjectiveRef) -> Option<bool> {
        self.objective_state(objective)
            .map(|o| o.display_auto_update)
    }

    /// The display slots that show the objective, in id order.
    pub(super) fn objective_visit_slots(
        &self,
        objective: ObjectiveRef,
        visit: &mut dyn FnMut(DisplaySlot) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ObjectiveReadError> {
        let name = &self.objective_visit_state(objective)?.name;
        Ok(objective_slots(&self.score.board, name).try_for_each(visit))
    }

    /// `Scoreboard.getPlayerScoreInfo`'s value.
    pub(super) fn objective_score(
        &self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
    ) -> Option<i32> {
        let name = &self.objective_state(objective)?.name;
        let key = self.score_holder_key(&holder)?;
        self.score.board.score(&key, name).map(|s| s.value)
    }

    /// `Scoreboard.listPlayerScores`.
    pub(super) fn objective_visit_scores(
        &self,
        objective: ObjectiveRef,
        visit: &mut dyn for<'a> FnMut(ScoreEntry<'a>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ObjectiveReadError> {
        let name = &self.objective_visit_state(objective)?.name;
        Ok(self
            .score
            .board
            .score_rows(name)
            .map(|(holder, score)| ScoreEntry::new(holder, score.value))
            .try_for_each(visit))
    }

    /// `ScoreAccess.locked`.
    pub(super) fn objective_score_locked(
        &self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
    ) -> Option<bool> {
        let name = &self.objective_state(objective)?.name;
        let key = self.score_holder_key(&holder)?;
        self.score.board.score(&key, name).map(|s| s.locked)
    }

    /// `Objective.setDisplayName`.
    pub(super) fn objective_set_display_name(
        &mut self,
        objective: ObjectiveRef,
        name: EngineText,
    ) -> Result<(), ObjectiveError> {
        let objective = self.objective_for_write(objective)?;
        if let Some(o) = self.score.board.objective_mut(&objective) {
            o.display_name = name.component();
        }
        Ok(())
    }

    /// `Objective.setRenderType`.
    pub(super) fn objective_set_render_type(
        &mut self,
        objective: ObjectiveRef,
        render_type: ObjectiveRenderType,
    ) -> bool {
        let Ok(name) = self.objective_for_write(objective) else {
            return false;
        };
        if let Some(o) = self.score.board.objective_mut(&name) {
            o.render_type = engine_render_type(render_type);
        }
        true
    }

    /// `Objective.setNumberFormat`.
    pub(super) fn objective_set_number_format(
        &mut self,
        objective: ObjectiveRef,
        format: Option<NumberFormat>,
    ) -> Result<(), ObjectiveError> {
        let format = format.as_ref().map(engine_number_format).transpose()?;
        let name = self.objective_for_write(objective)?;
        if let Some(o) = self.score.board.objective_mut(&name) {
            o.number_format = format;
        }
        Ok(())
    }

    /// `Objective.setDisplayAutoUpdate`.
    pub(super) fn objective_set_display_auto_update(
        &mut self,
        objective: ObjectiveRef,
        auto: bool,
    ) -> bool {
        let Ok(name) = self.objective_for_write(objective) else {
            return false;
        };
        if let Some(o) = self.score.board.objective_mut(&name) {
            o.display_auto_update = auto;
        }
        true
    }

    /// `ScoreAccess.set`.
    pub(super) fn objective_set_score(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        value: i32,
    ) -> Result<(), ObjectiveError> {
        let (name, key, display) = self.score_write(objective, &holder)?;
        self.score
            .board
            .set_with(&key, &name, value, display.as_ref());
        Ok(())
    }

    /// `ScoreAccess.add`.
    pub(super) fn objective_add_score(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        delta: i32,
    ) -> Result<i32, ObjectiveError> {
        let (name, key, display) = self.score_write(objective, &holder)?;
        let board = &mut self.score.board;
        let value = board.get(&key, &name).wrapping_add(delta);
        board.set_with(&key, &name, value, display.as_ref());
        Ok(value)
    }

    /// `Scoreboard.resetSinglePlayerScore`.
    pub(super) fn objective_reset_score(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
    ) -> bool {
        let Ok(name) = self.objective_for_write(objective) else {
            return false;
        };
        let Some(key) = self.score_holder_key(&holder) else {
            return false;
        };
        if self.score.board.score(&key, &name).is_none() {
            return false;
        }
        self.score.board.reset_one(&key, &name);
        true
    }

    /// `ScoreAccess.lock` and `unlock`.
    pub(super) fn objective_set_score_locked(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        locked: bool,
    ) -> bool {
        let Ok(name) = self.objective_for_write(objective) else {
            return false;
        };
        let Some(key) = self.score_holder_key(&holder) else {
            return false;
        };
        let Some(score) = self
            .score
            .board
            .scores
            .get_mut(&key)
            .and_then(|scores| scores.get_mut(&name))
        else {
            return false;
        };
        score.locked = locked;
        self.sync_score_registries();
        true
    }

    /// `Scoreboard.removeObjective`.
    pub(super) fn objective_remove(&mut self, objective: ObjectiveRef) -> bool {
        let Ok(name) = self.objective_for_write(objective) else {
            return false;
        };
        self.score.board.remove_objective(&name);
        self.sync_score_registries();
        true
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::{lan::LanSettings, native_entity_refs::tests::with_game};
    use super::*;
    use mod_api::{TeamColor, prelude::ScoreboardServerHost, text::TextStyle};
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use sunrise_protocol::packets::configuration::ClientInformation;

    /// The API's slot and color names are the board's, in the board's order.
    #[test]
    fn slot_and_color_names_are_the_boards() {
        let board_slots = sb::display_slots();
        assert!(
            DisplaySlot::all()
                .map(DisplaySlot::name)
                .eq(board_slots.iter().map(String::as_str))
        );
        let colors: Vec<_> = TeamColor::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(colors, sb::team_colors().collect::<Vec<_>>());
        for slot in DisplaySlot::all() {
            assert_eq!(DisplaySlot::from_name(slot.name()), Some(slot));
        }
        assert_eq!(DisplaySlot::from_name("sidebar.team.pink"), None);
    }

    /// A number format crosses to the board and back unchanged.
    #[test]
    fn number_formats_round_trip_through_the_board() {
        let mut styled = TextStyle::default();
        styled.color = Some("red".into());
        styled.bold = Some(true);
        for format in [
            NumberFormat::Blank,
            NumberFormat::Fixed(mod_api::Text::literal("lots")),
            NumberFormat::Styled(styled),
        ] {
            let engine = engine_number_format(&format).unwrap();
            assert_eq!(api_number_format(&engine), Ok(format));
        }
        let mut red = TextStyle::default();
        red.color = Some("red".into());
        let engine = engine_number_format(&NumberFormat::Styled(red)).unwrap();
        assert_eq!(
            engine,
            sb::NumberFormat::Styled(serde_json::json!({"color": "red"}))
        );
    }

    /// A component deeper than the serialization limit survives in-memory capture, while
    /// writing it back across the serialization boundary is refused by name, never flattened.
    #[test]
    fn a_text_too_deep_for_the_api_is_refused_by_name() {
        let mut deep = literal("x");
        for _ in 0..600 {
            deep = serde_json::json!({"text": "", "extra": [deep]});
        }
        let captured = api_text(&comp_text(&deep)).unwrap();
        assert_eq!(engine_text(&captured), Err(TextFormError::TooDeep));
        let fixed = api_number_format(&sb::NumberFormat::Fixed(deep)).unwrap();
        assert_eq!(engine_number_format(&fixed), Err(TextFormError::TooDeep));
        assert_eq!(
            api_text(
                &EngineText::empty()
                    .with_component(serde_json::json!({"text": "x", "test:meta": [null]}))
            ),
            Err(TextFormError::NestedNull)
        );
        assert!(api_text(&comp_text(&literal("x"))).is_ok());
    }

    /// Opens the game to LAN guests with no saved board on disk (the test world keeps its
    /// folder between runs); returns the scoreboard file's path.
    pub(in crate::game) fn open_lan(game: &mut Game<'_>) -> std::path::PathBuf {
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 4,
            simulation_distance: 4,
            dimension: "minecraft:overworld".into(),
            allow_flight: true,
            seed: 0,
            max_players: 10,
            integrated: true,
            levels: game.level_shapes.clone(),
            level_seeds: game.levels.runtime.seeds(),
        };
        let mut net = NetConfig::lan(0, "test".into(), 10);
        net.listen = false;
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        game.lan_host(settings, net, config).unwrap();
        // The test world keeps its folder between runs: start from no saved board.
        let saved_board = game.world_root.join("data/minecraft/scoreboard.dat");
        let _ = std::fs::remove_file(&saved_board);
        saved_board
    }

    pub(in crate::game) fn guest(uuid: u128, conn: u32) -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }

    #[test]
    #[ignore = "daily"]
    fn objectives_are_written_synced_saved_and_stale_once_removed() {
        with_game(|game| {
            game.world_root = game.save.root.clone();
            let saved_board = open_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(123, 1));
            let player = game.player(123).unwrap();
            let text = mod_api::Text::literal;
            assert_eq!(
                ScoreboardServerHost::visit_objectives(game, &mut |_| panic!("empty board")),
                Ok(ControlFlow::Continue(()))
            );

            // Adding refuses what the board refuses, and nothing else.
            let kills = ScoreboardServerHost::add_objective(
                game,
                "kills",
                "dummy",
                text("Kills"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            assert_eq!(
                ScoreboardServerHost::add_objective(
                    game,
                    "kills",
                    "dummy",
                    text("x"),
                    ObjectiveRenderType::Integer
                ),
                Err(ObjectiveError::NameTaken)
            );
            assert_eq!(
                ScoreboardServerHost::add_objective(
                    game,
                    "n",
                    "nonsense",
                    text("x"),
                    ObjectiveRenderType::Integer
                ),
                Err(ObjectiveError::UnknownCriteria)
            );
            // Any string is a name: empty, wildcard and long ones included.
            for odd in ["", "*", &"x".repeat(100)] {
                assert!(
                    ScoreboardServerHost::add_objective(
                        game,
                        odd,
                        "dummy",
                        text("x"),
                        ObjectiveRenderType::Integer
                    )
                    .is_ok()
                );
            }
            assert_eq!(game.objective("kills"), Some(kills));
            let mut first = Vec::new();
            assert_eq!(
                ScoreboardServerHost::visit_objectives(game, &mut |objective| {
                    first.push(objective);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [kills]);
            assert_eq!(kills.name(game), Some("kills"));
            assert_eq!(kills.criteria(game), Some("dummy"));
            assert_eq!(kills.display_name(game), Ok(text("Kills")));
            assert_eq!(
                kills.visit_slots(game, &mut |_| panic!("no display slot")),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(
                kills.visit_scores(game, &mut |_| panic!("no score")),
                Ok(ControlFlow::Continue(()))
            );

            // A player and that player's entity are one holder, keyed by the name.
            let as_player = ScoreHolder::Player(player);
            let as_entity = ScoreHolder::Entity(player.entity());
            kills.set_score(game, as_player.clone(), 3).unwrap();
            assert_eq!(kills.add_score(game, as_entity.clone(), 2), Ok(5));
            assert_eq!(kills.score(game, as_player), Some(5));
            assert_eq!(
                kills.score(game, ScoreHolder::Name("guest1".into())),
                Some(5)
            );
            assert_eq!(game.score.board.score("guest1", "kills").unwrap().value, 5);
            kills
                .set_score(game, ScoreHolder::Name("#zero".into()), 0)
                .unwrap();
            assert_eq!(
                kills.score(game, ScoreHolder::Name("#zero".into())),
                Some(0)
            );
            let mut scores = Vec::new();
            assert_eq!(
                kills.visit_scores(game, &mut |row| {
                    scores.push((row.holder.to_owned(), row.value));
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            // The fastutil board walks bucket 26 (#zero) before bucket 14 (guest1).
            assert_eq!(
                scores,
                [(String::from("#zero"), 0), (String::from("guest1"), 5)]
            );
            assert_eq!(
                kills.visit_scores(game, &mut |row| {
                    assert_eq!(row, ScoreEntry::new("#zero", 0));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            // The command reads what the mod wrote, and the lock starts set.
            assert_eq!(kills.score_locked(game, as_entity.clone()), Some(true));
            assert!(kills.set_score_locked(game, as_entity, false));

            // Read-only criteria refuse a write, as the command does.
            let hp = ScoreboardServerHost::add_objective(
                game,
                "hp",
                "health",
                text("HP"),
                ObjectiveRenderType::Hearts,
            )
            .unwrap();
            assert_eq!(
                hp.set_score(game, ScoreHolder::Name("x".into()), 1),
                Err(ObjectiveError::ReadOnlyCriteria)
            );
            // A holder that is not on the server is not a stale handle.
            let offline = ScoreHolder::Entity(mod_api::EntityRef::new(
                game.native_lifecycle.epoch().unwrap(),
                9_999_999,
            ));
            assert_eq!(
                kills.set_score(game, offline.clone(), 1),
                Err(ObjectiveError::UnknownHolder)
            );
            assert_eq!(
                kills.add_score(game, offline.clone(), 1),
                Err(ObjectiveError::UnknownHolder)
            );
            assert_eq!(kills.score(game, offline), None);

            // Shown in the sidebar, a guest's view has it with its score.
            let red_sidebar = DisplaySlot::TeamSidebar(TeamColor::Red);
            assert!(game.set_display_slot(red_sidebar, Some(kills)));
            assert!(game.set_display_slot(DisplaySlot::Sidebar, Some(kills)));
            assert_eq!(
                game.display_slot_objective(DisplaySlot::Sidebar),
                Some(kills)
            );
            let mut slots = Vec::new();
            assert_eq!(
                kills.visit_slots(game, &mut |slot| {
                    slots.push(slot);
                    ControlFlow::Continue(())
                }),
                Ok(ControlFlow::Continue(()))
            );
            assert_eq!(slots, [DisplaySlot::Sidebar, red_sidebar]);
            game.lan_sync_scoreboard();
            let view = sunrise_client::lan::scoreboard::View::of(&game.score.board);
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].scoreboard, view);

            // Saved, the file holds the objective, its score and its slot, as a command's would.
            game.save_scoreboard();
            let saved = sunrise_client::save::read_dat(&saved_board).unwrap();
            let loaded = sb::Scoreboard::unpack(saved.get("data").unwrap());
            assert_eq!(loaded.score("guest1", "kills").unwrap().value, 5);
            assert_eq!(
                loaded.display.get("sidebar").map(String::as_str),
                Some("kills")
            );

            // Reopened, the saved objective is there for a mod reading at startup, before any
            // write or tick; a handle from before the open is stale.
            game.native_lifecycle_open_started();
            game.score = Default::default();
            game.native_lifecycle_world_ready();
            let kills_again = game.objective("kills").unwrap();
            assert_ne!(kills_again, kills);
            assert_eq!(kills.name(game), None);
            assert_eq!(
                kills_again.score(game, ScoreHolder::Name("guest1".into())),
                Some(5)
            );
            assert_eq!(
                game.display_slot_objective(DisplaySlot::Sidebar),
                Some(kills_again)
            );

            // Removed, and added again under its name: the old handle stays stale.
            assert!(kills_again.remove(game));
            let renamed = ScoreboardServerHost::add_objective(
                game,
                "kills",
                "dummy",
                text("Again"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            assert_ne!(renamed, kills_again);
            assert_eq!(kills_again.name(game), None);
            assert_eq!(kills_again.display_name(game), Err(ObjectiveError::Stale));
            assert_eq!(
                kills_again.visit_scores(game, &mut |_| panic!("stale objective")),
                Err(ObjectiveReadError::Stale)
            );
            assert_eq!(
                kills_again.visit_slots(game, &mut |_| panic!("stale objective")),
                Err(ObjectiveReadError::Stale)
            );
            assert_eq!(
                kills_again.set_score(game, ScoreHolder::Name("a".into()), 1),
                Err(ObjectiveError::Stale)
            );
            assert!(!kills_again.remove(game));
            assert_eq!(renamed.score(game, ScoreHolder::Name("a".into())), None);
            // An aborted transaction puts the board back; a committed one keeps its writes.
            let before = game.score.board.clone();
            let aborted: Result<(), ()> = game.storage_transaction(|host, _| {
                let temp = ScoreboardServerHost::add_objective(
                    host,
                    "temp",
                    "dummy",
                    text("T"),
                    ObjectiveRenderType::Integer,
                )
                .map_err(|_| ())?;
                temp.set_score(host, ScoreHolder::Name("p".into()), 4)
                    .map_err(|_| ())?;
                assert!(renamed.remove(host));
                Err(())
            });
            assert!(aborted.is_err());
            assert_eq!(game.score.board, before);
            assert!(game.objective("temp").is_none());
            assert_eq!(renamed.name(game), Some("kills"));
            let kept: Result<(), ()> = game.storage_transaction(|host, _| {
                ScoreboardServerHost::add_objective(
                    host,
                    "temp",
                    "dummy",
                    text("T"),
                    ObjectiveRenderType::Integer,
                )
                .map(|_| ())
                .map_err(|_| ())
            });
            assert!(kept.is_ok());
            assert!(game.objective("temp").is_some());

            // The command's removal makes a handle stale the same way.
            game.ui.registries.permission = 4;
            assert!(game.run_command("scoreboard objectives remove kills"));
            assert_eq!(renamed.name(game), None);
            let _ = std::fs::remove_file(&saved_board);
        });
    }
}
