//! The Simple Voice Chat wire format (compatibility version 20, mod 2.6.x),
//! written from the packet layouts: plugin-channel payloads over the game
//! connection, and encrypted UDP voice packets.
//!
//! All numbers are big-endian like Minecraft's `FriendlyByteBuf`; a UUID is
//! its most then least significant 64 bits (Horizon's `u128`); a byte array
//! and a string are a VarInt length, then the bytes.

/// The handshake version a 2.6.x server expects in `request_secret`.
pub const COMPATIBILITY_VERSION: i32 = 20;
/// Ordered compatibility versions to try when a server runs an older or newer SVC build (18..=22).
pub const COMPATIBILITY_VERSIONS: [i32; 5] = [20, 21, 19, 18, 22];
/// The first byte of every UDP voice packet.
pub const MAGIC: u8 = 0xFF;
/// Largest Opus frame the protocol carries.
pub const MAX_OPUS: usize = 1275;
/// Largest encrypted body read from a UDP packet.
pub const MAX_UDP_BODY: usize = 2048;

/// Plugin channels.
pub mod channel {
    pub const REQUEST_SECRET: &str = "voicechat:request_secret";
    pub const SECRET: &str = "voicechat:secret";
    pub const UPDATE_STATE: &str = "voicechat:update_state";
    pub const STATE: &str = "voicechat:state";
    pub const STATES: &str = "voicechat:states";
    pub const REMOVE_STATE: &str = "voicechat:remove_state";
    pub const CREATE_GROUP: &str = "voicechat:create_group";
    pub const SET_GROUP: &str = "voicechat:set_group";
    pub const LEAVE_GROUP: &str = "voicechat:leave_group";
    pub const JOINED_GROUP: &str = "voicechat:joined_group";
    pub const ADD_GROUP: &str = "voicechat:add_group";
    pub const REMOVE_GROUP: &str = "voicechat:remove_group";
    pub const ADD_CATEGORY: &str = "voicechat:add_category";
    pub const REMOVE_CATEGORY: &str = "voicechat:remove_category";
    /// The channels the server sends on, announced in `minecraft:register`
    /// (a Paper server only sends plugin messages on registered channels).
    pub const CLIENTBOUND: [&str; 9] = [
        SECRET,
        STATE,
        STATES,
        REMOVE_STATE,
        JOINED_GROUP,
        ADD_GROUP,
        REMOVE_GROUP,
        ADD_CATEGORY,
        REMOVE_CATEGORY,
    ];
}

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn err<T>(m: &str) -> Result<T> {
    Err(Error(m.into()))
}

pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.at
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return err("packet too short");
        }
        let s = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    pub fn uuid(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.array()?))
    }
    pub fn varint(&mut self) -> Result<i32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            v |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v as i32);
            }
        }
        err("varint too long")
    }
    fn len(&mut self, max: usize) -> Result<usize> {
        let n = self.varint()?;
        match usize::try_from(n) {
            Ok(n) if n <= max => Ok(n),
            _ => err("length out of range"),
        }
    }
    pub fn byte_array(&mut self, max: usize) -> Result<Vec<u8>> {
        let n = self.len(max)?;
        Ok(self.bytes(n)?.to_vec())
    }
    /// `readUtf(max)`: at most `max` characters, so `max * 3` bytes.
    pub fn string(&mut self, max: usize) -> Result<String> {
        let n = self.len(max * 3)?;
        let s = std::str::from_utf8(self.bytes(n)?).map_err(|_| Error("bad utf-8".into()))?;
        if s.chars().count() > max {
            return err("string too long");
        }
        Ok(s.to_owned())
    }
}

