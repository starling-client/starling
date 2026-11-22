//! `TestEnvironmentDefinition`: what a game test batch changes in the world
//! while it runs (game rules, weather, the day time, difficulty, functions,
//! all of them), set up when the batch starts and torn down after it. The
//! definitions are the data pack's `test_environment` entries, or inline.
use serde_json::Value as Json;
use sunrise_client_core::gamerules::{GameRules, Value};

/// `Weather.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherType {
    Clear,
    Rain,
    Thunder,
}

impl WeatherType {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "clear" => Some(Self::Clear),
            "rain" => Some(Self::Rain),
            "thunder" => Some(Self::Thunder),
            _ => None,
        }
    }

    /// (clearTime, rainTime, raining, thundering).
    pub fn parameters(self) -> (i32, i32, bool, bool) {
        match self {
            Self::Clear => (100_000, 0, false, false),
            Self::Rain => (0, 100_000, true, false),
            Self::Thunder => (0, 100_000, true, true),
        }
    }
}

/// A `TestEnvironmentDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub enum Environment {
    AllOf(Vec<Environment>),
    ClockTime {
        clock: String,
        time: i64,
    },
    Difficulty(u8),
    Functions {
        setup: Option<String>,
        teardown: Option<String>,
    },
    GameRules(Vec<(String, Value)>),
    /// `timeline_attributes`: Sunrise has no environment attribute system to
    /// layer timelines on, so it changes nothing.
    Timelines,
    Weather(WeatherType),
}

/// The `SavedDataType` of an activated definition.
#[derive(Clone, Debug, PartialEq)]
pub enum Saved {
    AllOf(Vec<(Environment, Saved)>),
    Clock(i64),
    Difficulty(u8),
    Unit,
    GameRules(Vec<(String, Value)>),
    Weather(WeatherType),
}

/// `TestEnvironmentDefinition.Activation`.
#[derive(Clone, Debug)]
pub struct Activation {
    pub environment: Environment,
    pub saved: Saved,
}

/// Reads a definition from its JSON (`DIRECT_CODEC`); a string is the id of
/// a registry entry, read by `load`.
pub fn parse(
    json: &Json,
    load: &dyn Fn(&str) -> Option<Json>,
    depth: u32,
    rules: &GameRules,
) -> Option<Environment> {
    if depth > 16 {
        return None;
    }
    if let Some(id) = json.as_str() {
        return parse(&load(id)?, load, depth + 1, rules);
    }
    let kind = json.get("type")?.as_str()?;
    let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
    Some(match kind {
        "all_of" => Environment::AllOf(
            json.get("definitions")?
                .as_array()?
                .iter()
                .map(|d| parse(d, load, depth + 1, rules))
                .collect::<Option<_>>()?,
        ),
        "clock_time" => Environment::ClockTime {
            clock: json.get("clock")?.as_str()?.to_owned(),
            time: json.get("time")?.as_i64().filter(|t| *t >= 0)?,
        },
        "difficulty" => Environment::Difficulty(
            ["peaceful", "easy", "normal", "hard"]
                .iter()
                .position(|d| Some(*d) == json.get("difficulty").and_then(Json::as_str))?
                as u8,
        ),
        "function" => Environment::Functions {
            setup: json.get("setup").and_then(Json::as_str).map(str::to_owned),
            teardown: json
                .get("teardown")
                .and_then(Json::as_str)
                .map(str::to_owned),
        },
        "game_rules" => Environment::GameRules(
            json.get("rules")?
                .as_object()?
                .iter()
                .map(|(name, v)| {
                    let rule = rules.rule(name)?;
                    let value = match (v.as_bool(), v.as_i64()) {
                        (Some(b), _) => Value::Bool(b),
                        (_, Some(i)) => Value::Int(i32::try_from(i).ok()?),
                        _ => return None,
                    };
                    Some((rule.name.to_owned(), value))
                })
                .collect::<Option<_>>()?,
        ),
        "timeline_attributes" => Environment::Timelines,
        "weather" => Environment::Weather(WeatherType::from_name(json.get("weather")?.as_str()?)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn load(id: &str) -> Option<Json> {
        match id {
            "minecraft:night" => Some(
                json!({"type": "minecraft:clock_time", "clock": "minecraft:overworld", "time": 18000}),
            ),
            _ => None,
        }
    }

    #[test]
    fn definitions_parse_with_references_and_nesting() {
        let json = json!({
            "type": "minecraft:all_of",
            "definitions": [
                "minecraft:night",
                {"type": "minecraft:weather", "weather": "thunder"},
                {"type": "minecraft:game_rules", "rules": {"minecraft:advance_time": false}},
                {"type": "minecraft:difficulty", "difficulty": "hard"},
                {"type": "minecraft:function", "setup": "test:on"}
            ]
        });
        let Some(Environment::AllOf(all)) = parse(&json, &load, 0, &GameRules::default()) else {
            panic!("not all_of");
        };
        assert_eq!(
            all[0],
            Environment::ClockTime {
                clock: "minecraft:overworld".into(),
                time: 18000
            }
        );
        assert_eq!(all[1], Environment::Weather(WeatherType::Thunder));
        assert!(matches!(&all[2], Environment::GameRules(r) if r.len() == 1));
        assert_eq!(all[3], Environment::Difficulty(3));
        assert_eq!(
            all[4],
            Environment::Functions {
                setup: Some("test:on".into()),
                teardown: None
            }
        );
    }

    #[test]
    fn unknown_or_invalid_definitions_do_not_parse() {
        assert_eq!(
            parse(&json!("minecraft:missing"), &load, 0, &GameRules::default()),
            None
        );
        assert_eq!(
            parse(
                &json!({"type": "minecraft:nothing"}),
                &load,
                0,
                &GameRules::default()
            ),
            None
        );
        assert_eq!(
            parse(
                &json!({"type": "minecraft:clock_time", "clock": "c", "time": -1}),
                &load,
                0,
                &GameRules::default()
            ),
            None
        );
    }

    #[test]
    fn weather_types_set_vanillas_timers() {
        assert_eq!(WeatherType::Clear.parameters(), (100_000, 0, false, false));
        assert_eq!(WeatherType::Thunder.parameters(), (0, 100_000, true, true));
    }
}
