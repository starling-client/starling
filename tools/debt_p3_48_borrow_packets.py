#!/usr/bin/env python3
"""Migrate the play-packet hooks and their callers to borrowed views (debt p3-48).

Run once on the part-2 checkout, then rustfmt the changed Rust files.
"""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"
VOCAB = CRATES / "sunrise-mod-vocab/src"
CLIENT = CRATES / "sunrise-client"
NATIVE = CLIENT / "src/game/native_hooks"
sources: dict[Path, str] = {}


def replace(path: Path, old: str, new: str, expected: int = 1) -> None:
    if path not in sources:
        sources[path] = path.read_text()
    source = sources[path]
    count = source.count(old)
    if count != expected:
        raise SystemExit(
            f"{path.relative_to(ROOT)}: expected {expected} matches, found {count} for {old!r}"
        )
    sources[path] = source.replace(old, new)


replace(
    VOCAB / "client/types.rs",
    '''/// The metadata of a server play packet observed by `Hooks::client_packet`.
///
/// Fields beyond the type id are exposed only for custom payloads. This is
/// plain mod API data, not the game's decoded protocol enum.
// no wire: this is a local envelope for already-encoded vanilla packet bytes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ClientPacket {
    /// The protocol type, such as `minecraft:ping` or `minecraft:custom_payload`.
    /// An id outside the protocol's packet table is reported as `sunrise:unknown`.
    pub id: Identifier,
    /// The channel and bytes of a custom payload; `None` for other packet types.
    pub custom_payload: Option<ClientPayload>,
}

impl ClientPacket {
    /// Construct packet metadata with an optional custom payload.
    pub fn new(id: Identifier, custom_payload: Option<ClientPayload>) -> Self {
        Self { id, custom_payload }
    }
}

/// The channel and uninterpreted bytes of a server custom payload.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientPayload {
    /// The namespaced plugin channel, exactly as received.
    pub channel: String,
    /// The payload bytes, without the packet id or channel prefix.
    pub data: Vec<u8>,
}

impl ClientPayload {
    /// Construct a custom payload without interpreting its bytes.
    pub fn new(channel: String, data: Vec<u8>) -> Self {
        Self { channel, data }
    }
}
''',
    '''/// A read-only view of a play packet in `Hooks::client_packet` or `Hooks::client_packet_send`.
///
/// The view is valid only during the hook. Only custom payloads expose more
/// than the packet type. Copy any data you need to keep after the hook returns.
// no wire: borrows the decoded packet for synchronous observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientPacket<'a> {
    /// The protocol type, such as `minecraft:ping` or `minecraft:custom_payload`.
    /// An id outside the protocol's packet table is reported as `sunrise:unknown`.
    pub id: &'a Identifier,
    /// The channel and bytes of a custom payload; `None` for other packet types.
    pub custom_payload: Option<ClientPayload<'a>>,
}

impl<'a> ClientPacket<'a> {
    /// Construct a view from a borrowed type identifier and an optional payload view.
    pub fn new(id: &'a Identifier, custom_payload: Option<ClientPayload<'a>>) -> Self {
        Self { id, custom_payload }
    }
}

/// A borrowed channel and uninterpreted byte slice from a play custom payload.
// no wire: borrows the decoded packet for synchronous observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientPayload<'a> {
    /// The namespaced plugin channel, exactly as carried by the packet.
    pub channel: &'a str,
    /// The payload bytes, without the packet id or channel prefix.
    pub data: &'a [u8],
}

impl<'a> ClientPayload<'a> {
    /// Borrow a custom payload's channel and bytes without interpreting them.
    pub fn new(channel: &'a str, data: &'a [u8]) -> Self {
        Self { channel, data }
    }
}
''',
)

for directory in (VOCAB / "hooks", NATIVE):
    for hook, delimiter in (("client_packet", ")"), ("client_packet_send", ",")):
        replace(
            directory / f"{hook}.rs",
            f"packet: &ClientPacket{delimiter}",
            f"packet: &ClientPacket<'_>{delimiter}",
        )

replace(
    CRATES / "sunrise-mod-api-gen/dispatch-tests/src/fixtures/stubs/ClientPacket.rs",
    "pub(super) struct ClientPacket;",
    "pub(super) struct ClientPacket<'a>(pub &'a ());",
)