#[derive(Default)]
pub struct Writer {
    pub data: Vec<u8>,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub fn i16(&mut self, v: i16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn uuid(&mut self, v: u128) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn varint(&mut self, v: i32) {
        let mut v = v as u32;
        loop {
            if v & !0x7F == 0 {
                self.u8(v as u8);
                return;
            }
            self.u8((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    pub fn byte_array(&mut self, b: &[u8]) {
        self.varint(b.len() as i32);
        self.data.extend_from_slice(b);
    }
    pub fn string(&mut self, s: &str) {
        self.byte_array(s.as_bytes());
    }
}

// ---------------------------------------------------------------- plugin channel

/// The Opus application the server asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Voip,
    Audio,
    RestrictedLowDelay,
}
impl Codec {
    pub fn opus_application(self) -> i32 {
        match self {
            Codec::Voip => 2048,
            Codec::Audio => 2049,
            Codec::RestrictedLowDelay => 2051,
        }
    }
}

/// `voicechat:secret`: everything needed to open the voice connection.
#[derive(Clone, Debug, PartialEq)]
pub struct Secret {
    pub secret: [u8; 16],
    pub server_port: i32,
    pub player: u128,
    pub codec: Codec,
    pub mtu: i32,
    pub distance: f64,
    pub keep_alive_ms: i32,
    pub groups_enabled: bool,
    /// Empty: the game server's host. Else a port, or `host[:port]`.
    pub voice_host: String,
    pub allow_recording: bool,
}
impl Secret {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let secret = r.array()?;
        let server_port = r.i32()?;
        let player = r.uuid()?;
        let codec = match r.u8()? {
            0 => Codec::Voip,
            1 => Codec::Audio,
            2 => Codec::RestrictedLowDelay,
            _ => return err("unknown codec"),
        };
        Ok(Self {
            secret,
            server_port,
            player,
            codec,
            mtu: r.i32()?,
            distance: r.f64()?,
            keep_alive_ms: r.i32()?,
            groups_enabled: r.bool()?,
            voice_host: r.string(32767)?,
            allow_recording: r.bool()?,
        })
    }
    /// Where to send UDP: (host, port), from the voice host setting and the
    /// game server's host.
    pub fn voice_address(&self, game_host: &str) -> (String, u16) {
        let default_port = self.server_port.clamp(0, 65535) as u16;
        let host = self.voice_host.trim();
        if host.is_empty() {
            return (game_host.to_owned(), default_port);
        }
        if let Ok(port) = host.parse::<u16>() {
            return (game_host.to_owned(), port);
        }
        let host = host.strip_prefix("voicechat://").unwrap_or(host);
        // [v6]:port, host:port, or a bare host.
        if let Some(rest) = host.strip_prefix('[')
            && let Some((h, tail)) = rest.split_once(']')
        {
            let port = tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .unwrap_or(default_port);
            return (h.to_owned(), port);
        }
        match host.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h.to_owned(), p.parse().unwrap_or(default_port)),
            _ => (host.to_owned(), default_port),
        }
    }
}

/// A player's voice state (`voicechat:state`, and each of `voicechat:states`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerState {
    pub disabled: bool,
    pub disconnected: bool,
    pub uuid: u128,
    pub name: String,
    pub group: Option<u128>,
}
impl PlayerState {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let disabled = r.bool()?;
        let disconnected = r.bool()?;
        let uuid = r.uuid()?;
        let name = r.string(32767)?;
        let group = if r.bool()? { Some(r.uuid()?) } else { None };
        Ok(Self {
            disabled,
            disconnected,
            uuid,
            name,
            group,
        })
    }
    pub fn write(&self, w: &mut Writer) {
        w.bool(self.disabled);
        w.bool(self.disconnected);
        w.uuid(self.uuid);
        w.string(&self.name);
        w.bool(self.group.is_some());
        if let Some(g) = self.group {
            w.uuid(g);
        }
    }
}

pub fn decode_states(data: &[u8]) -> Result<Vec<PlayerState>> {
    let mut r = Reader::new(data);
    let n = r.i32()?;
    if !(0..=100_000).contains(&n) {
        return err("bad state count");
    }
    (0..n).map(|_| PlayerState::read(&mut r)).collect()
}

/// How a group hears the world: 0 normal (members hear each other and
/// players nearby), 1 open (anyone nearby hears the group too), 2 isolated
/// (members hear only each other).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GroupKind {
    #[default]
    Normal,
    Open,
    Isolated,
}
impl GroupKind {
    pub const ALL: [GroupKind; 3] = [GroupKind::Normal, GroupKind::Open, GroupKind::Isolated];
    pub fn from_i16(v: i16) -> Self {
        match v {
            1 => GroupKind::Open,
            2 => GroupKind::Isolated,
            _ => GroupKind::Normal,
        }
    }
    pub fn to_i16(self) -> i16 {
        match self {
            GroupKind::Normal => 0,
            GroupKind::Open => 1,
            GroupKind::Isolated => 2,
        }
    }
}

