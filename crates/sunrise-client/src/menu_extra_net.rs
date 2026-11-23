//! Menu extra data, client side: the answer to a Sunrise server's `sunrise:menu_extra` revision
//! query in configuration (`sunrise_protocol::menu_extra`). A server whose query the client
//! answered sends this revision's payload layout; one that never asked (an older Sunrise
//! server) sends the legacy layout, and the client reads that.
use sunrise_protocol::{menu_extra, packets::configuration as cfg};

/// Whether `payload` is the server's menu-extra revision query.
pub fn asked(payload: &cfg::CustomPayload) -> bool {
    payload.channel == menu_extra::CHANNEL
}

/// The answer to the query `data`, or the reason that closes the connection.
pub fn answer(data: &[u8]) -> Result<cfg::Serverbound, String> {
    menu_extra::read_query(data).map_err(|error| error.to_string())?;
    Ok(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
        channel: menu_extra::CHANNEL.into(),
        data: menu_extra::answer(),
    }))
}