shared = NATIVE / "shared.rs"
replace(shared, ") -> ClientPacket {", ") -> ClientPacket<'_> {", expected=3)
replace(
    shared,
    '''fn packet_view(
    types: &[mod_api::Identifier],
    id: i32,
    payload: Option<&sunrise_protocol::packets::configuration::CustomPayload>,
) -> ClientPacket<'_> {''',
    '''fn packet_view<'a>(
    types: &'static [mod_api::Identifier],
    id: i32,
    payload: Option<&'a sunrise_protocol::packets::configuration::CustomPayload>,
) -> ClientPacket<'a> {''',
)
replace(
    shared,
    '''        .unwrap_or(&UNKNOWN)
        .clone();
    let payload = payload.map(|p| ClientPayload::new(p.channel.clone(), p.data.clone()));''',
    '''        .unwrap_or(&UNKNOWN);
    let payload = payload.map(|p| ClientPayload::new(p.channel.as_str(), p.data.as_slice()));''',
)

view_tests = NATIVE / "shared/packet_tests.rs"
replace(
    view_tests,
    "    let ping = super::client_packet_view(&Clientbound::Ping(Ping { id: 7 }));",
    "    let packet = Clientbound::Ping(Ping { id: 7 });\n"
    "    let ping = super::client_packet_view(&packet);",
)
replace(
    view_tests,
    '''    let payload = super::client_packet_view(&Clientbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![0, 127, 255],
    }));''',
    '''    let packet = Clientbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![0, 127, 255],
    });
    let payload = super::client_packet_view(&packet);''',
)
replace(
    view_tests,
    "    assert_eq!(payload.data, [0, 127, 255]);",
    "    assert_eq!(payload.data, &[0, 127, 255]);",
)
replace(
    view_tests,
    "        let unknown = super::client_packet_view(&Clientbound::Unknown(id, vec![9]));",
    "        let packet = Clientbound::Unknown(id, vec![9]);\n"
    "        let unknown = super::client_packet_view(&packet);",
)
replace(
    view_tests,
    '''    let slot =
        super::serverbound_packet_view(&Serverbound::SetCarriedItem(SetCarriedItem { slot: 3 }));''',
    "    let packet = Serverbound::SetCarriedItem(SetCarriedItem { slot: 3 });\n"
    "    let slot = super::serverbound_packet_view(&packet);",
)
replace(
    view_tests,
    '''    let payload = super::serverbound_packet_view(&Serverbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![1, 2],
    }));''',
    '''    let packet = Serverbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![1, 2],
    });
    let payload = super::serverbound_packet_view(&packet);''',
)
replace(
    view_tests,
    "(payload.channel.as_str(), payload.data.as_slice())",
    "(payload.channel, payload.data)",
)
replace(
    view_tests,
    "    let unknown = super::serverbound_packet_view(&Serverbound::Unknown(-1, vec![9]));",
    "    let packet = Serverbound::Unknown(-1, vec![9]);\n"
    "    let unknown = super::serverbound_packet_view(&packet);",
)

integration_test = CLIENT / "tests/it/mod_api_hooks.rs"
replace(
    integration_test,
    ".push((p.channel.clone(), p.data.clone()));",
    ".push((p.channel.to_owned(), p.data.to_vec()));",
)
replace(
    integration_test,
    '''        dispatch_client_packet(
            &mut FakeHost::new(),
            &ClientPacket::new(Identifier::parse("minecraft:ping").unwrap(), None),
        );
        let payload = ClientPayload::new("voicechat:secret".into(), vec![1, 2, 3]);
        dispatch_client_packet(
            &mut FakeHost::new(),
            &ClientPacket::new(
                Identifier::parse("minecraft:custom_payload").unwrap(),
                Some(payload),
            ),
        );''',
    '''        let ping = Identifier::parse("minecraft:ping").unwrap();
        dispatch_client_packet(&mut FakeHost::new(), &ClientPacket::new(&ping, None));
        let custom = Identifier::parse("minecraft:custom_payload").unwrap();
        let payload = ClientPayload::new("voicechat:secret", &[1, 2, 3]);
        dispatch_client_packet(
            &mut FakeHost::new(),
            &ClientPacket::new(&custom, Some(payload)),
        );''',
)

for path, source in sources.items():
    path.write_text(source)
