//! `TestEnvironmentDefinition`: what a game test batch changes in the world
//! while it runs (game rules, weather, the day time, difficulty, functions,
//! all of them), set up when the batch starts and torn down after it. The
//! definitions are the data pack's `test_environment` entries, or inline.
use super::Game;
use serde_json::Value as Json;
use sunrise_client_core::gamerules::Value;
pub(super) use sunrise_gameplay::test_environments::{
    Activation, Environment, Saved, WeatherType, parse,
};

impl Game<'_> {
    /// The data pack entry `data/<ns>/test_environment/<path>.json`.
    fn read_test_environment_json(&self, id: &str) -> Option<Json> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = self
            .options
            .resources
            .join("data")
            .join(ns)
            .join("test_environment")
            .join(format!("{path}.json"));
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    }

    /// The definition of a batch's environment (`batch()` of its tests).
    pub(super) fn test_environment(&self, id: &str) -> Option<Environment> {
        parse(
            &Json::String(id.to_owned()),
            &|i| self.read_test_environment_json(i),
            0,
            &self.ui.rules,
        )
    }

    /// `TestEnvironmentDefinition.activate`.
    pub(super) fn activate_test_environment(&mut self, environment: Environment) -> Activation {
        let saved = self.set_up_environment(&environment);
        Activation { environment, saved }
    }

    /// `Activation.teardown`.
    pub(super) fn tear_down_test_environment(&mut self, activation: Activation) {
        self.tear_down_environment(&activation.environment, activation.saved);
    }

    fn set_up_environment(&mut self, environment: &Environment) -> Saved {
        match environment {
            Environment::AllOf(all) => Saved::AllOf(
                all.iter()
                    .map(|e| (e.clone(), self.set_up_environment(e)))
                    .collect(),
            ),
            Environment::ClockTime { clock, time } => {
                // Only the overworld clock runs in Sunrise.
                let previous = self.world.day_time;
                if clock == "minecraft:overworld" {
                    self.world.day_time = *time;
                    self.ui.clock.partial = 0.0;
                }
                Saved::Clock(previous)
            }
            Environment::Difficulty(d) => {
                let previous = self.ui.difficulty;
                self.set_difficulty_forced(*d);
                Saved::Difficulty(previous)
            }
            Environment::Functions { setup, .. } => {
                if let Some(id) = setup {
                    self.run_environment_function(id);
                }
                Saved::Unit
            }
            Environment::GameRules(rules) => {
                let original = rules
                    .iter()
                    .filter_map(|(name, _)| {
                        let rule = self.ui.rules.rule(name)?;
                        Some((
                            rule.name.to_owned(),
                            self.ui.rules.get(&rule.name).unwrap_or(rule.default),
                        ))
                    })
                    .collect();
                self.set_environment_rules(rules);
                Saved::GameRules(original)
            }
            Environment::Timelines => Saved::Unit,
            Environment::Weather(kind) => {
                let w = &self.world.weather;
                let previous = if w.thundering {
                    WeatherType::Thunder
                } else if w.raining {
                    WeatherType::Rain
                } else {
                    WeatherType::Clear
                };
                self.set_weather_type(*kind);
                Saved::Weather(previous)
            }
        }
    }

    fn tear_down_environment(&mut self, environment: &Environment, saved: Saved) {
        match (environment, saved) {
            (Environment::AllOf(_), Saved::AllOf(all)) => {
                for (e, s) in all.into_iter().rev() {
                    self.tear_down_environment(&e, s);
                }
            }
            (Environment::ClockTime { clock, .. }, Saved::Clock(previous)) => {
                if clock == "minecraft:overworld" {
                    self.world.day_time = previous;
                    self.ui.clock.partial = 0.0;
                }
            }
            (_, Saved::Difficulty(d)) => self.set_difficulty_forced(d),
            (
                Environment::Functions {
                    teardown: Some(id), ..
                },
                Saved::Unit,
            ) => self.run_environment_function(id),
            (_, Saved::GameRules(original)) => self.set_environment_rules(&original),
            (_, Saved::Weather(previous)) => {
                // resetWeatherCycle, then the saved type.
                let w = &mut self.world.weather;
                w.rain_time = 0;
                w.raining = false;
                w.thunder_time = 0;
                w.thundering = false;
                self.set_weather_type(previous);
            }
            _ => {}
        }
    }

    /// `GameRules.setAll`.
    fn set_environment_rules(&mut self, rules: &[(String, Value)]) {
        for (name, value) in rules {
            if let Some(rule) = self.ui.rules.rule(name).cloned()
                && self.ui.rules.set(&rule.name, *value)
            {
                self.game_rule_changed(&rule.name, *value, None);
            }
        }
        self.sync_rules();
    }

    /// `Weather.Type.apply`: `ServerLevel.setWeatherParameters`.
    fn set_weather_type(&mut self, kind: WeatherType) {
        let (clear, rain, raining, thundering) = kind.parameters();
        let w = &mut self.world.weather;
        w.clear_time = clear;
        w.rain_time = rain;
        w.thunder_time = rain;
        w.raining = raining;
        w.thundering = thundering;
    }

    /// `Functions.run`: the function with game master permission and
    /// suppressed output; a missing one is logged.
    fn run_environment_function(&mut self, id: &str) {
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let found = self
            .functions_named(&id)
            .ok()
            .and_then(|f| f.into_iter().next());
        match found {
            Some(function) => {
                let source = crate::game::commands::execute::Source {
                    silent: true,
                    ..self.server_source()
                };
                self.exec.commands = 0;
                let _ = self.run_function(&function, None, source);
            }
            None => log::error!("Test Batch failed for non-existent function {id}"),
        }
    }
}
