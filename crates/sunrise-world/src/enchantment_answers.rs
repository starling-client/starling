//! Registry-snapshot answers to item/enchantment questions, resolved when parts install.
//! Primary eligibility and enchantment support each own one table; engines never dispatch
//! listeners inside random rolls. Empty tables retain vanilla's answers with one check.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// Bind a member of an enchantment's item holder set to its canonical identifier.
/// Keep the tag marker so each snapshot can look up its bound item tags.
pub fn canonical_item_holder(holder: &str) -> String {
    let (marker, id) = holder
        .strip_prefix('#')
        .map_or(("", holder), |id| ("#", id));
    match sunrise_core::identifier::Identifier::parse(id) {
        Ok(id) => format!("{marker}{id}"),
        Err(_) => holder.to_owned(),
    }
}

/// Per item id, the enchantments with an installed answer and the answer itself.
/// Explicit support answers are retained even when they match a caller's default.
pub type Answers = HashMap<Arc<str>, HashMap<Arc<str>, bool>>;

/// One registry snapshot's resolved answers: written when the server installs its parts,
/// read by every engine operation over that snapshot.
#[derive(Default)]
pub struct AnswerTable {
    answers: RwLock<Answers>,
    /// Whether `answers` holds any answer: selection's one check when none does.
    any: AtomicBool,
}

impl Clone for AnswerTable {
    fn clone(&self) -> Self {
        let answers = self.read().clone();
        let any = AtomicBool::new(!answers.is_empty());
        Self {
            answers: RwLock::new(answers),
            any,
        }
    }
}

impl AnswerTable {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Answers> {
        self.answers.read().unwrap_or_else(PoisonError::into_inner)
    }
    /// Replace the stored answers, including explicit support answers that match a
    /// default. An empty table retires them.
    pub fn install(&self, mut answers: Answers) {
        answers.retain(|_, by_enchantment| !by_enchantment.is_empty());
        let any = !answers.is_empty();
        let mut stored = self.answers.write().unwrap_or_else(PoisonError::into_inner);
        *stored = answers;
        self.any.store(any, Ordering::Release);
    }
    /// Copy the resolved answers when another engine publishes the same registry snapshot.
    pub fn snapshot(&self) -> Answers {
        self.read().clone()
    }
    /// Retire every stored answer: selection is vanilla's again.
    pub fn clear(&self) {
        self.install(Answers::new());
    }
    /// Whether any answer is stored.
    pub fn is_empty(&self) -> bool {
        !self.any.load(Ordering::Acquire)
    }
    /// The installed answer for `item` and `enchantment`, if one was recorded.
    pub fn answer(&self, item: &str, enchantment: &str) -> Option<bool> {
        if self.is_empty() {
            return None;
        }
        self.read().get(item)?.get(enchantment).copied()
    }
}
