//! Typed payloads over the registered raw transport: registration that hands out a token bound
//! to the declaration it made, and sends that encode a value and go through the raw sends. The
//! body on the wire is exactly `encode_payload(&value)`; there is no envelope and no second route.
use super::{
    IPayloadContext, IPayloadHandler, PayloadAccepted, PayloadBytes, PayloadDeclaration,
    PayloadDirection, PayloadHandleError, PayloadId, PayloadProtocol, PayloadRegistrar,
    PayloadSendError,
};
use crate::wire::{Reader, Wire, WireError, WireSchema, Writer};
use crate::{
    ChunkRef, ClientHost, EntityReadError, EntityRef, LevelReadError, LevelRef,
    PAYLOAD_DECODE_BUDGET, PlayerRef, ServerHost, decode_payload, encode_payload, payload_schema,
};
use core::ops::ControlFlow;
use std::{marker::PhantomData, sync::Arc};

/// What a typed registration claims: the declaration's slot in this load's frozen table, the
/// load, its protocol, the one direction the token sends in and its value type's schema
/// fingerprint. Constructing or decoding one grants nothing: every send checks it against the
/// frozen table and refuses a token the registrar did not hand out with `StaleToken`.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: a claim is plain data; the frozen table, not its constructor, decides what it may send.
pub struct PayloadToken {
    /// The declared channel.
    pub id: PayloadId,
    /// The declaration's index in its load's typed table.
    pub slot: u32,
    /// The load that registered it; 0 for a registration that was refused.
    pub load: u64,
    /// The declared protocols.
    pub protocol: PayloadProtocol,
    /// The direction this token sends in: `ToServer` or `ToClient`, never `Bidirectional`.
    pub direction: PayloadDirection,
    /// The value type's schema fingerprint.
    pub schema: u64,
}

impl PayloadToken {
    /// A claim on the declaration at `slot` of load `load`. The registrar builds the ones it
    /// hands out; any other is refused at the first send.
    pub fn new(
        id: PayloadId,
        slot: u32,
        load: u64,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        schema: u64,
    ) -> Self {
        Self {
            id,
            slot,
            load,
            protocol,
            direction,
            schema,
        }
    }
    /// `Ok` when `value_schema`, the schema of the value type the token is held as, is the one
    /// it was handed out for and `direction` its own: two compares, no schema walk.
    fn claims(
        &self,
        value_schema: u64,
        direction: PayloadDirection,
    ) -> Result<(), PayloadSendError> {
        if self.direction == direction && self.schema == value_schema {
            Ok(())
        } else {
            Err(PayloadSendError::StaleToken(self.id.clone()))
        }
    }
    /// The checked body of `value`: what the peer's decoder accepts, or nothing is queued.
    fn body<T: Wire>(&self, value: &T) -> Result<PayloadBytes, PayloadSendError> {
        encode_payload(value, PAYLOAD_DECODE_BUDGET)
            .map(PayloadBytes)
            .map_err(|error| {
                PayloadSendError::EncodeFailed(format!("typed payload {}: {error:?}", self.id.0))
            })
    }
}

macro_rules! token {
    ($name:ident, $direction:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Keep the token from registration and use it to send payloads. Each send checks the
        /// token against the frozen table before encoding.
        // exhaustive: its fields are private; the registrar is the only constructor.
        // no wire: a manual codec below, the token's own; the value type is only a parameter.
        pub struct $name<T> {
            token: PayloadToken,
            /// `T`'s schema, walked once when the token is made or read as this type.
            value_schema: u64,
            value: PhantomData<fn() -> T>,
        }
        impl<T> $name<T> {
            fn new(token: PayloadToken, value_schema: u64) -> Self {
                Self {
                    token,
                    value_schema,
                    value: PhantomData,
                }
            }
            /// The claim this token makes.
            pub fn token(&self) -> &PayloadToken {
                &self.token
            }
        }
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                Self {
                    token: self.token.clone(),
                    value_schema: self.value_schema,
                    value: PhantomData,
                }
            }
        }
        impl<T> std::fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.token).finish()
            }
        }
        impl<T: Wire> $name<T> {
            /// Queue `value` as a reply on the connection `context` received on, as
            /// [`IPayloadContext::reply`] does, after the same token check as a send.
            pub fn reply(
                &self,
                context: &mut dyn IPayloadContext,
                value: &T,
            ) -> Result<PayloadAccepted, PayloadSendError> {
                self.token
                    .claims(self.value_schema, PayloadDirection::$direction)?;
                context.check_payload_token(&self.token)?;
                let body = self.token.body(value)?;
                context.reply(&self.token.id, body)
            }
        }
        /// The token's own claim; the value type is only its parameter. Reading one as `T`
        /// (the only way to retype a token) walks `T`'s schema once.
        impl<T: Wire + 'static> Wire for $name<T> {
            crate::wire::layout_fields!("token" => PayloadToken);
            const MIN: usize = <PayloadToken as Wire>::MIN;
            fn put(&self, out: &mut Writer) {
                self.token.put(out);
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                PayloadToken::take(input).map(|token| Self::new(token, payload_schema::<T>()))
            }
        }
    };
}
token!(
    ToServer,
    ToServer,
    "A typed serverbound payload: the token [`PayloadRegistrar`]'s `*_to_server_typed` and `*_bidirectional_typed` return."
);
token!(
    ToClient,
    ToClient,
    "A typed clientbound payload: the token [`PayloadRegistrar`]'s `*_to_client_typed` and `*_bidirectional_typed` return."
);

