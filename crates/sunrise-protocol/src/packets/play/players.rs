//! The player list and signed chat: `ClientboundPlayerInfoUpdatePacket` and
//! `ClientboundPlayerInfoRemovePacket`, the signed player chat and its deletion, and the
//! serverbound chat, command signatures, last-seen acknowledgements and chat session.
use super::ChatTypeBound;
use crate::{
    Result,
    buf::{
        Byte, BytesN, Codec, List, Long, Opt, Reader, Str, StrN, TrustedComponent, Uuid, VarInt,
    },
    packet,
    packets::login::PropertyCodec,
};
use sunrise_core::{nbt::Tag, profile::AccountProperty, vanilla_bytes::Writer};

/// MessageSignature.Packed: an index into the last-seen cache or a full signature.
pub struct PackedSignature;
impl Codec<(i32, Option<Vec<u8>>)> for PackedSignature {
    fn read(r: &mut Reader) -> Result<(i32, Option<Vec<u8>>)> {
        let id = r.varint()? - 1;
        Ok(if id == -1 {
            (-1, Some(r.bytes(256)?.to_vec()))
        } else {
            (id, None)
        })
    }
    fn write(v: &(i32, Option<Vec<u8>>), w: &mut Writer) {
        w.varint(v.0 + 1);
        if let Some(sig) = &v.1 {
            w.bytes(sig);
        }
    }
}
/// MessageSignature: 256 raw bytes.
pub struct Signature;
impl Codec<Vec<u8>> for Signature {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        Ok(r.bytes(256)?.to_vec())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.bytes(v)
    }
    fn validate(v: &Vec<u8>) -> Result<()> {
        if v.len() != crate::secure_chat::SIGNATURE_BYTES {
            return Err(crate::Error::new("invalid MessageSignature width"));
        }
        Ok(())
    }
}
/// FilterMask: 0 pass-through, 1 fully filtered, 2 partially (with a BitSet).
pub struct FilterMask;
impl Codec<(i32, Vec<i64>)> for FilterMask {
    fn read(r: &mut Reader) -> Result<(i32, Vec<i64>)> {
        let kind = r.varint()?;
        Ok((
            kind,
            if kind == 2 {
                r.long_array()?
            } else {
                Vec::new()
            },
        ))
    }
    fn write(v: &(i32, Vec<i64>), w: &mut Writer) {
        w.varint(v.0);
        if v.0 == 2 {
            w.long_array(&v.1);
        }
    }
}
packet!(PlayerChat {
    global_index: i32 => VarInt,
    sender: u128 => Uuid,
    index: i32 => VarInt,
    signature: Option<Vec<u8>> => Opt<Signature>,
    content: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    last_seen: Vec<(i32, Option<Vec<u8>>)> => List<PackedSignature>,
    unsigned_content: Option<Tag> => Opt<TrustedComponent>,
    filter: (i32, Vec<i64>) => FilterMask,
    chat_type: ChatTypeBound => ChatTypeBound,
});
packet!(
    /// ClientboundDeleteChatPacket: the server deleted a player message,
    /// named by its packed signature.
    DeleteChat { signature: (i32, Option<Vec<u8>>) => PackedSignature }
);
packet!(PlayerInfoRemove { uuids: Vec<u128> => List<Uuid> });
/// One ClientboundPlayerInfoUpdatePacket entry; fields are set for the
/// actions in the packet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInfo {
    pub uuid: u128,
    pub name: Option<String>,
    pub properties: Vec<AccountProperty>,
    /// RemoteChatSession.Data: session id, key expiry, key, key signature.
    pub chat_session: Option<(u128, i64, Vec<u8>, Vec<u8>)>,
    pub game_mode: Option<i32>,
    pub listed: Option<bool>,
    pub latency: Option<i32>,
    pub display_name: Option<Option<Tag>>,
    pub list_order: Option<i32>,
    pub show_hat: Option<bool>,
}
/// Actions bits: ADD_PLAYER, INITIALIZE_CHAT, UPDATE_GAME_MODE, UPDATE_LISTED,
/// UPDATE_LATENCY, UPDATE_DISPLAY_NAME, UPDATE_LIST_ORDER, UPDATE_HAT.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfoUpdate {
    pub actions: u8,
    pub entries: Vec<PlayerInfo>,
}
impl Codec<PlayerInfoUpdate> for PlayerInfoUpdate {
    fn read(r: &mut Reader) -> Result<Self> {
        let actions = r.u8()?;
        let n = r.count(r.remaining())?;
        let mut entries = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let mut e = PlayerInfo {
                uuid: r.uuid()?,
                ..PlayerInfo::default()
            };
            if actions & 1 != 0 {
                e.name = Some(r.string(16)?);
                let count = r.count(16)?;
                e.properties = (0..count)
                    .map(|_| PropertyCodec::read(r))
                    .collect::<Result<_>>()?;
            }
            if actions & 2 != 0 && r.bool()? {
                e.chat_session =
                    Some((r.uuid()?, r.i64()?, r.byte_array(512)?, r.byte_array(4096)?));
            }
            if actions & 4 != 0 {
                e.game_mode = Some(r.varint()?);
            }
            if actions & 8 != 0 {
                e.listed = Some(r.bool()?);
            }
            if actions & 16 != 0 {
                e.latency = Some(r.varint()?);
            }
            if actions & 32 != 0 {
                e.display_name = Some(Opt::<TrustedComponent>::read(r)?);
            }
            if actions & 64 != 0 {
                e.list_order = Some(r.varint()?);
            }
            if actions & 128 != 0 {
                e.show_hat = Some(r.bool()?);
            }
            entries.push(e);
        }
        Ok(Self { actions, entries })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.u8(v.actions);
        w.varint(v.entries.len() as i32);
        for e in &v.entries {
            w.uuid(e.uuid);
            if v.actions & 1 != 0 {
                w.string(e.name.as_deref().unwrap_or(""));
                w.varint(e.properties.len() as i32);
                e.properties.iter().for_each(|p| PropertyCodec::write(p, w));
            }
            if v.actions & 2 != 0 {
                w.bool(e.chat_session.is_some());
                if let Some((id, expires, key, sig)) = &e.chat_session {
                    w.uuid(*id);
                    w.i64(*expires);
                    w.byte_array(key);
                    w.byte_array(sig);
                }
            }
            if v.actions & 4 != 0 {
                w.varint(e.game_mode.unwrap_or(0));
            }
            if v.actions & 8 != 0 {
                w.bool(e.listed.unwrap_or(false));
            }
            if v.actions & 16 != 0 {
                w.varint(e.latency.unwrap_or(0));
            }
            if v.actions & 32 != 0 {
                let name = e.display_name.clone().flatten();
                w.bool(name.is_some());
                if let Some(t) = &name {
                    w.nbt(t);
                }
            }
            if v.actions & 64 != 0 {
                w.varint(e.list_order.unwrap_or(0));
            }
            if v.actions & 128 != 0 {
                w.bool(e.show_hat.unwrap_or(false));
            }
        }
    }
}
packet!(LastSeenUpdate { offset: i32 => VarInt, acknowledged: [u8; 3] => Fixed3, checksum: i8 => Byte });
/// LastSeenMessages.Update's fixed 20-bit BitSet (3 bytes).
pub struct Fixed3;
impl Codec<[u8; 3]> for Fixed3 {
    fn read(r: &mut Reader) -> Result<[u8; 3]> {
        Ok(r.bytes(3)?.try_into().expect("3 bytes"))
    }
    fn write(v: &[u8; 3], w: &mut Writer) {
        w.bytes(v)
    }
}
packet!(Chat {
    message: String => StrN<256>,
    timestamp: i64 => Long,
    salt: i64 => Long,
    signature: Option<Vec<u8>> => Opt<Signature>,
    last_seen: LastSeenUpdate => LastSeenUpdate,
});
packet!(ChatAck { offset: i32 => VarInt });
/// ArgumentSignatures: each signed argument's name (at most 16 chars) and
/// signature, at most 8.
pub struct ArgumentSignatures;
impl Codec<Vec<(String, Vec<u8>)>> for ArgumentSignatures {
    fn read(r: &mut Reader) -> Result<Vec<(String, Vec<u8>)>> {
        let n = r.count(8)?;
        (0..n)
            .map(|_| Ok((r.string(16)?, r.bytes(256)?.to_vec())))
            .collect()
    }
    fn write(v: &Vec<(String, Vec<u8>)>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for (name, signature) in v {
            w.string(name);
            w.bytes(signature);
        }
    }
    fn validate(v: &Vec<(String, Vec<u8>)>) -> Result<()> {
        // ArgumentSignatures.STREAM_CODEC is a ByteBufCodecs.list(8),
        // with Entry.STREAM_CODEC using stringUtf8(16) and MessageSignature.
        if v.len() > 8 {
            return Err(crate::Error::new("more than 8 signed command arguments"));
        }
        for (name, signature) in v {
            StrN::<16>::validate(name)?;
            Signature::validate(signature)?;
        }
        Ok(())
    }
}
packet!(ChatCommandSigned {
    command: String => Str,
    timestamp: i64 => Long,
    salt: i64 => Long,
    arguments: Vec<(String, Vec<u8>)> => ArgumentSignatures,
    last_seen: LastSeenUpdate => LastSeenUpdate,
});
packet!(
    /// RemoteChatSession.Data: the session id, then ProfilePublicKey.Data
    /// (expiry in epoch milliseconds, X.509 key, the services' signature).
    ChatSessionUpdate {
        session: u128 => Uuid,
        expires_at: i64 => Long,
        key: Vec<u8> => BytesN<512>,
        key_signature: Vec<u8> => BytesN<4096>,
    }
);
