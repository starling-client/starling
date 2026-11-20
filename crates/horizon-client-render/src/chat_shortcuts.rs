//! Touch-friendly customizable quick-command action bar for mobile chat.
//!
//! Provides a scrollable dock of quick action chips rendered above the chat input box
//! on mobile / touch devices.
//!
//! # Environment Awareness
//! In singleplayer / local survival worlds (`!remote`), this bar is hidden by default
//! to preserve vanilla purity, but can be enabled via `show_in_singleplayer`.
//! In multiplayer (`remote == true`), it is displayed with standard server defaults:
//! `[/login ]`, `[/register ]`, `[/home]`, `[/spawn]`, `[/money]`, `[/back]`.

use serde::{Deserialize, Serialize};

/// The execution mode when tapping a shortcut chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutTrigger {
    /// Inserts the command text (with trailing space if applicable) into the text field,
    /// placing the cursor at the end so the user can immediately type or paste arguments.
    Insert,
    /// Dispatches the command directly over the network without opening the system soft keyboard.
    QuickCast,
}

/// The action resulting from tapping a shortcut chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Command text was inserted into the text field.
    Inserted,
    /// Command should be immediately dispatched to the server.
    Execute(String),
}

/// A single shortcut action displayed as a chip in the quick action bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatShortcut {
    /// Label shown inside the chip.
    pub label: String,
    /// Command to execute or insert (e.g. "/login ", "/home").
    pub command: String,
    /// Trigger mode (Insert vs QuickCast).
    pub trigger: ShortcutTrigger,
}

impl ChatShortcut {
    pub const fn new(label: String, command: String, trigger: ShortcutTrigger) -> Self {
        Self {
            label,
            command,
            trigger,
        }
    }
}

/// Persistent configuration for the quick-action chat bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatShortcutsConfig {
    /// Global master toggle for the quick action bar.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to display the bar in singleplayer / local survival worlds.
    /// Defaults to false so local vanilla survival stays unencumbered.
    #[serde(default)]
    pub show_in_singleplayer: bool,
    /// Configured list of shortcuts in display order.
    #[serde(default = "default_shortcuts")]
    pub shortcuts: Vec<ChatShortcut>,
}

const fn default_true() -> bool {
    true
}

fn default_shortcuts() -> Vec<ChatShortcut> {
    vec![
        ChatShortcut {
            label: "login".into(),
            command: "/login ".into(),
            trigger: ShortcutTrigger::Insert,
        },
        ChatShortcut {
            label: "register".into(),
            command: "/register ".into(),
            trigger: ShortcutTrigger::Insert,
        },
        ChatShortcut {
            label: "home".into(),
            command: "/home".into(),
            trigger: ShortcutTrigger::QuickCast,
        },
        ChatShortcut {
            label: "spawn".into(),
            command: "/spawn".into(),
            trigger: ShortcutTrigger::QuickCast,
        },
        ChatShortcut {
            label: "money".into(),
            command: "/money".into(),
            trigger: ShortcutTrigger::QuickCast,
        },
        ChatShortcut {
            label: "back".into(),
            command: "/back".into(),
            trigger: ShortcutTrigger::QuickCast,
        },
    ]
}

impl Default for ChatShortcutsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_in_singleplayer: false,
            shortcuts: default_shortcuts(),
        }
    }
}

impl ChatShortcutsConfig {
    /// Evaluates whether the bar should be rendered for the active game session.
    #[inline]
    pub fn should_display(&self, is_remote_server: bool) -> bool {
        self.enabled && (is_remote_server || self.show_in_singleplayer)
    }

    /// Appends a new custom shortcut.
    pub fn add_shortcut(
        &mut self,
        label: impl Into<String>,
        command: impl Into<String>,
        trigger: ShortcutTrigger,
    ) {
        self.shortcuts.push(ChatShortcut {
            label: label.into(),
            command: command.into(),
            trigger,
        });
    }

    /// Removes a shortcut by index.
    pub fn remove_shortcut(&mut self, index: usize) -> Option<ChatShortcut> {
        if index < self.shortcuts.len() {
            Some(self.shortcuts.remove(index))
        } else {
            None
        }
    }
}

/// Hitbox bounds for a rendered chip on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ChipBounds {
    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }
}

/// Layout calculator for chips positioned horizontally above the chat input box.
#[derive(Clone, Debug)]
pub struct ChatShortcutsLayout {
    pub bounds: Vec<(ChipBounds, usize)>,
    pub total_width: f32,
    pub bar_y: f32,
    pub bar_height: f32,
}

impl ChatShortcutsLayout {
    pub const CHIP_HEIGHT: f32 = 18.0;
    pub const CHIP_PADDING_X: f32 = 6.0;
    pub const CHIP_GAP: f32 = 4.0;
    pub const CHAR_WIDTH_APPROX: f32 = 6.0;

