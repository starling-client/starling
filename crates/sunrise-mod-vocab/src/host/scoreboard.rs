//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `scoreboard` area, implemented by the game and reached through [`ServerHost`].
///
/// Every write is the one `/scoreboard` makes on the server's board, so clients, saves and selectors
/// cannot tell a mod's write from the command. Inside `Host::transaction` the writes apply at once
/// and an abort puts the board back as it was when the transaction first wrote it.
#[sunrise_mod_api_macros::host_area]
pub trait ScoreboardServerHost {
    /// Side: server.
    /// `Scoreboard.getObjective`: the objective with that name, or None. The name is any
    /// string; saved objectives are there from the world's start.
    fn objective(&self, name: &str) -> Option<ObjectiveRef>;
    /// Side: server.
    /// `Scoreboard.getObjectives`: every objective. Vanilla keeps them in a hash map, so no
    /// order is promised; Sunrise answers its board's own.
    fn objectives(&self) -> Vec<ObjectiveRef>;
    /// Side: server.
    /// `Scoreboard.addObjective`: a new objective with no number format that does not update
    /// scores' display names, shown nowhere. The name is any string, as the board takes it;
    /// the `/scoreboard` command only reads a single word. Refused with
    /// [`ObjectiveError::NameTaken`] or [`ObjectiveError::UnknownCriteria`].
    ///
    /// `display_name` is bounded as every `Text` is: one deeper than 512 component nodes is
    /// refused with [`ObjectiveError::Text`] before any change.
    #[serialize_text]
    fn add_objective(
        &mut self,
        name: &str,
        criteria: &str,
        display_name: Text,
        render_type: ObjectiveRenderType,
    ) -> Result<ObjectiveRef, ObjectiveError>;
    /// Side: server.
    /// `Scoreboard.getDisplayObjective`: the objective shown in the slot, or None.
    fn display_slot_objective(&self, slot: DisplaySlot) -> Option<ObjectiveRef>;
    /// Side: server.
    /// `Scoreboard.setDisplayObjective`: shows the objective in the slot, or clears the slot
    /// with None. Clients are sent the change by the board's usual per-tick sync. False for a
    /// stale objective.
    fn set_display_slot(&mut self, slot: DisplaySlot, objective: Option<ObjectiveRef>) -> bool;
    /// Side: server.
    /// `Scoreboard.getPlayerTeam`: the team with that name, or None.
    fn team(&self, name: &str) -> Option<TeamRef>;
    /// Side: server.
    /// `Scoreboard.getPlayerTeams`: every team, in the board's own order (vanilla's is a hash
    /// map's, so no order is promised).
    fn teams(&self) -> Vec<TeamRef>;
    /// Side: server.
    /// `/team add`: a new team with its name as display name, no color, no members and
    /// vanilla's default options. The name is any string, as the board takes it. Refused with
    /// [`TeamError::NameTaken`] when a team has it.
    fn add_team(&mut self, name: &str) -> Result<TeamRef, TeamError>;
    /// Side: server.
    /// `Scoreboard.getPlayersTeam`: the team the holder is on, or None when it is on none or
    /// does not resolve. Membership is by the holder's key, so an offline player's name works.
    fn holder_team(&self, holder: ScoreHolder) -> Option<TeamRef>;
}
/// The handles' reads of the `scoreboard` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ScoreboardBackend {
    /// Side: server.
    /// `Objective.getName`, or None for a stale handle.
    fn objective_name(&self, objective: ObjectiveRef) -> Option<String>;
    /// Side: server.
    /// `Objective.getCriteria`: its name (`dummy`, `trigger`, `deathCount`, `minecraft.mined:minecraft.stone`...).
    fn objective_criteria(&self, objective: ObjectiveRef) -> Option<String>;
    /// Side: server.
    /// `Objective.getDisplayName`.
    fn objective_display_name(&self, objective: ObjectiveRef) -> Result<Text, ObjectiveError>;
    /// Side: server.
    /// `Objective.getRenderType`.
    fn objective_render_type(&self, objective: ObjectiveRef) -> Option<ObjectiveRenderType>;
    /// Side: server.
    /// `Objective.numberFormat`: None inside the Ok when the objective has none, so the slot's
    /// default shows. A fixed text the API cannot hold is [`ObjectiveError::Text`].
    fn objective_number_format(
        &self,
        objective: ObjectiveRef,
    ) -> Result<Option<NumberFormat>, ObjectiveError>;
    /// Side: server.
    /// `Objective.displayAutoUpdate`: whether setting a score copies the holder's display name
    /// into the score.
    fn objective_display_auto_update(&self, objective: ObjectiveRef) -> Option<bool>;
    /// Side: server.
    /// The display slots that show the objective, in id order.
    fn objective_slots(&self, objective: ObjectiveRef) -> Option<Vec<DisplaySlot>>;
    /// Side: server.
    /// `Scoreboard.getPlayerScoreInfo`: the holder's score, or None when it has none, a holder
    /// that does not resolve (an offline player's entity) or a stale handle. It never creates a score.
    fn objective_score(&self, objective: ObjectiveRef, holder: ScoreHolder) -> Option<i32>;
    /// Side: server.
    /// `Scoreboard.listPlayerScores`: every holder's score. Vanilla's order is a hash map's;
    /// the docs promise none, and Sunrise answers the board's own. None for a stale handle.
    fn objective_scores(&self, objective: ObjectiveRef) -> Option<Vec<ScoreEntry>>;
    /// Side: server.
    /// `ScoreAccess.locked`, the lock `/trigger` uses: a score created at runtime starts locked,
    /// one loaded from a save without the flag, unlocked. None when there is no score.
    fn objective_score_locked(&self, objective: ObjectiveRef, holder: ScoreHolder) -> Option<bool>;
    /// Side: server.
    /// `PlayerTeam.getName`, or None for a stale handle.
    fn team_name(&self, team: TeamRef) -> Option<String>;
    /// Side: server.
    /// `PlayerTeam.getDisplayName`.
    fn team_display_name(&self, team: TeamRef) -> Result<Text, TeamError>;
    /// Side: server.
    /// `PlayerTeam.getColor`: None inside the Ok when the team has none.
    fn team_color(&self, team: TeamRef) -> Result<Option<TeamColor>, TeamError>;
    /// Side: server.
    /// `PlayerTeam.getPlayerPrefix`.
    fn team_prefix(&self, team: TeamRef) -> Result<Text, TeamError>;
    /// Side: server.
    /// `PlayerTeam.getPlayerSuffix`.
    fn team_suffix(&self, team: TeamRef) -> Result<Text, TeamError>;
    /// Side: server.
    /// `PlayerTeam.isAllowFriendlyFire`.
    fn team_friendly_fire(&self, team: TeamRef) -> Option<bool>;
    /// Side: server.
    /// `PlayerTeam.canSeeFriendlyInvisibles`.
    fn team_see_friendly_invisibles(&self, team: TeamRef) -> Option<bool>;
    /// Side: server.
    /// `PlayerTeam.getNameTagVisibility`.
    fn team_name_tag_visibility(&self, team: TeamRef) -> Option<TeamVisibility>;
    /// Side: server.
    /// `PlayerTeam.getDeathMessageVisibility`.
    fn team_death_message_visibility(&self, team: TeamRef) -> Option<TeamVisibility>;
    /// Side: server.
    /// `PlayerTeam.getCollisionRule`.
    fn team_collision_rule(&self, team: TeamRef) -> Option<CollisionRule>;
    /// Side: server.
    /// Visit `PlayerTeam.getPlayers` in the team's native Java hash set order, which
    /// vanilla leaves unspecified. The visitor gets each holder key as a borrowed
    /// `&str`, valid only during that callback, including names that do not resolve
    /// to an online player. No result list is built and no names are copied.
    ///
    /// The visitor can make shared reads on the same host. Return `ControlFlow::Continue(())`
    /// to keep visiting or `ControlFlow::Break(())` to stop. The visit returns
    /// `Ok(ControlFlow::Break(()))` if stopped or `Ok(ControlFlow::Continue(()))` on
    /// completion, including an empty team. [`TeamMembersReadError::Stale`] and
    /// [`TeamMembersReadError::Unavailable`] are refusals: neither calls the visitor.
    ///
    /// To retain a name, make an owned copy in the visitor. For a scoreboard write,
    /// collect the names you need, then use `ScoreHolder::Name(name)` with the mutable
    /// host after the visit.
    fn team_visit_members(
        &self,
        team: TeamRef,
        visit: &mut dyn FnMut(&str) -> core::ops::ControlFlow<()>,
    ) -> Result<core::ops::ControlFlow<()>, TeamMembersReadError>;
}
/// The handles' writes of the `scoreboard` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait ScoreboardBackendMut {
    /// Side: server.
    /// `Objective.setDisplayName`: viewers are sent the new name.
    ///
    /// `name` is bounded as every `Text` is: one deeper than 512 component nodes is refused with
    /// [`ObjectiveError::Text`] before any change.
    #[serialize_text]
    fn objective_set_display_name(
        &mut self,
        objective: ObjectiveRef,
        name: Text,
    ) -> Result<(), ObjectiveError>;
    /// Side: server.
    /// `Objective.setRenderType`. False for a stale handle.
    fn objective_set_render_type(
        &mut self,
        objective: ObjectiveRef,
        render_type: ObjectiveRenderType,
    ) -> bool;
    /// Side: server.
    /// `Objective.setNumberFormat`: None goes back to the slot's default.
    fn objective_set_number_format(
        &mut self,
        objective: ObjectiveRef,
        format: Option<NumberFormat>,
    ) -> Result<(), ObjectiveError>;
    /// Side: server.
    /// `Objective.setDisplayAutoUpdate`. False for a stale handle.
    fn objective_set_display_auto_update(&mut self, objective: ObjectiveRef, auto: bool) -> bool;
    /// Side: server.
    /// `ScoreAccess.set`, creating the score at 0 first (locked). Clients are told only when
    /// something changed. Refused with [`ObjectiveError::Stale`] for a stale handle,
    /// [`ObjectiveError::ReadOnlyCriteria`] for a read-only criteria, and
    /// [`ObjectiveError::UnknownHolder`] for a holder that does not resolve.
    fn objective_set_score(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        value: i32,
    ) -> Result<(), ObjectiveError>;
    /// Side: server.
    /// `ScoreAccess.add`: the new score, wrapping on overflow, with the refusals of `set_score`.
    fn objective_add_score(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        delta: i32,
    ) -> Result<i32, ObjectiveError>;
    /// Side: server.
    /// `Scoreboard.resetSinglePlayerScore`: removes the holder's score. False when it had none.
    fn objective_reset_score(&mut self, objective: ObjectiveRef, holder: ScoreHolder) -> bool;
    /// Side: server.
    /// `ScoreAccess.lock` and `unlock`. Clients are never sent the lock. False when the holder has no score.
    fn objective_set_score_locked(
        &mut self,
        objective: ObjectiveRef,
        holder: ScoreHolder,
        locked: bool,
    ) -> bool;
    /// Side: server.
    /// `Scoreboard.removeObjective`: its display slots clear, its scores go, and the handle (and
    /// any other for it) is stale for good, even if the name is added again.
    fn objective_remove(&mut self, objective: ObjectiveRef) -> bool;
    /// Side: server.
    /// `PlayerTeam.setDisplayName`: members' clients are sent the change.
    ///
    /// `name` is bounded as every `Text` is: one deeper than 512 component nodes is refused with
    /// [`TeamError::Text`] before any change.
    #[serialize_text]
    fn team_set_display_name(&mut self, team: TeamRef, name: Text) -> Result<(), TeamError>;
    /// Side: server.
    /// `PlayerTeam.setColor`: None clears it. False for a stale handle.
    fn team_set_color(&mut self, team: TeamRef, color: Option<TeamColor>) -> bool;
    /// Side: server.
    /// `PlayerTeam.setPlayerPrefix`, with the bound on `Text` of `set_display_name`.
    #[serialize_text]
    fn team_set_prefix(&mut self, team: TeamRef, prefix: Text) -> Result<(), TeamError>;
    /// Side: server.
    /// `PlayerTeam.setPlayerSuffix`, with the bound on `Text` of `set_display_name`.
    #[serialize_text]
    fn team_set_suffix(&mut self, team: TeamRef, suffix: Text) -> Result<(), TeamError>;
    /// Side: server.
    /// `PlayerTeam.setAllowFriendlyFire`. False for a stale handle.
    fn team_set_friendly_fire(&mut self, team: TeamRef, allowed: bool) -> bool;
    /// Side: server.
    /// `PlayerTeam.setSeeFriendlyInvisibles`. False for a stale handle.
    fn team_set_see_friendly_invisibles(&mut self, team: TeamRef, allowed: bool) -> bool;
    /// Side: server.
    /// `PlayerTeam.setNameTagVisibility`. False for a stale handle.
    fn team_set_name_tag_visibility(&mut self, team: TeamRef, visibility: TeamVisibility) -> bool;
    /// Side: server.
    /// `PlayerTeam.setDeathMessageVisibility`. False for a stale handle.
    fn team_set_death_message_visibility(
        &mut self,
        team: TeamRef,
        visibility: TeamVisibility,
    ) -> bool;
    /// Side: server.
    /// `PlayerTeam.setCollisionRule`. False for a stale handle.
    fn team_set_collision_rule(&mut self, team: TeamRef, rule: CollisionRule) -> bool;
    /// Side: server.
    /// `Scoreboard.addPlayerToTeam`: the holder leaves the team it is on, if any (this one
    /// included), then joins this one, so it is on exactly one team. Returns whether the
    /// team's member set changed. Refused with [`TeamError::Stale`] for a stale handle and
    /// [`TeamError::UnknownHolder`] for a holder that does not resolve.
    fn team_join(&mut self, team: TeamRef, holder: ScoreHolder) -> Result<bool, TeamError>;
    /// Side: server.
    /// `Scoreboard.removePlayerFromTeam(holder, team)`: false when the holder is not on this
    /// team (vanilla throws), the handle is stale or the holder does not resolve.
    fn team_leave(&mut self, team: TeamRef, holder: ScoreHolder) -> bool;
    /// Side: server.
    /// `Scoreboard.removePlayerTeam`: its members are on no team and the handle (and any other
    /// for it) is stale for good, even if the name is added again.
    fn team_remove(&mut self, team: TeamRef) -> bool;
}
/// The client host's methods of the `scoreboard` area, implemented by the game and reached through [`ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ScoreboardClientHost {
    /// Side: client.
    /// What this client has been sent of the scoreboard: the objectives shown in a display
    /// slot, with their scores, and every team. None outside a logged-in client world. A text the API cannot hold
    /// (nested deeper than 512 component nodes) refuses the whole snapshot with
    /// [`TextFormError::TooDeep`]; nothing is flattened or skipped.
    fn client_scoreboard(&self) -> Result<Option<ClientScoreboard>, TextFormError>;
}
