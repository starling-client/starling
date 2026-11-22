//! Account profile data shared by login, saved profiles and the mod host.

/// A property of a game profile (authlib's `Property`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountProperty {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}
