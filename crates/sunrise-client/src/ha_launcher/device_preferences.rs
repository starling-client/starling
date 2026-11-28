//! Client-wide diagnostics preferences, independent of touch layouts: the game's process reads
//! them when it starts (`ha_launcher::game_started`).
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub gpu_profile: bool,
}

impl Settings {
    /// Upgrade the old touch-file preference only when the client has not
    /// saved the corresponding choice. Explicit Off wins.
    pub fn migrate_missing(&mut self, client: &Value, touch: &Value) {
        if client.get("gpu_profile").is_none() {
            self.gpu_profile = touch["gpu_profile"].as_bool().unwrap_or(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_touch_preferences_migrate_without_overriding_explicit_off() {
        let old = json!({"gpu_profile":true});
        let mut settings = Settings::default();
        settings.migrate_missing(&json!({}), &old);
        assert!(settings.gpu_profile);
        let saved = json!({"gpu_profile":false});
        let mut settings: Settings = serde_json::from_value(saved.clone()).unwrap();
        settings.migrate_missing(&saved, &old);
        assert!(!settings.gpu_profile);
    }

    #[test]
    fn corrupt_old_values_use_safe_defaults() {
        for value in [json!(1), json!("true"), Value::Null] {
            let mut settings = Settings::default();
            settings.migrate_missing(&json!({}), &json!({"gpu_profile":value}));
            assert!(!settings.gpu_profile);
        }
    }
}
