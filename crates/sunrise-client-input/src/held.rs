//! The keys a player's mappings hold, as their vanilla default keys. Devices
//! share held mappings; a direct `KeyMapping.setDown` write changes the result
//! without changing which devices hold the key.

use crate::actions::Action;
use std::collections::HashMap;
use winit::keyboard::KeyCode;

/// Where a press came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The keyboard and mouse.
    Keyboard,
    Pad,
    /// `sunrise-ctl`'s `key` and `mouse` and the agent commands that walk and click.
    Agent,
}

impl Source {
    fn bit(self) -> u8 {
        match self {
            Source::Keyboard => 1,
            Source::Pad => 2,
            Source::Agent => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Hold {
    sources: u8,
    override_down: Option<bool>,
}

impl Hold {
    fn down(self) -> bool {
        self.override_down.unwrap_or(self.sources != 0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldKeys {
    held: HashMap<KeyCode, Hold>,
}

impl HeldKeys {
    /// Whether this mapping is down, whatever the mapping is bound to.
    pub fn held(&self, action: Action) -> bool {
        action.key().is_some_and(|key| self.raw(&key))
    }
    /// Whether a key that is no mapping's default is held (Right Shift, the arrows, Alt, the system
    /// key), or the key a caller already holds as a key: most code wants [`HeldKeys::held`].
    pub fn raw(&self, key: &KeyCode) -> bool {
        self.held.get(key).is_some_and(|hold| hold.down())
    }
    /// Set the mapping's held flag, not a device's held flag. Its next input event takes over.
    pub fn override_down(&mut self, key: KeyCode, down: bool) {
        self.held.entry(key).or_default().override_down = Some(down);
    }
    pub fn override_for(&self, key: &KeyCode) -> Option<bool> {
        self.held.get(key)?.override_down
    }
    /// Drop the direct write when the mapping receives an input handled outside this table.
    pub fn resume_device_input(&mut self, key: &KeyCode) {
        if let Some(hold) = self.held.get_mut(key) {
            hold.override_down = None;
            if hold.sources == 0 {
                self.held.remove(key);
            }
        }
    }
    /// Starling: [`HeldKeys::raw`] under the name native mods made for Sunrise use, from when the
    /// held keys were a `HashSet<KeyCode>` (`game.keys.contains(&KeyCode::ShiftLeft)`).
    pub fn contains(&self, key: &KeyCode) -> bool {
        self.raw(key)
    }
    pub fn clear_overrides(&mut self) {
        self.held.retain(|_, hold| {
            hold.override_down = None;
            hold.sources != 0
        });
    }
    /// Synchronize a sneak/sprint latch without treating background reconciliation as a key
    /// event. A direct write lasts until input actually reaches the mapping again.
    pub fn sync_latch(&mut self, key: KeyCode, down: bool) {
        if down {
            let hold = self.held.entry(key).or_default();
            if hold.sources == 0 {
                hold.sources = Source::Keyboard.bit();
            }
        } else if let Some(hold) = self.held.get_mut(&key) {
            hold.sources = 0;
            if hold.override_down.is_none() {
                self.held.remove(&key);
            }
        }
    }
    /// `source` holds `key`. True when no device held it before; a direct mapping write
    /// must not swallow the next real press.
    pub fn press(&mut self, key: KeyCode, source: Source) -> bool {
        let hold = self.held.entry(key).or_default();
        let new = hold.sources == 0;
        hold.sources |= source.bit();
        hold.override_down = None;
        new
    }
    /// `source` lets go of `key`. True when nothing holds it any more.
    pub fn release(&mut self, key: &KeyCode, source: Source) -> bool {
        let Some(hold) = self.held.get_mut(key) else {
            return true;
        };
        hold.sources &= !source.bit();
        hold.override_down = None;
        if hold.sources == 0 {
            self.held.remove(key);
            true
        } else {
            false
        }
    }
    pub fn held_by(&self, key: &KeyCode, source: Source) -> bool {
        self.held
            .get(key)
            .is_some_and(|hold| hold.sources & source.bit() != 0)
    }
    /// `source` lets go of everything: the keys nothing holds any more.
    pub fn release_source(&mut self, source: Source) -> Vec<KeyCode> {
        let keys: Vec<KeyCode> = self
            .held
            .iter()
            .filter(|(_, hold)| hold.sources & source.bit() != 0)
            .map(|(key, _)| *key)
            .collect();
        keys.into_iter()
            .filter(|key| self.release(key, source))
            .collect()
    }
    /// Held, by the keyboard unless a device already holds it: for callers
    /// that don't say where the press came from.
    pub fn insert(&mut self, key: KeyCode) -> bool {
        let hold = self.held.entry(key).or_default();
        let new = hold.sources == 0;
        hold.override_down = None;
        if hold.sources == 0 {
            hold.sources = Source::Keyboard.bit();
        }
        new
    }
    /// Let go of `key` whoever holds it.
    pub fn remove(&mut self, key: &KeyCode) -> bool {
        self.held.remove(key).is_some_and(Hold::down)
    }
    pub fn clear(&mut self) {
        self.held.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
    pub fn len(&self) -> usize {
        self.iter().count()
    }
    pub fn iter(&self) -> impl Iterator<Item = &KeyCode> {
        self.held
            .iter()
            .filter_map(|(key, hold)| hold.down().then_some(key))
    }
}

impl FromIterator<KeyCode> for HeldKeys {
    fn from_iter<I: IntoIterator<Item = KeyCode>>(keys: I) -> Self {
        let mut held = HeldKeys::default();
        for key in keys {
            held.insert(key);
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_stays_held_while_any_device_holds_it() {
        let mut keys = HeldKeys::default();
        assert!(keys.press(KeyCode::Space, Source::Pad));
        assert!(!keys.press(KeyCode::Space, Source::Keyboard));
        assert!(keys.press(KeyCode::KeyW, Source::Keyboard));
        assert_eq!(keys.release_source(Source::Keyboard), vec![KeyCode::KeyW]);
        assert!(keys.held(Action::Jump) && !keys.held(Action::Forward));
        assert!(keys.held_by(&KeyCode::Space, Source::Pad));
        assert!(!keys.held_by(&KeyCode::Space, Source::Keyboard));
        assert!(keys.release(&KeyCode::Space, Source::Pad));
        assert!(keys.is_empty());
    }

    #[test]
    fn a_script_holds_a_key_by_itself() {
        let mut keys = HeldKeys::default();
        keys.press(KeyCode::KeyW, Source::Agent);
        keys.press(KeyCode::KeyW, Source::Keyboard);
        assert!(!keys.release(&KeyCode::KeyW, Source::Keyboard));
        assert!(keys.held(Action::Forward));
        assert!(keys.release_source(Source::Pad).is_empty());
        assert_eq!(keys.release_source(Source::Agent), vec![KeyCode::KeyW]);
        assert!(!keys.held(Action::Forward));
    }

    #[test]
    fn untagged_inserts_and_removes_act_as_before() {
        let mut keys = HeldKeys::default();
        keys.press(KeyCode::ShiftLeft, Source::Pad);
        assert!(!keys.insert(KeyCode::ShiftLeft), "already held");
        assert!(!keys.held_by(&KeyCode::ShiftLeft, Source::Keyboard));
        assert!(
            keys.remove(&KeyCode::ShiftLeft),
            "remove lets go for everyone"
        );
        assert!(!keys.held(Action::Sneak));
        assert!(keys.release(&KeyCode::KeyA, Source::Keyboard), "never held");
    }

    #[test]
    fn direct_mapping_write_yields_to_the_next_device_event() {
        let mut keys = HeldKeys::default();
        let key = KeyCode::KeyW;
        for source in [Source::Keyboard, Source::Pad, Source::Agent] {
            keys.press(key, source);
            keys.override_down(key, false);
            assert!(!keys.held(Action::Forward));
            assert!(keys.is_empty());
            assert!(keys.held_by(&key, source));
            keys.press(key, source);
            assert!(keys.held(Action::Forward));
            keys.override_down(key, true);
            keys.release(&key, source);
            assert!(!keys.held(Action::Forward));
        }
        keys.override_down(key, true);
        assert!(
            keys.press(key, Source::Keyboard),
            "a real press is still new"
        );
        keys.press(key, Source::Pad);
        keys.override_down(key, false);
        keys.release(&key, Source::Keyboard);
        assert!(
            keys.held(Action::Forward),
            "the pad's own hold is unchanged"
        );
    }
}
