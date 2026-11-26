//! Handshake state: ClientIntentionPacket.
use crate::{
    buf::{StrN, UShort, VarInt},
    packet, packet_enum,
};

/// ClientIntent ids.
pub const STATUS: i32 = 1;
pub const LOGIN: i32 = 2;
pub const TRANSFER: i32 = 3;

packet!(Intention {
    protocol: i32 => VarInt,
    host: String => StrN<255>,
    port: u16 => UShort,
    intent: i32 => VarInt,
});

packet_enum!(Serverbound, crate::ids::handshake::serverbound { Intention(Intention) = INTENTION });

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.

// End generated retained traversals.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intention_bytes() {
        let p = Serverbound::Intention(Intention {
            protocol: 777,
            host: "localhost".into(),
            port: 25565,
            intent: LOGIN,
        });
        let body = p.encode();
        assert_eq!(body[0], 0);
        assert_eq!(&body[1..3], &[0x89, 0x06]);
        assert_eq!(&body[body.len() - 3..], &[0x63, 0xdd, 2]);
        crate::packets::round_trip(p, Serverbound::encode, Serverbound::decode);
    }

    /// `ClientIntentionPacket` reads the host with `readUtf(255)`, and nothing may follow the intent.
    #[test]
    fn the_host_is_read_up_to_vanillas_255() {
        let encode = |host: String| {
            Serverbound::Intention(Intention {
                protocol: 777,
                host,
                port: 25565,
                intent: LOGIN,
            })
            .encode()
        };
        for host in [
            "a".repeat(255),
            format!("{}🚀", "a".repeat(253)),
            "entry.test\0suffix".into(),
        ] {
            let body = encode(host.clone());
            let Serverbound::Intention(decoded) =
                Serverbound::decode(i32::from(body[0]), &body[1..]).unwrap()
            else {
                panic!("expected intention");
            };
            assert_eq!(decoded.host, host);
        }
        for host in ["a".repeat(256), format!("{}🚀", "a".repeat(254))] {
            let body = encode(host);
            assert!(Serverbound::decode(i32::from(body[0]), &body[1..]).is_err());
        }
        let mut body = encode("localhost".into());
        body.push(0);
        assert!(Serverbound::decode(i32::from(body[0]), &body[1..]).is_err());
    }
}