impl<T: Wire> ToServer<T> {
    /// Queue `value` to the connected server through
    /// [`ClientHost::send_payload_to_server`], with its results, after checking the token:
    /// `StaleToken` for one the registrar did not hand out for this type.
    pub fn send(
        &self,
        host: &mut dyn ClientHost,
        value: &T,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        self.token
            .claims(self.value_schema, PayloadDirection::ToServer)?;
        host.check_payload_token(&self.token)?;
        let body = self.token.body(value)?;
        host.send_payload_to_server(&self.token.id, body)
    }
}

impl<T: Wire> ToClient<T> {
    /// Queue `value` to `player` through [`PlayerRef::send_payload`], with its results, after
    /// checking the token: `StaleToken` for one the registrar did not hand out for this type.
    ///
    /// A serverbound token has no `send` to a player, so the wrong direction does not compile:
    /// ```compile_fail
    /// # use mod_api::*;
    /// fn wrong(token: &ToServer<i32>, host: &mut dyn ServerHost, player: PlayerRef) {
    ///     let _ = token.send(host, player, &7);
    /// }
    /// ```
    pub fn send(
        &self,
        host: &mut dyn ServerHost,
        player: PlayerRef,
        value: &T,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        let body = self.encode(host, value)?;
        player.send_payload(host, &self.token.id, body)
    }

    /// Send `value` to every player on the server (`host.players().server_players()`), like
    /// NeoForge's `PacketDistributor.sendToAllPlayers`. Returns how many sends were accepted.
    ///
    /// The token is checked and `value` encoded once, before anything is sent: a stale token or
    /// an encode failure is the `Err`. The same bytes then go to each player through
    /// [`send`](Self::send)'s route; a player whose send is refused (no channel, gone since the
    /// list was read, backpressure) is skipped and not counted, never an error.
    pub fn send_to_all(
        &self,
        host: &mut dyn ServerHost,
        value: &T,
    ) -> Result<u32, PayloadSendError> {
        let body = self.encode(host, value)?;
        let players = host.players().server_players();
        Ok(self.deliver(host, players, body))
    }

    /// Send `value` to the players visited by [`LevelRef::visit_online_players`], like
    /// NeoForge's `PacketDistributor.sendToPlayersInDimension`. Returns how many sends were
    /// accepted. Encoding runs first; after a successful encode, an empty visit or a
    /// stale or unavailable level returns `Ok(0)`. Recipients are collected with a
    /// shared host read before taking mutable host access to queue the sends.
    ///
    /// Encoding and per-player refusals follow [`send_to_all`](Self::send_to_all).
    pub fn send_in_level(
        &self,
        host: &mut dyn ServerHost,
        level: LevelRef,
        value: &T,
    ) -> Result<u32, PayloadSendError> {
        let body = self.encode(host, value)?;
        let mut players = Vec::new();
        if level
            .visit_online_players(host, &mut |player| {
                players.push(player);
                ControlFlow::Continue(())
            })
            .is_err()
        {
            return Ok(0);
        }
        Ok(self.deliver(host, players, body))
    }

