// exports: none (reached inside the crate only)
//! Registration refusals and loading messages.

/// Why a native declaration was not accepted.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum DeclarationError {
    /// The thread is no longer in the registration phase.
    RegistrationClosed {
        /// Identifier supplied by the caller.
        id: String,
    },
    /// The identifier is not a valid resource location.
    InvalidId {
        /// Rejected identifier, including any inferred namespace.
        id: String,
    },
    /// The mod's manifest does not own the identifier's namespace.
    UnownedNamespace {
        /// Rejected identifier.
        id: String,
        /// Namespace not available to the declaring mod.
        namespace: String,
    },
    /// A declaration failed another registry-specific check.
    Rejected {
        /// Rejected identifier.
        id: String,
        /// Registry-specific explanation.
        reason: String,
    },
}

impl DeclarationError {
    /// The identifier of the rejected declaration.
    pub fn id(&self) -> &str {
        match self {
            Self::RegistrationClosed { id }
            | Self::InvalidId { id }
            | Self::UnownedNamespace { id, .. }
            | Self::Rejected { id, .. } => id,
        }
    }
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RegistrationClosed { id } => write!(f, "registration is closed for {id:?}"),
            Self::InvalidId { id } => write!(f, "invalid identifier {id:?}"),
            Self::UnownedNamespace { id, namespace } => {
                write!(f, "{id:?} in unowned namespace {namespace:?}")
            }
            Self::Rejected { id, reason } => write!(f, "{id:?}: {reason}"),
        }
    }
}
impl std::error::Error for DeclarationError {}

// no wire: engine-side admission record; only its typed error crosses to the mod.
// hidden: the game collects these records before opening any world.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclarationRefusal {
    /// ID of the submitting package, not necessarily the declaration's namespace.
    pub mod_id: String,
    /// Registry or declaration kind.
    pub kind: DeclarationKind,
    /// Rejected identifier and reason.
    pub error: DeclarationError,
}

impl DeclarationRefusal {
    /// Attribute a refused declaration to the package that submitted it.
    pub fn new(mod_id: impl Into<String>, kind: DeclarationKind, error: DeclarationError) -> Self {
        Self {
            mod_id: mod_id.into(),
            kind,
            error,
        }
    }
}

// no wire: engine-side admission record discriminant.
// hidden: the game owns the admission report.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the engine translates every kind used in admission diagnostics.
pub enum DeclarationKind {
    /// A native block declaration.
    Block,
    /// A native item declaration.
    Item,
}

/// A loading message as a translation key and its arguments, resolved against the
/// selected language when the game shows it (NeoForge's `ModLoadingIssue`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadingMessage {
    pub(crate) key: String,
    pub(crate) args: Vec<String>,
}

impl LoadingMessage {
    /// A message from `key` and its `%s` arguments, stored verbatim.
    pub fn new(key: &str, args: &[&str]) -> Self {
        Self {
            key: key.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }
    /// The translation key; also the text shown when no language has it.
    pub fn key(&self) -> &str {
        &self.key
    }
    /// The arguments, in `%s` order.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// Registration diagnostics for the mod currently running
/// [`Hooks::register`](crate::Hooks::register).
pub trait DiagnosticsRegistrar {
    /// Refuse this mod's registration without panicking.
    ///
    /// Every refusal reason recorded during the callback is preserved verbatim and reported,
    /// including an empty reason. If the callback later panics, the reasons are logged with
    /// the mod's ID before the pending batch is discarded. All of this mod's pending declarations
    /// from the callback are discarded, including declarations made before or after the call,
    /// without adding secondary validation errors. Other mods' declarations are unchanged.
    /// Return from the callback after refusing registration.
    fn refuse(&mut self, reason: &str);

    /// Record a loading problem without refusing this mod (NeoForge's
    /// `ModLoader.addLoadingIssue` with `ModLoadingIssue.withAffectedMod`).
    ///
    /// `key` is a translation key in the mod's own `lang/` files and `args` its `%s`
    /// arguments, stored verbatim in call order with no per-mod limit. The log names the
    /// mod, key, and arguments; the Mods screen shows the first warning in the player's
    /// language.
    fn warn(&mut self, key: &str, args: &[&str]);
}

/// The loading warnings mod `id` gave during registration, in order, from the installed Parts.
// hidden: read by the game's Mods screen.
#[doc(hidden)]
pub fn registration_warnings(id: &str) -> Vec<LoadingMessage> {
    super::installed().map_or_else(Vec::new, |installed| {
        installed
            .warnings
            .iter()
            .filter(|(owner, _)| *owner == id)
            .flat_map(|(_, warnings)| warnings.iter().cloned())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loading_message_keeps_key_and_arguments_verbatim() {
        let long_key = format!("{}é!", "a".repeat(4096));
        let long_arg = format!("{}é!", "b".repeat(4096));
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let mut args: Vec<&str> = vec!["", &long_arg];
        args.extend(many.iter().map(String::as_str));
        let warning = LoadingMessage::new(&long_key, &args);
        assert_eq!(warning.key(), long_key);
        assert_eq!(warning.args()[0], "");
        assert_eq!(warning.args()[1], long_arg);
        assert_eq!(warning.args().len(), 22);
        assert_eq!(warning.args()[21], "19");
    }
}