/// A group the server lists (`voicechat:add_group`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: u128,
    pub name: String,
    pub has_password: bool,
    pub persistent: bool,
    pub hidden: bool,
    pub kind: GroupKind,
}
impl Group {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        Ok(Self {
            id: r.uuid()?,
            name: r.string(512)?,
            has_password: r.bool()?,
            persistent: r.bool()?,
            hidden: r.bool()?,
            kind: GroupKind::from_i16(r.i16()?),
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.uuid(self.id);
        w.string(&self.name);
        w.bool(self.has_password);
        w.bool(self.persistent);
        w.bool(self.hidden);
        w.i16(self.kind.to_i16());
        w.data
    }
}

/// `voicechat:create_group`: a name (at most 24 characters), an optional
/// password (the same) and the kind.
pub fn create_group(name: &str, password: Option<&str>, kind: GroupKind) -> Vec<u8> {
    let cut = |s: &str| s.chars().take(24).collect::<String>();
    let mut w = Writer::new();
    w.string(&cut(name));
    w.bool(password.is_some());
    if let Some(p) = password {
        w.string(&cut(p));
    }
    w.i16(kind.to_i16());
    w.data
}

/// `voicechat:set_group`: join group `id`, with its password if it has one.
pub fn join_group(id: u128, password: Option<&str>) -> Vec<u8> {
    let mut w = Writer::new();
    w.uuid(id);
    w.bool(password.is_some());
    if let Some(p) = password {
        w.string(p);
    }
    w.data
}

/// `voicechat:joined_group`: the group joined (None = none), and whether
/// the password was wrong.
pub fn decode_joined_group(data: &[u8]) -> Result<(Option<u128>, bool)> {
    let mut r = Reader::new(data);
    let group = if r.bool()? { Some(r.uuid()?) } else { None };
    Ok((group, r.bool()?))
}

/// `voicechat:request_secret`.
pub fn request_secret() -> Vec<u8> {
    request_secret_version(COMPATIBILITY_VERSION)
}

/// `voicechat:request_secret` with an explicit compatibility version.
pub fn request_secret_version(version: i32) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(version);
    w.data
}

/// `voicechat:update_state`: whether this player turned voice chat off.
pub fn update_state(disabled: bool) -> Vec<u8> {
    vec![disabled as u8]
}

/// `minecraft:register`: channel names separated by NUL.
pub fn register_payload() -> Vec<u8> {
    channel::CLIENTBOUND.join("\0").into_bytes()
}

// ---------------------------------------------------------------- UDP voice

/// AES-128-GCM with the 16-byte secret: a random 12-byte IV, then the
/// ciphertext and its 16-byte tag.
pub struct Cipher {
    key: ring::aead::LessSafeKey,
    rng: ring::rand::SystemRandom,
}
impl Cipher {
    pub fn new(secret: &[u8; 16]) -> Self {
        let unbound =
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, secret).expect("16-byte key");
        Self {
            key: ring::aead::LessSafeKey::new(unbound),
            rng: ring::rand::SystemRandom::new(),
        }
    }
    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        use ring::rand::SecureRandom;
        let mut iv = [0u8; 12];
        self.rng.fill(&mut iv).expect("random iv");
        let mut buf = plain.to_vec();
        self.key
            .seal_in_place_append_tag(
                ring::aead::Nonce::assume_unique_for_key(iv),
                ring::aead::Aad::empty(),
                &mut buf,
            )
            .expect("seal");
        let mut out = iv.to_vec();
        out.extend_from_slice(&buf);
        out
    }
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < 12 + 16 {
            return None;
        }
        let iv: [u8; 12] = data[..12].try_into().ok()?;
        let mut buf = data[12..].to_vec();
        let plain = self
            .key
            .open_in_place(
                ring::aead::Nonce::assume_unique_for_key(iv),
                ring::aead::Aad::empty(),
                &mut buf,
            )
            .ok()?;
        Some(plain.to_vec())
    }
}