    /// Send `value` to the players in `level`, other than `except`, whose squared distance from
    /// `pos` is below `radius` squared, like NeoForge's `PacketDistributor.sendToPlayersNear`
    /// (vanilla's `PlayerList.broadcast`). A player on the radius itself is not reached. Returns
    /// how many sends were accepted. After a successful encode, no matches or a stale
    /// or unavailable level returns `Ok(0)`. The distance checks are shared reads during
    /// the player visit; sends start after the visit has finished.
    ///
    /// Encoding and per-player refusals follow [`send_to_all`](Self::send_to_all).
    pub fn send_near(
        &self,
        host: &mut dyn ServerHost,
        level: LevelRef,
        pos: [f64; 3],
        radius: f64,
        except: Option<PlayerRef>,
        value: &T,
    ) -> Result<u32, PayloadSendError> {
        let body = self.encode(host, value)?;
        let mut near = Vec::new();
        let radius_squared = radius * radius;
        if level
            .visit_online_players(host, &mut |player| {
                if Some(player) != except
                    && player.entity().position(host).is_some_and(|at| {
                        let [dx, dy, dz] = [at[0] - pos[0], at[1] - pos[1], at[2] - pos[2]];
                        dx * dx + dy * dy + dz * dz < radius_squared
                    })
                {
                    near.push(player);
                }
                ControlFlow::Continue(())
            })
            .is_err()
        {
            return Ok(0);
        }
        Ok(self.deliver(host, near, body))
    }

    /// Send `value` to the players watching `chunk` ([`ChunkRef::visit_watchers`]), like NeoForge's
    /// `PacketDistributor.sendToPlayersTrackingChunk`. The list is the engine's, never a
    /// distance test. Returns how many sends were accepted; a stale chunk handle or an empty
    /// watcher set returns `Ok(0)`. If the watcher read is unavailable, returns
    /// [`PayloadSendError::Unavailable`] without sending.
    ///
    /// Encoding and per-player refusals follow [`send_to_all`](Self::send_to_all).
    pub fn send_tracking_chunk(
        &self,
        host: &mut dyn ServerHost,
        chunk: ChunkRef,
        value: &T,
    ) -> Result<u32, PayloadSendError> {
        let body = self.encode(host, value)?;
        let mut watchers = Vec::new();
        match chunk.visit_watchers(host, &mut |player| {
            watchers.push(player);
            ControlFlow::Continue(())
        }) {
            Ok(_) => (),
            Err(LevelReadError::Stale) => return Ok(0),
            Err(LevelReadError::Unavailable) => return Err(PayloadSendError::Unavailable),
        }
        Ok(self.deliver(host, watchers, body))
    }

    /// Send `value` to the players tracking `entity` ([`EntityRef::visit_trackers`]), like NeoForge's
    /// `PacketDistributor.sendToPlayersTrackingEntity`; with `and_self`, also to the entity
    /// itself when it is a player (`sendToPlayersTrackingEntityAndSelf`). Returns how many sends
    /// were accepted. A stale entity returns `Ok(0)`; an unavailable tracker read returns
    /// [`PayloadSendError::Unavailable`] without looking up `and_self` or sending.
    /// A readable entity with no trackers can still receive the payload with `and_self`.
    ///
    /// Encoding and per-player refusals follow [`send_to_all`](Self::send_to_all).
    pub fn send_tracking_entity(
        &self,
        host: &mut dyn ServerHost,
        entity: EntityRef,
        and_self: bool,
        value: &T,
    ) -> Result<u32, PayloadSendError> {
        let body = self.encode(host, value)?;
        let mut players = Vec::new();
        match entity.visit_trackers(host, &mut |player| {
            players.push(player);
            ControlFlow::Continue(())
        }) {
            Ok(_) => (),
            Err(EntityReadError::Stale) => return Ok(0),
            Err(EntityReadError::Unavailable) => return Err(PayloadSendError::Unavailable),
        }
        if and_self {
            let own = host
                .players()
                .server_players()
                .into_iter()
                .find(|player| player.entity() == entity);
            if let Some(own) = own.filter(|player| !players.contains(player)) {
                players.push(own);
            }
        }
        Ok(self.deliver(host, players, body))
    }

    /// The up-front half of every group send: the token's claims and the host's check, then the
    /// one encoding of `value`.
    fn encode(&self, host: &dyn ServerHost, value: &T) -> Result<PayloadBytes, PayloadSendError> {
        self.token
            .claims(self.value_schema, PayloadDirection::ToClient)?;
        host.check_payload_token(&self.token)?;
        self.token.body(value)
    }

    /// Send the encoded `body` to `players` in one host call, which counts the accepted sends.
    /// A refused send (including a player no longer online) is skipped quietly.
    fn deliver(
        &self,
        host: &mut dyn ServerHost,
        players: Vec<PlayerRef>,
        body: PayloadBytes,
    ) -> u32 {
        host.players_send_payload(&players, &self.token.id, body)
    }
}

/// The receiving side of a typed declaration: decode the whole body as one `T`, then call the
/// mod's handler. A body that is not a `T` is the session's named refusal, and the handler does
/// not run.
struct Typed<T, F> {
    id: PayloadId,
    handler: F,
    value: PhantomData<fn() -> T>,
}

