//! The profiles service: account profiles through the server's `ProfileResolver.Cached`, the
//! resolver `/fetchprofile` and dynamic player heads use.
use super::Game;
use mod_api::{AccountProfile, AccountProfileProperty, ProfileResolution};
use sunrise_client::skins::{NameOrId, Resolve};

impl Game<'_> {
    /// Where the lookup of `key` stands, starting it the first time; `Done(None)` off a server.
    pub(in crate::game) fn native_resolve_profile(&self, key: NameOrId) -> ProfileResolution {
        if !self.is_logical_server() {
            return ProfileResolution::Done(None);
        }
        match self.skin_profiles.borrow_mut().state(&key) {
            Resolve::Pending => ProfileResolution::Pending,
            Resolve::Done(profile) => ProfileResolution::Done(profile.map(|profile| {
                AccountProfile {
                    uuid: profile.id,
                    name: profile.name,
                    properties: profile
                        .properties
                        .into_iter()
                        .map(|(name, value, signature)| AccountProfileProperty {
                            name,
                            value,
                            signature,
                        })
                        .collect(),
                }
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        execution_role::ExecutionRole,
        native_entity_refs::tests::{run_fresh_cases, with_game},
    };
    use mod_api::{ProfileResolution, ServerHost};

    /// A name `StringUtil.isValidPlayerName` refuses answers at once with no profile, as
    /// `fetchByName` does without a request; a client resolves nothing.
    #[test]
    #[ignore = "daily"]
    fn invalid_names_and_clients_resolve_no_profile() {
        if run_fresh_cases(
            "game::native_profiles::tests::invalid_names_and_clients_resolve_no_profile",
            "SUNRISE_PROFILES_CASE",
            &["server"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.execution_role = ExecutionRole::IntegratedServer;
            for name in ["two words", "seventeen_letters", "ünicode"] {
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    host.profiles().resolve_name(name),
                    ProfileResolution::Done(None),
                    "{name}"
                );
            }
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(
                game.native_resolve_profile(sunrise_client::skins::NameOrId::Id(1)),
                ProfileResolution::Done(None)
            );
        });
    }
}