/// A voice packet, either way.
#[derive(Clone, Debug, PartialEq)]
pub enum Voice {
    /// 1: our microphone (empty `data` = stopped talking).
    Mic {
        data: Vec<u8>,
        sequence: i64,
        whispering: bool,
    },
    /// 2: a player's voice, at that player's entity.
    PlayerSound {
        channel: u128,
        sender: u128,
        data: Vec<u8>,
        sequence: i64,
        distance: f32,
        whispering: bool,
        category: Option<String>,
    },
    /// 3: a group member's voice, not positioned.
    GroupSound {
        channel: u128,
        sender: u128,
        data: Vec<u8>,
        sequence: i64,
        category: Option<String>,
    },
    /// 4: a voice at a fixed place.
    LocationSound {
        channel: u128,
        sender: u128,
        location: [f64; 3],
        data: Vec<u8>,
        sequence: i64,
        distance: f32,
        category: Option<String>,
    },
    /// 5
    Authenticate { player: u128, secret: [u8; 16] },
    /// 6 (v18..=v20 empty ack)
    AuthenticateAck,
    /// 6 (v21+ ack carrying 16-byte challenge nonce)
    AuthenticateAckNonce([u8; 16]),
    /// 7: answered with the same packet.
    Ping { id: u128, timestamp: i64 },
    /// 8: answered with the same packet.
    KeepAlive,
    /// 9 (v18..=v20 empty connection check)
    ConnectionCheck,
    /// 9 (v21+ connection check echoing the 16-byte challenge nonce)
    ConnectionCheckNonce([u8; 16]),
    /// 10
    ConnectionCheckAck,
}

impl Voice {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Voice::Mic {
                data,
                sequence,
                whispering,
            } => {
                w.u8(1);
                w.byte_array(data);
                w.i64(*sequence);
                w.bool(*whispering);
            }
            Voice::PlayerSound {
                channel,
                sender,
                data,
                sequence,
                distance,
                whispering,
                category,
            } => {
                w.u8(2);
                w.uuid(*channel);
                w.uuid(*sender);
                w.byte_array(data);
                w.i64(*sequence);
                w.f32(*distance);
                w.u8((*whispering as u8) | ((category.is_some() as u8) << 1));
                if let Some(c) = category {
                    w.string(c);
                }
            }
            Voice::GroupSound {
                channel,
                sender,
                data,
                sequence,
                category,
            } => {
                w.u8(3);
                w.uuid(*channel);
                w.uuid(*sender);
                w.byte_array(data);
                w.i64(*sequence);
                w.u8((category.is_some() as u8) << 1);
                if let Some(c) = category {
                    w.string(c);
                }
            }
            Voice::LocationSound {
                channel,
                sender,
                location,
                data,
                sequence,
                distance,
                category,
            } => {
                w.u8(4);
                w.uuid(*channel);
                w.uuid(*sender);
                for v in location {
                    w.f64(*v);
                }
                w.byte_array(data);
                w.i64(*sequence);
                w.f32(*distance);
                w.u8((category.is_some() as u8) << 1);
                if let Some(c) = category {
                    w.string(c);
                }
            }
            Voice::Authenticate { player, secret } => {
                w.u8(5);
                w.uuid(*player);
                w.data.extend_from_slice(secret);
            }
            Voice::AuthenticateAck => w.u8(6),
            Voice::AuthenticateAckNonce(nonce) => {
                w.u8(6);
                w.data.extend_from_slice(nonce);
            }
            Voice::Ping { id, timestamp } => {
                w.u8(7);
                w.uuid(*id);
                w.i64(*timestamp);
            }
            Voice::KeepAlive => w.u8(8),
            Voice::ConnectionCheck => w.u8(9),
            Voice::ConnectionCheckNonce(nonce) => {
                w.u8(9);
                w.data.extend_from_slice(nonce);
            }
            Voice::ConnectionCheckAck => w.u8(10),
        }
        w.data
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let category = |r: &mut Reader, flags: u8| -> Result<Option<String>> {
            if flags & 2 != 0 {
                Ok(Some(r.string(16)?))
            } else {
                Ok(None)
            }
        };
        Ok(match r.u8()? {
            1 => Voice::Mic {
                data: r.byte_array(MAX_OPUS)?,
                sequence: r.i64()?,
                whispering: r.bool()?,
            },
            2 => {
                let channel = r.uuid()?;
                let sender = r.uuid()?;
                let data = r.byte_array(MAX_OPUS)?;
                let sequence = r.i64()?;
                let distance = r.f32()?;
                let flags = r.u8()?;
                let category = category(&mut r, flags)?;
                Voice::PlayerSound {
                    channel,
                    sender,
                    data,
                    sequence,
                    distance,
                    whispering: flags & 1 != 0,
                    category,
                }
            }
            3 => {
                let channel = r.uuid()?;
                let sender = r.uuid()?;
                let data = r.byte_array(MAX_OPUS)?;
                let sequence = r.i64()?;
                let flags = r.u8()?;
                let category = category(&mut r, flags)?;
                Voice::GroupSound {
                    channel,
                    sender,
                    data,
                    sequence,
                    category,
                }
            }
            4 => {
                let channel = r.uuid()?;
                let sender = r.uuid()?;
                let location = [r.f64()?, r.f64()?, r.f64()?];
                let data = r.byte_array(MAX_OPUS)?;
                let sequence = r.i64()?;
                let distance = r.f32()?;
                let flags = r.u8()?;
                let category = category(&mut r, flags)?;
                Voice::LocationSound {
                    channel,
                    sender,
                    location,
                    data,
                    sequence,
                    distance,
                    category,
                }
            }
            5 => Voice::Authenticate {
                player: r.uuid()?,
                secret: r.array()?,
            },
            6 => {
                if r.remaining() >= 16 {
                    Voice::AuthenticateAckNonce(r.array()?)
                } else {
                    Voice::AuthenticateAck
                }
            }
            7 => Voice::Ping {
                id: r.uuid()?,
                timestamp: r.i64()?,
            },
            8 => Voice::KeepAlive,
            9 => {
                if r.remaining() >= 16 {
                    Voice::ConnectionCheckNonce(r.array()?)
                } else {
                    Voice::ConnectionCheck
                }
            }
            10 => Voice::ConnectionCheckAck,
            id => return Err(Error(format!("unknown voice packet {id}"))),
        })
    }
}

