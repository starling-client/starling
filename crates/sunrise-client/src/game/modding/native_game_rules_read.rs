use super::Game;
use mod_api::GameRuleReadError;
use sunrise_client_core::gamerules::Value as GameRuleValue;

impl Game<'_> {
    pub(super) fn game_rule(&self, id: &str) -> Result<GameRuleValue, GameRuleReadError> {
        self.ui.rules.get(id).ok_or(GameRuleReadError::UnknownRule)
    }

    pub(super) fn game_rules(&self) -> Vec<(String, GameRuleValue)> {
        self.ui
            .rules
            .available_rules()
            .filter_map(|rule| Some((rule.name.clone(), self.ui.rules.get(&rule.name)?)))
            .collect()
    }
}
