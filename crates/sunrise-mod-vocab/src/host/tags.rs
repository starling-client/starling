//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `tags` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait TagsHost {
    /// The merged-tag service, using load-resolved block, item, fluid and tag keys.
    #[service]
    fn tags(&self) -> &dyn Tags;
}