/// A client's UDP datagram: magic, our UUID, then the encrypted packet.
pub fn wrap(cipher: &Cipher, player: u128, packet: &Voice) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(MAGIC);
    w.uuid(player);
    w.byte_array(&cipher.encrypt(&packet.encode()));
    w.data
}

/// A server's UDP datagram: magic, then the encrypted packet.
pub fn unwrap(cipher: &Cipher, datagram: &[u8]) -> Result<Voice> {
    let mut r = Reader::new(datagram);
    if r.u8()? != MAGIC {
        return err("not a voice packet");
    }
    let body = r.byte_array(MAX_UDP_BODY)?;
    let plain = cipher
        .decrypt(&body)
        .ok_or_else(|| Error("failed to decrypt".into()))?;
    Voice::decode(&plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u128 = 0x0123_4567_89ab_cdef_0011_2233_4455_6677;

    #[test]
    fn secret_packet_decodes() {
        let mut w = Writer::new();
        w.data.extend_from_slice(&[7u8; 16]);
        w.i32(24454);
        w.uuid(ME);
        w.u8(0);
        w.i32(1024);
        w.f64(48.0);
        w.i32(1000);
        w.bool(true);
        w.string("");
        w.bool(false);
        let s = Secret::decode(&w.data).unwrap();
        assert_eq!(s.secret, [7; 16]);
        assert_eq!(
            (s.server_port, s.player, s.codec, s.mtu),
            (24454, ME, Codec::Voip, 1024)
        );
        assert_eq!(
            (s.distance, s.keep_alive_ms, s.groups_enabled),
            (48.0, 1000, true)
        );
        assert_eq!(
            s.voice_address("play.example.net"),
            ("play.example.net".into(), 24454)
        );
    }

    #[test]
    fn voice_host_forms() {
        let mut s = Secret::decode(&{
            let mut w = Writer::new();
            w.data.extend_from_slice(&[0; 16]);
            w.i32(24454);
            w.uuid(1);
            w.u8(1);
            w.i32(1024);
            w.f64(48.0);
            w.i32(1000);
            w.bool(false);
            w.string("");
            w.bool(false);
            w.data
        })
        .unwrap();
        for (host, want) in [
            ("25000", ("game", 25000)),
            ("voice.example.net", ("voice.example.net", 24454)),
            ("voice.example.net:1234", ("voice.example.net", 1234)),
            ("voicechat://10.0.0.1:99", ("10.0.0.1", 99)),
            ("[::1]:5", ("::1", 5)),
        ] {
            s.voice_host = host.into();
            assert_eq!(
                s.voice_address("game"),
                (want.0.to_owned(), want.1),
                "{host}"
            );
        }
    }

    #[test]
    fn states_round_trip() {
        let a = PlayerState {
            disabled: false,
            disconnected: false,
            uuid: ME,
            name: "azetech_".into(),
            group: None,
        };
        let b = PlayerState {
            disabled: true,
            disconnected: false,
            uuid: 9,
            name: "x".into(),
            group: Some(42),
        };
        let mut w = Writer::new();
        w.i32(2);
        a.write(&mut w);
        b.write(&mut w);
        assert_eq!(decode_states(&w.data).unwrap(), vec![a, b]);
    }

    #[test]
    fn voice_packets_round_trip() {
        let packets = [
            Voice::Mic {
                data: vec![1, 2, 3],
                sequence: 7,
                whispering: true,
            },
            Voice::Mic {
                data: vec![],
                sequence: 8,
                whispering: false,
            },
            Voice::PlayerSound {
                channel: 1,
                sender: 2,
                data: vec![9; 40],
                sequence: -1,
                distance: 48.0,
                whispering: false,
                category: Some("music".into()),
            },
            Voice::GroupSound {
                channel: 3,
                sender: 4,
                data: vec![5],
                sequence: 10,
                category: None,
            },
            Voice::LocationSound {
                channel: 5,
                sender: 6,
                location: [1.0, 2.0, 3.0],
                data: vec![],
                sequence: 0,
                distance: 10.0,
                category: None,
            },
            Voice::Authenticate {
                player: ME,
                secret: [3; 16],
            },
            Voice::AuthenticateAck,
            Voice::Ping {
                id: 77,
                timestamp: 123,
            },
            Voice::KeepAlive,
            Voice::ConnectionCheck,
            Voice::ConnectionCheckAck,
        ];
        for p in packets {
            assert_eq!(Voice::decode(&p.encode()).unwrap(), p);
        }
    }

    #[test]
    fn mic_packet_bytes_match_the_layout() {
        // id 1, VarInt length 2, data, long sequence, boolean.
        let bytes = Voice::Mic {
            data: vec![0xAA, 0xBB],
            sequence: 1,
            whispering: false,
        }
        .encode();
        assert_eq!(bytes, [1, 2, 0xAA, 0xBB, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
    }

    #[test]
    fn encrypted_datagram_round_trips_and_rejects_tampering() {
        let cipher = Cipher::new(&[9; 16]);
        let packet = Voice::Authenticate {
            player: ME,
            secret: [9; 16],
        };
        let datagram = wrap(&cipher, ME, &packet);
        assert_eq!(datagram[0], MAGIC);
        assert_eq!(u128::from_be_bytes(datagram[1..17].try_into().unwrap()), ME);
        // The server's form has no UUID: drop it to read our own packet back.
        let mut server_form = vec![MAGIC];
        server_form.extend_from_slice(&datagram[17..]);
        assert_eq!(unwrap(&cipher, &server_form).unwrap(), packet);
        let last = server_form.len() - 1;
        server_form[last] ^= 1;
        assert!(unwrap(&cipher, &server_form).is_err());
        assert!(
            unwrap(&Cipher::new(&[8; 16]), &{
                let mut s = vec![MAGIC];
                s.extend_from_slice(&datagram[17..]);
                s
            })
            .is_err()
        );
    }

    #[test]
    fn register_names_every_clientbound_channel() {
        let p = String::from_utf8(register_payload()).unwrap();
        assert_eq!(p.split('\0').count(), 9);
        assert!(p.starts_with("voicechat:secret\0"));
    }

    /// Group packets in the plugin's layout (ClientGroup, CreateGroupPacket,
    /// JoinGroupPacket, JoinedGroupPacket).
    #[test]
    fn group_packets() {
        let g = Group {
            id: 7,
            name: "Base".into(),
            has_password: true,
            persistent: false,
            hidden: false,
            kind: GroupKind::Isolated,
        };
        let bytes = g.encode();
        // uuid(16) + varint len + "Base" + 3 bools + short
        assert_eq!(bytes.len(), 16 + 1 + 4 + 3 + 2);
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 2]);
        assert_eq!(Group::decode(&bytes).unwrap(), g);

        assert_eq!(
            create_group("ab", None, GroupKind::Open),
            vec![2, b'a', b'b', 0, 0, 1]
        );
        assert_eq!(
            create_group("ab", Some("x"), GroupKind::Normal),
            vec![2, b'a', b'b', 1, 1, b'x', 0, 0]
        );
        assert_eq!(
            create_group(&"n".repeat(40), None, GroupKind::Normal)[0],
            24
        );

        let j = join_group(1, Some("pw"));
        assert_eq!(j.len(), 16 + 1 + 1 + 2);
        assert_eq!(j[16], 1);
        assert_eq!(join_group(1, None)[16], 0);

        let mut w = Writer::new();
        w.bool(true);
        w.uuid(9);
        w.bool(false);
        assert_eq!(decode_joined_group(&w.data).unwrap(), (Some(9), false));
        assert_eq!(decode_joined_group(&[0, 1]).unwrap(), (None, true));
    }
}
