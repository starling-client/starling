//! First-writer attribution for saved-world migration markers.

/// A marked mod id and the first listener that marked it.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: attributed mismatch resolution record
pub struct ModMismatchResolution {
    /// The id whose migration was handled, including ids without a difference.
    pub mod_id: String,
    /// The first resolving listener, assigned by [`crate::ModMismatchEvent::mark_resolved`].
    pub resolver: String,
}

impl crate::ModMismatchEvent<'_> {
    /// Mark `mod_id` as migrated by this listener (NeoForge's `ModMismatchEvent.markResolved`).
    ///
    /// First-writer wins: if an earlier listener already resolved `mod_id`, the earlier
    /// attribution is kept unchanged.
    pub fn mark_resolved(&mut self, mod_id: &str) {
        if self.resolved.iter().any(|old| old.mod_id == mod_id) {
            return;
        }
        self.resolved.push(ModMismatchResolution {
            mod_id: mod_id.to_owned(),
            resolver: self.mod_id.to_owned(),
        });
    }

    /// Whether any listener has marked `mod_id` as resolved (NeoForge's
    /// `ModMismatchEvent.wasResolved`).
    pub fn was_resolved(&self, mod_id: &str) -> bool {
        self.resolved.iter().any(|entry| entry.mod_id == mod_id)
    }

    /// The first listener that resolved `mod_id`, if any (NeoForge's
    /// `ModMismatchEvent.getResolver`).
    pub fn resolver(&self, mod_id: &str) -> Option<&str> {
        self.resolved
            .iter()
            .find(|entry| entry.mod_id == mod_id)
            .map(|entry| entry.resolver.as_str())
    }

    /// All resolutions recorded so far, in first-resolved order.
    pub fn resolved(&self) -> &[ModMismatchResolution] {
        self.resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_marker_wins_and_is_attributed_at_insertion() {
        let mut resolved = Vec::new();
        {
            let mut event = crate::ModMismatchEvent {
                mod_id: "first",
                mismatches: &[],
                resolved: &mut resolved,
            };
            event.mark_resolved("target");
            assert!(event.was_resolved("target"));
            assert_eq!(event.resolver("target"), Some("first"));
            assert!(!event.was_resolved("other"));
            assert_eq!(event.resolver("other"), None);
        }
        {
            let mut event = crate::ModMismatchEvent {
                mod_id: "second",
                mismatches: &[],
                resolved: &mut resolved,
            };
            event.mark_resolved("target");
            event.mark_resolved("other");
            assert_eq!(event.resolver("target"), Some("first"));
            assert_eq!(event.resolver("other"), Some("second"));
            assert_eq!(event.resolved().len(), 2);
        }
        assert_eq!(
            resolved,
            vec![
                ModMismatchResolution {
                    mod_id: "target".into(),
                    resolver: "first".into(),
                },
                ModMismatchResolution {
                    mod_id: "other".into(),
                    resolver: "second".into(),
                },
            ]
        );
    }
}
