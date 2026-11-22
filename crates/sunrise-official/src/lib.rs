//! The official game: the platform's game (`sunrise-game`) with Sunrise's own in-game parts
//! plugged in through the platform's hooks (docs/layers.md, "How a Sunrise crate plugs in").
//! The release builds ship its executable under the game's name, `sunrise-client`
//! (`game-build.yml`); sunrise-client keeps its own plain executable.
//!
//! Each Sunrise crate that runs in the game gives a `&'static` value implementing
//! `sunrise_platform::hooks::Hooks`, and its settings store, pages and HUD layers if it has any,
//! and [`register`] adds them. Nothing here depends on an engine crate.
#![forbid(unsafe_code)]

// In the SDK this keeps the platform's link to the host dylib, which owns the hook and settings
// registries. Without it, rustc can link a private copy into the official dylib.
extern crate sunrise_game as _;

#[cfg(feature = "sunrise-extras")]
mod mod_audio;

use sunrise_platform::hooks::{self, ClientIdentity, GetTheGame, Hooks};
use sunrise_platform::{hud, lang, mod_rows, pages, release, settings};

/// Every Sunrise part in the game, in the order its hooks run.
const PARTS: &[&dyn Hooks] = &[
    // The anonymous "I'm playing" ping.
    &sunrise_heartbeat::HOOKS,
    // Play with Friends: `friend:` addresses join through its tunnel.
    &sunrise_friends::game::HOOKS,
    // Where a game started without its prepared files sends the player.
    &LAUNCHER_PAGE,
    // "Playing Sunrise" on the player's Discord profile.
    &sunrise_discord::HOOKS,
    // Open and Share for saved performance captures.
    &sunrise_perf_share::HOOKS,
    // The client identity the official Horizon server requires.
    &HORIZON_IDENTITY,
    // Auto-reconnect: the Disconnected screen connects again.
    #[cfg(feature = "sunrise-extras")]
    &sunrise_reconnect::HOOKS,
    // Frames per second and frame times over the game.
    #[cfg(feature = "sunrise-extras")]
    &sunrise_perf_overlay::HOOKS,
];
/// The game started by hand without its prepared files: start it from the launcher, downloaded
/// from Sunrise's website.
static LAUNCHER_PAGE: LauncherPage = LauncherPage;

struct LauncherPage;

impl Hooks for LauncherPage {
    fn get_the_game(&self) -> Option<GetTheGame> {
        Some(GetTheGame {
            address: sunrise_addresses::WEBSITE,
            title: "sunrise.get_the_game.title",
            paragraphs: &[
                "sunrise.get_the_game.about",
                "sunrise.get_the_game.minecraft",
                "sunrise.get_the_game.download",
            ],
            button: "sunrise.get_the_game.button",
        })
    }
}

/// The official Horizon server requires the brand `horizon` and a payload offer on
/// `horizon:payloads` before the client answers its known packs.
static HORIZON_IDENTITY: HorizonIdentity = HorizonIdentity;

struct HorizonIdentity;

/// The ports a typed address to the server may carry besides none: its SRV target's and the
/// default one (`ServerAddress.DEFAULT_PORT`).
const HORIZON_PORTS: [u16; 2] = [24454, 25565];

/// Whether the address as typed names the Horizon server: its host ignoring ASCII case and one
/// trailing dot, with no port or one of [`HORIZON_PORTS`].
fn is_horizon_server(address: &str) -> bool {
    let host = match address.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) if HORIZON_PORTS.contains(&port) => host,
            _ => return false,
        },
        None => address,
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    host.eq_ignore_ascii_case(sunrise_addresses::HORIZON_SERVER)
}

impl Hooks for HorizonIdentity {
    fn client_identity(&self, server: &str) -> Option<ClientIdentity> {
        is_horizon_server(server).then(|| ClientIdentity {
            brand: "horizon".to_owned(),
            // An offer declaring nothing: revision 1, no records.
            payloads: vec![("horizon:payloads".to_owned(), vec![1, 1, 0])],
        })
    }
}

