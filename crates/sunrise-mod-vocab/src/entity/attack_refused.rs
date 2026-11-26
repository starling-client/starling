//! Why a mod's attack on an entity did not run: [`PlayerRef::attack`](crate::PlayerRef::attack)
//! and [`MobRef::do_hurt_target`](crate::MobRef::do_hurt_target).

/// Why [`PlayerRef::attack`](crate::PlayerRef::attack) or
/// [`MobRef::do_hurt_target`](crate::MobRef::do_hurt_target) did nothing: it was refused before
/// any of the attack ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum AttackRefused {
    /// The attacker's or the target's handle names nothing here (another world epoch, an entity
    /// gone, a player who left), or the call came from a client or a host without a game.
    Stale,
    /// The target is not in the attacker's level, or the attacker's level is not the one the
    /// server is running now: the engine runs an attack in its own level's turn.
    OtherLevel,
    /// Another player's turn, an open transaction, or another `attack`, `do_hurt_target` or
    /// fake player's `perform_action` is running, or the mob is held out of the level for its
    /// own tick. A player's swapped-in state must not reach another one's call. As for
    /// `perform_action`, a `level_tick_pre` or `level_tick_post` listener's own failure
    /// boundary is not an open transaction here: the attack runs outside it, so the listener's
    /// panic doesn't undo the attack. A `host.transaction` the listener opens is one.
    Busy,
    /// Sunrise cannot run this attack on this target: a player's on itself or on an entity it
    /// keeps no attack for, or a mob's on an entity that is neither a player nor a mob (an
    /// armor stand, a boat, an item frame...).
    Unsupported,
}

impl std::fmt::Display for AttackRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale => "the attacker or the target is not here",
            Self::OtherLevel => "the target is not in the level the attacker's attack runs in",
            Self::Busy => "another player's turn, transaction or attack is running",
            Self::Unsupported => "Sunrise cannot run this attack on this target",
        })
    }
}

impl std::error::Error for AttackRefused {}

impl crate::NoBackend for AttackRefused {
    /// No game: as for a handle that names nothing here.
    fn no_backend() -> Self {
        Self::Stale
    }
}