impl<T, F> IPayloadHandler for Typed<T, F>
where
    T: Wire + 'static,
    F: Fn(T, &mut dyn IPayloadContext) -> Result<(), String> + Send + Sync + 'static,
{
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        let value = decode_payload::<T>(&bytes.0).map_err(|error| {
            PayloadHandleError::Failed(format!(
                "typed payload {} refused its body: {error:?}",
                self.id.0
            ))
        })?;
        (self.handler)(value, context).map_err(PayloadHandleError::Failed)
    }
}

fn typed<T, F>(id: &PayloadId, handler: F) -> Arc<dyn IPayloadHandler>
where
    T: Wire + 'static,
    F: Fn(T, &mut dyn IPayloadContext) -> Result<(), String> + Send + Sync + 'static,
{
    Arc::new(Typed {
        id: id.clone(),
        handler,
        value: PhantomData::<fn() -> T>,
    })
}

impl PayloadRegistrar<'_> {
    fn register_typed<T: Wire + 'static>(
        &mut self,
        id: PayloadId,
        protocol: PayloadProtocol,
        direction: PayloadDirection,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) -> PayloadToken {
        let declaration = PayloadDeclaration {
            id,
            version: self.version.clone(),
            protocol,
            direction,
            optional: self.optional,
            thread: self.thread,
            delivery: self.delivery,
        };
        let mut schema = WireSchema::default();
        let fingerprint = T::describe(&mut schema);
        self.registrar.borrow_mut().add_typed_payload(
            declaration,
            schema.admits_payload().then_some(fingerprint),
            server,
            client,
        )
    }
}

fn sending(token: &PayloadToken, direction: PayloadDirection) -> PayloadToken {
    PayloadToken {
        direction,
        ..token.clone()
    }
}

macro_rules! typed_methods {
    ($to_client:ident, $to_server:ident, $both:ident, $protocol:ident) => {
        impl PayloadRegistrar<'_> {
            /// Register a typed clientbound payload with its client handler, which receives
            /// each body decoded as one `T`. The declaration is an ordinary raw one, with
            /// `T`'s schema fingerprint after this view's version.
            pub fn $to_client<T: Wire + 'static>(
                &mut self,
                id: PayloadId,
                client: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> ToClient<T> {
                let client = typed(&id, client);
                let token = self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToClient,
                    None,
                    Some(client),
                );
                let schema = token.schema;
                ToClient::new(token, schema)
            }
            /// Register a typed serverbound payload with its server handler, which receives
            /// each body decoded as one `T`, as the clientbound form does. `T` is the only
            /// generic, so it can be named:
            ///
            /// ```
            /// # use mod_api::*;
            /// fn register(r: &mut dyn Registrar) -> ToServer<i32> {
            ///     r.payload_handlers()
            ///         .registrar("1")
            ///         .play_to_server_typed::<i32>(
            ///             PayloadId("example:counter".into()),
            ///             |value, _| if value >= 0 { Ok(()) } else { Err("negative".into()) },
            ///         )
            /// }
            /// ```
            pub fn $to_server<T: Wire + 'static>(
                &mut self,
                id: PayloadId,
                server: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> ToServer<T> {
                let server = typed(&id, server);
                let token = self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::ToServer,
                    Some(server),
                    None,
                );
                let schema = token.schema;
                ToServer::new(token, schema)
            }
            /// Register a typed payload in both directions, with both handlers: one
            /// declaration, and a token for each direction.
            pub fn $both<T: Wire + 'static>(
                &mut self,
                id: PayloadId,
                server: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
                client: impl Fn(T, &mut dyn IPayloadContext) -> Result<(), String>
                + Send
                + Sync
                + 'static,
            ) -> (ToServer<T>, ToClient<T>) {
                let server = typed(&id, server);
                let client = typed(&id, client);
                let token = self.register_typed::<T>(
                    id,
                    PayloadProtocol::$protocol,
                    PayloadDirection::Bidirectional,
                    Some(server),
                    Some(client),
                );
                (
                    ToServer::new(sending(&token, PayloadDirection::ToServer), token.schema),
                    ToClient::new(sending(&token, PayloadDirection::ToClient), token.schema),
                )
            }
        }
    };
}
typed_methods!(
    play_to_client_typed,
    play_to_server_typed,
    play_bidirectional_typed,
    Play
);
typed_methods!(
    configuration_to_client_typed,
    configuration_to_server_typed,
    configuration_bidirectional_typed,
    Configuration
);
typed_methods!(
    common_to_client_typed,
    common_to_server_typed,
    common_bidirectional_typed,
    Common
);