/// Sunrise's row on the Mods screen, after Minecraft.
static SUNRISE_ROW: mod_rows::Row = mod_rows::Row {
    id: "sunrise",
    name: "Sunrise",
    version: release::VERSION,
    authors: "sunrise.mods.screen.sunrise.authors",
    license: "PolyForm Shield 1.0.0",
    license_url: Some(sunrise_addresses::LICENSE),
    homepage: Some(sunrise_addresses::WEBSITE),
    issues: Some(sunrise_addresses::ISSUES),
    description: "sunrise.mods.screen.sunrise.description",
};

/// Their settings, in the player's settings files.
const STORES: &[&dyn settings::Store] = &[
    #[cfg(feature = "sunrise-extras")]
    &sunrise_reconnect::SETTING,
    #[cfg(feature = "sunrise-extras")]
    &mod_audio::SETTING,
    &sunrise_discord::SETTING,
    #[cfg(feature = "sunrise-extras")]
    &sunrise_perf_overlay::SETTING,
];
/// Their rows and pages in the options screens, in order: auto-reconnect's row above Discord's.
const PAGES: &[&dyn pages::Page] = &[
    #[cfg(feature = "sunrise-extras")]
    &sunrise_reconnect::SECTION,
    #[cfg(feature = "sunrise-extras")]
    &mod_audio::SECTION,
    &sunrise_discord::SECTION,
    #[cfg(feature = "sunrise-extras")]
    &sunrise_perf_overlay::SECTION,
];
/// What they draw on the HUD.
const LAYERS: &[&dyn hud::Layer] = &[
    #[cfg(feature = "sunrise-extras")]
    &sunrise_perf_overlay::LAYER,
];

/// Sunrise's official text (its rows and the launcher page), loaded with the game's.
static LANG: &lang::Files = &[("en_us", include_str!("../assets/lang/en_us.json"))];

/// Registers Sunrise's in-game parts with the game. The executable calls it before it runs the
/// game, and so does the launcher's generated executable for native mods.
pub fn register() {
    let late = PARTS.iter().copied().map(hooks::register);
    let late = late.chain(STORES.iter().copied().map(settings::register));
    let late = late.chain(std::iter::once(mod_rows::register(&SUNRISE_ROW)));
    let late = late.chain(std::iter::once(lang::register(LANG)));
    #[cfg(feature = "sunrise-extras")]
    let late = late.chain(std::iter::once(lang::register(sunrise_reconnect::LANG)));
    let late = late.chain(std::iter::once(lang::register(sunrise_discord::LANG)));
    let late = late.chain(PAGES.iter().copied().map(pages::register));
    for result in late.chain(LAYERS.iter().copied().map(hud::register)) {
        if let Err(e) = result {
            // The executable registered after starting the game: a bug, not a player's problem.
            log::warn!("Sunrise: an in-game part did not register: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_official_horizon_server_gets_its_identity() {
        let identity = |server: &str| HORIZON_IDENTITY.client_identity(server);
        let expected = Some(ClientIdentity {
            brand: "horizon".to_owned(),
            payloads: vec![("horizon:payloads".to_owned(), vec![1, 1, 0])],
        });
        for server in [
            "play.playhorizon.dev",
            "PLAY.PlayHorizon.DEV",
            "play.playhorizon.dev.",
            "play.playhorizon.dev:24454",
            "play.playhorizon.dev:25565",
            "Play.PlayHorizon.dev.:24454",
        ] {
            assert_eq!(identity(server), expected, "{server}");
        }
        for server in [
            "play.playhorizon.dev:25566",
            "play.playhorizon.dev:",
            "play.playhorizon.dev:abc",
            "play.playhorizon.dev..",
            "eu.play.playhorizon.dev",
            "playhorizon.dev",
            "example.org:24454",
            "127.0.0.1",
            "",
        ] {
            assert_eq!(identity(server), None, "{server}");
        }
    }

    #[test]
    fn the_sunrise_row_links_the_shipped_licence_site_and_issues() {
        assert_eq!(
            (
                SUNRISE_ROW.license_url,
                SUNRISE_ROW.homepage,
                SUNRISE_ROW.issues
            ),
            (
                Some("https://github.com/sunrise-org/sunrise/blob/main/LICENSE"),
                Some("https://playsunrise.net"),
                Some("https://github.com/sunrise-org/sunrise/issues")
            )
        );
    }
}