    /// Computes the layout given the chat input's top Y coordinate, available width,
    /// and current horizontal scroll offset.
    pub fn compute(
        config: &ChatShortcutsConfig,
        is_remote: bool,
        input_top_y: f32,
        scroll_x: f32,
    ) -> Option<Self> {
        if !config.should_display(is_remote) || config.shortcuts.is_empty() {
            return None;
        }

        let bar_height = Self::CHIP_HEIGHT + 4.0;
        let bar_y = input_top_y - bar_height;
        let mut cur_x = 4.0 - scroll_x;
        let mut bounds = Vec::with_capacity(config.shortcuts.len());

        for (idx, shortcut) in config.shortcuts.iter().enumerate() {
            let text_w = shortcut.label.len() as f32 * Self::CHAR_WIDTH_APPROX;
            let chip_w = text_w + (Self::CHIP_PADDING_X * 2.0);
            bounds.push((
                ChipBounds {
                    x: cur_x,
                    y: bar_y + 2.0,
                    width: chip_w,
                    height: Self::CHIP_HEIGHT,
                },
                idx,
            ));
            cur_x += chip_w + Self::CHIP_GAP;
        }

        Some(Self {
            bounds,
            total_width: cur_x + scroll_x,
            bar_y,
            bar_height,
        })
    }

    /// Tests a touch coordinate against all chip hitboxes.
    pub fn hit_test<'a>(
        &self,
        config: &'a ChatShortcutsConfig,
        px: f32,
        py: f32,
    ) -> Option<&'a ChatShortcut> {
        for (bounds, idx) in &self.bounds {
            if bounds.contains(px, py) {
                return config.shortcuts.get(*idx);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_server_shortcuts() {
        let config = ChatShortcutsConfig::default();
        assert!(config.enabled);
        assert!(!config.show_in_singleplayer);
        assert_eq!(config.shortcuts.len(), 6);
        assert_eq!(config.shortcuts[0].label, "login");
        assert_eq!(config.shortcuts[0].command, "/login ");
        assert_eq!(config.shortcuts[0].trigger, ShortcutTrigger::Insert);

        assert_eq!(config.shortcuts[2].label, "home");
        assert_eq!(config.shortcuts[2].command, "/home");
        assert_eq!(config.shortcuts[2].trigger, ShortcutTrigger::QuickCast);
    }

    #[test]
    fn singleplayer_context_filtering() {
        let mut config = ChatShortcutsConfig::default();

        // On multiplayer server: always displays
        assert!(config.should_display(true));

        // On singleplayer world: hidden by default
        assert!(!config.should_display(false));

        // When user enables singleplayer display: visible
        config.show_in_singleplayer = true;
        assert!(config.should_display(false));

        // Master disabled toggle overrides all
        config.enabled = false;
        assert!(!config.should_display(true));
        assert!(!config.should_display(false));
    }

    #[test]
    fn json_serialization_roundtrip() {
        let mut config = ChatShortcutsConfig::default();
        config.add_shortcut("warp", "/warp ", ShortcutTrigger::Insert);

        let json = serde_json::to_string(&config).expect("serialization failed");
        assert!(json.contains("\"warp\""));
        assert!(json.contains("\"/warp \""));
        assert!(json.contains("\"show_in_singleplayer\":false"));

        let deserialized: ChatShortcutsConfig =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.shortcuts.len(), 7);
        assert_eq!(deserialized.shortcuts[6].label, "warp");
    }

    #[test]
    fn layout_computation_and_hit_testing() {
        let config = ChatShortcutsConfig::default();
        let layout = ChatShortcutsLayout::compute(&config, true, 200.0, 0.0)
            .expect("layout should compute for remote server");

        assert_eq!(layout.bounds.len(), 6);
        assert!(layout.bar_y < 200.0);

        // Test first chip bounds (login)
        let (first_chip, idx) = &layout.bounds[0];
        assert_eq!(*idx, 0);

        // Point inside first chip
        let hit = layout.hit_test(&config, first_chip.x + 2.0, first_chip.y + 2.0);
        assert_eq!(hit.map(|s| s.label.as_str()), Some("login"));

        // Point outside all chips
        let miss = layout.hit_test(&config, 9999.0, 9999.0);
        assert!(miss.is_none());
    }

    #[test]
    fn layout_hidden_in_singleplayer_by_default() {
        let config = ChatShortcutsConfig::default();
        let layout = ChatShortcutsLayout::compute(&config, false, 200.0, 0.0);
        assert!(layout.is_none());
    }

    #[test]
    fn add_and_remove_shortcuts() {
        let mut config = ChatShortcutsConfig::default();
        let initial_count = config.shortcuts.len();

        config.add_shortcut("shop", "/shop", ShortcutTrigger::QuickCast);
        assert_eq!(config.shortcuts.len(), initial_count + 1);

        let removed = config.remove_shortcut(initial_count);
        assert_eq!(removed.map(|s| s.label), Some("shop".into()));
        assert_eq!(config.shortcuts.len(), initial_count);
    }
}
