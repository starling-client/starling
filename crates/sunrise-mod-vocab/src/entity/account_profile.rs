//! Account profiles as the session service returns them (`GameProfile`), owned.

/// One property of an account profile (`com.mojang.authlib.properties.Property`): a skin's
/// `textures`, for one, with the session service's signature.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a plain record mods read and build
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AccountProfileProperty {
    /// The property's name (`textures`).
    pub name: String,
    /// Its value, as the session service sent it.
    pub value: String,
    /// The session service's signature of the value, when it signed it.
    pub signature: Option<String>,
}

/// An account's profile (`GameProfile`): its UUID, its name and its properties.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a plain record mods read and build
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AccountProfile {
    /// The account's UUID.
    pub uuid: u128,
    /// The account's name, as the session service spells it.
    pub name: String,
    /// The profile's properties, in the session service's order, signatures intact.
    pub properties: Vec<AccountProfileProperty>,
}

/// Where a profile lookup stands ([`Profiles::resolve_name`](crate::Profiles::resolve_name),
/// [`resolve_id`](crate::Profiles::resolve_id)): ask again with the same name or UUID to
/// poll it.
// no wire: a host reply a mod polls in process
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileResolution {
    /// The lookup is in flight.
    Pending,
    /// The answer: the profile, or `None` when the session service gave none. As in vanilla,
    /// a failed request answers `None` too, so `None` alone does not prove there is no such
    /// account.
    Done(Option<AccountProfile>),
}
