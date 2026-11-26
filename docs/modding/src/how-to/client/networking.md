# Client networking

Call `host.disconnect(reason)` through `ClientHost` to leave the joined server or world at the end of the frame, outside hooks. Pass `Some(Text)` to show a disconnect reason or `None` for the normal leave flow. The disconnect screen renders the reason in the player's language, like other translatable text: a translatable component shows its translated text and a plain-text reason is unchanged. The call returns `Result<(), DisconnectError>`: `Unavailable` means no world is joined, it is closing, or a save is pending, and `Text` reports an invalid reason (a malformed or too-deep component is refused by that name and nothing changes). A pending request belongs to its current world epoch and cannot leave a later world. Server code continues to use the separate player disconnect operation.

## Moving work to the client thread

In `client_setup`, get the `ClientExecutor` and clone it into a network handler or another
worker. Handle `None`: restricted hosts such as the client-side `entity_entering_section`
host cannot issue an executor, and neither can a closing client or a client in which this mod's
executor already panicked.

```rust,ignore
// In client_setup:
let Some(executor) = host.client_executor() else { return; };
let worker_executor = executor.clone();
// In the worker, move owned data into the closure:
worker_executor.execute(Box::new(move |host| { /* use the client host here */ }));
```

The executor does not run the closure inline. Each client drain handles packets and queued
payload work first, then runs executor tasks in submission order with a live
`&mut dyn ClientHost`. The packet phase includes packets and queued payload work published
while earlier packets are being handled, even when a task was submitted first. If any packet
work is deferred, executor tasks wait for the packet phase to finish. When the client starts
quitting, including from the title screen, pending and new tasks are dropped. There is no extra
transaction around the task: effects already applied to the live client, such as audio placement,
are not undone on panic. The shared error log and toast name the panicking mod. Its executor is
disabled for the rest of the client session: already queued tasks for the same mod are skipped and
retained executors discard later submissions. Other mods' tasks still run. A phase refusal is
reported but does not disable the executor.

The executor belongs to the client application, not to a connection or level. Keep using
`IPayloadContext::enqueue_work` if the work needs the payload's original epoch, player, reply or
configuration controls. A `ClientExecutor` task receives no payload context. Check any retained
entity or level handle when the task runs; a disconnect or dimension change may have made it stale.

## Observing play packets

`client_packet` and `client_packet_send` lend each listener an immutable
`ClientPacket<'a>` view (`id: &'a Identifier`, `custom_payload: Option<ClientPayload<'a>>`
with `channel: &'a str` and `data: &'a [u8]`) valid only for the duration of the
callback. Neither hook exposes other decoded fields or mutable packet bytes. Copy
only what you need to retain after the callback returns (`event.packet.id.clone()`,
`payload.channel.to_owned()`, `payload.data.to_vec()`). Reassigning `event.packet`
inside a listener only changes that listener's local event struct; later listeners
and the game still observe the original borrowed packet.

Both hooks run client-side on any server (including single player's integrated
server), in packet order, and pay only a single subscriber check when no mod listens:

- `client_packet` fires for every decoded play-phase server packet before the
  window handles it, including custom payloads and each chunk packet from the
  batching path. An ID outside the protocol's packet table is reported as
  `sunrise:unknown`. Configuration and control packets do not fire it, standalone
  or bundled: keep-alives, chunk batch starts and finishes, bundle delimiters,
  resource pack pushes and pops, cookies, server links, transfers, disconnects, the
  configuration switch, `minecraft:brand`, and far terrain tiles. The receive hook
  cannot cancel a packet.
- `client_packet_send` fires for every play-phase packet the window is about to
  send, in send order, including the selected slot sent before an attack or use and
  play-phase pongs. Control replies (keep-alives, chunk batch acknowledgements,
  configuration pongs and cookies), configuration packets, phase-dependent connection
  answers (resource pack status and a dialog's custom click action), and far terrain
  requests do not fire it. Returning `Flow::Cancel` drops
  the packet unless a later listener marked `#[listener(receive_cancelled)]` sets
  `event.cancelled = false` and returns `Flow::Continue`.

A panic in either hook is logged and shown in an error toast naming the mod, and
disables only that mod's callback for that hook for the rest of the session (not the
other hook or other mods). Later eligible listeners still run on the original packet
view; a panicked send listener neither adds nor clears a cancellation, even if it
wrote `event.cancelled` before panicking.

## Holding outgoing play packets

Call `host.hold_packets()` to open a hold for your mod on the current physical play connection.
Store the issued `HoldId` locally; it has no `Wire` encoding. Only the issuing mod can end it.
Using another mod's live ID returns `HoldError::WrongOwner` and changes nothing.
Without an owning native mod, the call returns `NoOwner`.

A hold covers uncancelled engine play sends and mod packets whose root transactions commit
while it is active. Holds may overlap. There is one queue in original send order:

- `host.release_held(id)` ends your hold and publishes only the newly unblocked prefix.
  Its return value is the number queued to the writer, not the number delivered to the server.
  It can return `Ok(0)` if an older hold still blocks the queue.
- `host.drop_held(id)` ends your hold and discards the packets it covered, including overlapping
  packets. It leaves older packets and all other holds active and sends nothing.

For example, if A holds before packet 1 and B starts holding between packets 1 and 2, releasing
A sends packet 1; packet 2 still waits for B. Releasing B first sends nothing. Dropping B instead
would discard packet 2 without discarding packet 1 or ending A's hold. Neither operation replays
`client_packet_send` or exposes packet bodies.

Held packets retain ordinary transport credit. There is no separate hold limit and no automatic
flush. A `send_packet` can return `SendPacketError::Backpressure` if the physical writer has no
credit; an engine send hitting the writer's terminal pressure path still closes the connection.
If publication fails while releasing a prefix, `HoldError::Send { sent, error }` reports the number
already queued and the transport error; the rest of that prefix is discarded.

Disconnect, a protocol incarnation change, or an aborted dimension crossing discards the holds;
their IDs are then `Unknown`. Starting without a joined physical play connection returns `NoWorld`.
Keep-alives, chunk batch acknowledgements, configuration packets, cookies and resource-pack
answers bypass holds, standalone or bundled. Standalone cookies and transfers run on the main
thread. While the receive queue has room, the receiver answers standalone keep-alives and chunk
batches without waiting for queued bundle handlers. Inside a bundle, each handler runs inline on
the main thread in member order, including registered NETWORK payload handlers. Those handlers
cannot borrow a host directly; use queued work for that. In either case, `client_packet` hears
neither the batch's start nor its finish (it does hear the chunks between them), and
`client_packet_send` neither sees nor can cancel the acknowledgement.
No packet inspection or editing beyond the existing `ClientPacket` view is provided.

## Login handshakes

`player_handshake` runs on the server thread after the normal protocol and
transfer checks, but before the login hello, authentication, or admission.
There is no player or authenticated profile yet. Status pings don't fire it.

```rust,ignore
fn player_handshake(
    &self,
    _host: &mut dyn ServerHost,
    event: &mut PlayerHandshakeEvent<'_>,
) -> Flow {
    match event.requested_host.as_str() {
        "old.example" => *event.requested_host = "new.example".into(),
        "closed.example" => {
            event.cancelled = true;
            *event.reason = Some(Text::literal("This address is closed."));
        }
        _ => {}
    }
    Flow::Continue
}
```

The requested host is untrusted text, including any suffix after a NUL. Each
listener sees the last accepted rewrite. Login doesn't route by host; the
final value is what `PlayerRef::virtual_host` reports for the player. The vanilla limit of 255 UTF-16
code units applies to decoding the packet, not to the rewrite. Each listener
gets its own copy of the requested port, protocol version and peer IP, so no
listener changes what the next one reads. The peer IP is the numeric
transport address without a port, or `None` for an integrated server's
memory connection. A rewrite does not change the peer IP or bypass
authentication or admission.

Cancellation sends a login disconnect and closes the connection without
processing a login hello. The client gets the standard localized
"Disconnected" text if you leave `reason` unset or it fails Text serialization
validation.

## Reporting the client identity

Implement `Hooks::client_identity` to choose the brand and any extra configuration
payloads for a particular server. The hook runs on the network thread once per
join that reaches initial configuration. The read-only `ClientIdentityHost`
borrows the original target through `host.server_address()`: a typed address keeps
its original spelling, before SRV or DNS resolution, and a transfer supplies the
new destination. The Host exposes no game state or connection operations. An
integrated single-player connection sends only `vanilla`, without calling the hook.
Returning from play to configuration on the same connection does not repeat it.

```rust,ignore
use mod_api::{ClientIdentity, ClientIdentityEvent, ClientIdentityHost, ClientIdentityPayload, Identifier};

// In your Hooks implementation:
fn client_identity(
    &self,
    host: &dyn ClientIdentityHost,
    _: &mut ClientIdentityEvent,
) -> Option<ClientIdentity> {
    if host.server_address() != "example.org:25565" {
        return None;
    }
    let channel = Identifier::parse("example:hello").ok()?;
    Some(ClientIdentity::new(
        "example-client",
        vec![ClientIdentityPayload::new(channel, vec![1, 0])],
    ))
}
```

`None` leaves the decision to the next listener. The first `Some` (by listener
priority, then dependency order) replaces the whole identity, including any
extras. If nobody answers, the Sunrise layer's identity is used; if it has none,
the client sends `vanilla` with no extras. The brand is protocol data, not
display text. It goes on `minecraft:brand` after login acknowledgement, followed
by the extra payloads in list order, then client information. Supply an empty
list to send only the brand.

Use only channels that your server expects before negotiation. Reserved or
transport-owned channels, including `minecraft:brand`, cannot appear in the extra
list. Each extra body is sent as-is, with a limit of 32,767 bytes
(`ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`). The vanilla string limits
apply to the brand (`BrandPayload.write` / `FriendlyByteBuf.writeUtf`) and to
channel identifiers (`Identifier.STREAM_CODEC` / `ByteBufCodecs.STRING_UTF8`).
Rejected channels or encoding errors fail the join. The identity is not a Wire
record; use registered configuration payload handlers for negotiated traffic.
Play packet hooks and holds do not intercept these packets.

## Configuration tasks before play

The server's `register_configuration_tasks` hook (version 3) runs once per
connecting client, including vanilla clients, after the built-in registry, tag
and resource-pack steps. Use `event.connection.has_channel(host, &channel)`
to check what the peer negotiated before registering a task.

Implement `ConfigurationTask` for a `Send + 'static` struct and pass an instance to
`event.register(MyTask { id, ... })`. Return its namespaced type from
`task_type(&self) -> &Identifier`. The server reads it once as the registering mod;
it must be nonempty and unique across all listeners for that connection. `register`
returns `EmptyType` or `DuplicateType(id)` for an invalid type and `CallbackFailed(reason)`
if reading the type panics. The first error refuses admission and supplies the disconnect
diagnostic even if the listener ignores it. If a listener panics or hits a phase refusal in
the listener or in `task_type`, only that listener's staged tasks, reservations, diagnostic
and journaled writes are discarded; other listeners still configure the same peer, and only
a panicking registration callback is skipped on later peers. Rejected task
instances are dropped under the registering mod's panic policy, including when the implementation
was already disabled and `task_type` was skipped. Keep `Drop` non-panicking: a destructor panic
is logged and shown in an error toast with the mod's name; it does not replace the first diagnostic.

Implement `start` and, if needed, `tick`, both with `&mut self`, `host: &mut dyn ServerHost`
and `context: &mut ConfigurationTaskContext<'_>`. Keep per-connection state in the task
struct. Only the registered instance runs: once for `start`, then at most once per later
server tick while pending, in registration order and without holding the connection task lock.

| Task callback operation | Call |
| --- | --- |
| Check whether the peer negotiated a channel | `context.has_channel(host, &channel)` |
| Send a configuration payload | `context.send_payload(host, &channel, bytes)` |
| Complete the task | `context.finish()` |
| Abort admission with a reason | `context.fail(reason)` |

For delayed completion, call `cx.finish_configuration_task(&task_type)` in a
server configuration payload handler — on either the main thread or the network
thread — passing the `Identifier` returned by `ConfigurationTask::task_type`
(matching NeoForge's `ServerPayloadContext.finishCurrentTask(ConfigurationTask.Type)`
and vanilla's `ServerConfigurationPacketListenerImpl.finishCurrentTask`).
Completion always targets the receiving connection of that payload handler.
Use `cx.reply(&channel, bytes)` for a final reply on the same connection; any
staged replies publish before task completion advances the queue or sends
`FinishConfiguration`.

If the currently running task on this connection has a different type,
`finish_configuration_task` returns `WrongType`; if no task is currently running
(or this handler already requested completion), it returns `NotRunning`.
Calling on the client or outside the configuration phase returns `WrongContext`;
an ended connection or configuration epoch (or a pending disconnect) returns
`Disconnected`. A refusal never advances a task. Tasks, contexts, connection
capabilities, and task errors have no `Wire` encoding.

On the **main thread**, `finish_configuration_task` completes the matching
running task directly in the configuration session with no transaction-layer
staging and independently of nested child transactions; the server advances to
the next task or `FinishConfiguration` on the next tick, after the handler's
committed replies are queued. On the **network thread**, `finish_configuration_task`
validates and binds the currently running task turn and publishes completion
inside the same active-epoch gate after staged replies succeed; handler failure,
an explicit `disconnect`, or epoch invalidation discards that staged completion.
In task callbacks (`start`/`tick`), repeated `context.finish()` calls coalesce
and `fail` wins over `finish`, publishing once the callback succeeds and its
deferred sends commit.

A refused task payload send returns `Err(ConfigurationTaskError::Payload(_))`
without failing the task session, so the task can handle it, call
`context.fail(...)`, or finish. An explicit `fail` or callback panic aborts the
task callback's journaled writes and closes that client with the configuration-error reason.
A panic names the mod and task type in the log and shows an error toast if there is a local client.
That mod's task type is skipped for the rest of the server session, including for later peers.
Other task types still run. A returned refusal is reported but does not disable the task.
A send uses the live configuration connection, not the current task state: even after another handler completes the
task, sends can still queue while the connection is configuring. After the peer enters
Play, a saved `ConfigurationConnection` returns `Disconnected`, even for a channel
negotiated for Play. Check the channel
before sending to an optional or vanilla peer, then request completion or explain
why it cannot join.

There is **no per-task timeout**. A client answering keep-alives can remain in
configuration indefinitely while its task waits. The connection's ordinary idle
and keep-alive checks still apply.

## Typed payloads

A typed payload sends a value of your own type on a registered channel. Its body is the value's
[Wire](../../concepts/wire-types.md) encoding, with no extra envelope. Wire uses
little-endian numbers and lengths; it is not the vanilla packet codec. Use a raw channel
when another implementation requires a specific byte format.

Use owned string fields such as `String`, `Cow<'static, str>`, `Box<str>`, or `Arc<str>`, not static strings or interned level keys: the network codec never
interns incoming text. The registrar also refuses Wire forms that allocate outside the shared
budget, such as `Identifier`, `SpawnerKey`, `CustomSpawners`, JSON values and `Text`,
even in nested fields. Send bounded owned fields and convert explicitly where you need those types.

Derive `Wire` for the value, register it in `Hooks::register`, and keep the token the
registrar returns:

```rust,ignore
use mod_api::*;
use std::sync::OnceLock;

#[derive(Wire)]
struct Counter {
    counter: i32,
}

static REPLY: OnceLock<ToClient<Counter>> = OnceLock::new();

fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    // The server answers each counter with the next one.
    view.play_to_server_typed::<Counter>(PayloadId("example:count".into()), |value, cx| {
        let counter = value.counter.checked_add(1).ok_or("counter is too large")?;
        let next = Counter { counter };
        REPLY.get().unwrap().reply(cx, &next).map(drop).map_err(|e| format!("{e:?}"))
    });
    let reply = view.play_to_client_typed::<Counter>(
        PayloadId("example:count_reply".into()),
        |value, _| Ok(log::info!("server counted {}", value.counter)),
    );
    let _ = REPLY.set(reply);
}
```

- **Registration.** `play_to_server_typed`, `play_to_client_typed` and `play_bidirectional_typed`,
  and their `configuration_*` and `common_*` forms, sit beside the raw methods on the same
  registrar and take its version, `optional()`, `droppable()` and `executes_on(...)`. The handler receives the
  decoded value and the connection's `IPayloadContext`. A typed id has one declaration: the same
  id in another protocol refuses load.
- **Tokens.** Registration returns a `ToServer<T>` (client to server) or `ToClient<T>` (server to
  client), both for a bidirectional one. Keep it and send with it. A token grants nothing on its own: each send checks it.
- **Sending and replying.** `ToServer::send(host, &value)` sends from the client
  (`ClientHost::send_payload_to_server`), `ToClient::send(host, player, &value)` from the server
  (`PlayerRef::send_payload`), and `reply(cx, &value)` on either answers on the connection a
  handler received on. They return the raw sends' results: `Queued` means accepted locally.
  Sending the wrong direction does not compile. To address the single-player owner,
  use the ordinary player ref from the [players service](../entities/server-services-and-players.md).
- **Sending to a group.** A `ToClient<T>` also sends to a set of players without you building
  the set, each checking the token and encoding once, then sending the same bytes to each
  player and returning how many sends were accepted: `send_to_all(host, &value)` (every
  server player), `send_in_level(host, level, &value)`, `send_near(host, level, pos, radius,
  except, &value)` (players in the level, not `except`, strictly closer to `pos` than
  `radius`) and `send_tracking_chunk(host, chunk, &value)` (the chunk's watchers). They
  are NeoForge's `PacketDistributor` `sendToAllPlayers`, `sendToPlayersInDimension`,
  `sendToPlayersNear` and `sendToPlayersTrackingChunk`. A player whose send is refused (no
  channel, gone since the list was read, backpressure) is skipped and not counted. Token
  and encoding checks happen before any recipient lookup. `send_in_level` and `send_near`
  use the level's online-player visit and return `Ok(0)` for a stale or unavailable level.
  `send_tracking_chunk` uses `chunk.visit_watchers(host, &mut visit)` and returns `Ok(0)`
  for a stale chunk or no watchers, but `Err(PayloadSendError::Unavailable)` if the current
  phase or view cannot supply chunk watchers.
- **To the players tracking an entity.** `send_tracking_entity(host, entity, and_self, &value)`
  sends to the entity's trackers (`ChunkMap.TrackedEntity.seenBy`, NeoForge's
  `sendToPlayersTrackingEntity`; `entity.visit_trackers(host, &mut visit)` visits those
  same players in server order), and
  also to the entity itself when it is a player and `and_self` is set
  (`sendToPlayersTrackingEntityAndSelf`), each player once. A stale entity returns `Ok(0)`.
  If the tracker read is unavailable, the send returns `Err(PayloadSendError::Unavailable)`
  without looking up `and_self` or sending. A readable entity with no trackers can still be
  sent the payload itself if it is a player and `and_self` is set.
- **The one-call send underneath.** `PayloadsServerHost::players_send_payload(players, id, bytes)`
  queues the same bytes on each listed player's connection with one boundary call, where
  `player_send_payload` crosses once per player; the group sends above use it. Each connection's
  queue owns its own copy of the bytes. Every player is judged by `player_send_payload`'s rules on
  its own: a player whose send is refused is skipped and not counted, and the result is how many
  were accepted (0 for an empty list). A player listed twice is sent to twice. Like every send,
  it is published when the root transaction commits, and an abort publishes nothing.
- **Schema negotiation.** The derive gives the value type a schema fingerprint (field types in
  order, variant positions; not names), and it is part of the negotiated version. A peer whose
  type has another encoding is refused at join when the payload is required; when both sides
  declared it `optional()`, it is absent and a send returns `UnsupportedPeer`. A body that kept
  its encoding (for example, a record of `u8` and `bool`) still negotiates with the old codec.
  The fingerprint describes the negotiated encoding, not what a field or variant means.
  If you reorder fields or variants that have the same encoding, or reuse a position for a
  different meaning, bump the registrar's application version even if the fingerprint is unchanged.
- **Ask before sending.** On a client, `ClientHost::has_channel(&id)` (NeoForge's `hasChannel`) is
  the test `send_payload_to_server` makes before it queues anything, for the connection's current
  phase: true means the send won't return `UnsupportedPeer`, so a mod can tell a server that has
  its optional channel from one that doesn't. It is false with no connection and for a channel
  declared for the other phase or direction.
- **Named errors.** `PayloadSendError::StaleToken` for a token the registrar did not hand out
  as it claims (forged, from another load, read as another type, or its mod off);
  `EncodeFailed` for a value the peer could not decode (a bound or the allocation budget),
  before anything is queued; `WrongProtocol` for a send in a phase the id is not declared in. A
  malformed body for an enabled typed handler closes the session, naming the mod, without
  running the handler. A handler that panics is disabled for the session: the game logs the
  owning mod and shows an error toast. Work in the handler’s transaction journal or staged
  send queue is discarded. Later packets for that callback still pass framing and channel
  checks, but are skipped before reserving callback or main-thread queue space. Other handlers
  and the rest of the bundle still run. The engine releases its `PayloadWork` reference under
  that same registration after execution, refusal, or cancellation. A destructor panic is
  reported under its owner and discards effects staged by the affected delivery.
- **Raw channel failures stay local.** A raw channel (`play_raw`) carries bytes from any client,
  so a declaring mod's handler that returns `Failed` does not close the session: the game logs the mod and the channel, rolls back that delivery's staged
  work (a staged disconnect included), and still delivers the other declaring mods' copies. A
  handler that succeeds and asks to disconnect still disconnects, and so does a refusal by the
  main thread's work queue. A typed payload's failure closes the session as above.

## Syncing remote item cooldowns

Vanilla sends an item cooldown to the affected player, not to players tracking
that player. To display a remote shield's cooldown, register a clientbound play
payload on both ends and have the server send the authoritative state. For
example, read `PlayerRef::item_cooldowns(host)` after the change has committed;
`player_shield_disable` is a pre-cooldown notification, not proof that it started.
A subsequent `player_item_cooldown` listener can still veto the start.

Send changes (including an early end) with `ToClient::send_tracking_entity` and
send the current snapshot directly to each new observer in
`player_start_tracking`. Key the client state by the holder's entity ID in the
current level, so [held-item tinting](model-drawing.md#third-person-held-items)
can look it up. Your mod owns expiry, interpolation and cleanup when an entity
stops being tracked or when the client leaves the level or session. Without an
authoritative snapshot or update, the remote cooldown is unknown; leave the
model's tint unchanged rather than guessing from its pose or combat animation.

## Droppable delivery: audio, live data

A payload channel is `Reliable` by default: a queued send is delivered, in order, or the connection
fails. Declare `registrar.droppable()` for data that is worthless when late (voice, a live
position). The packets are the same vanilla `minecraft:custom_payload`; only what happens to a send
that cannot go out yet changes.

**Use it when** a newer message replaces an older one, or a gap is better than a delay. **Don't** for
anything a handler counts on (inventories, state changes): keep those `Reliable`.

**Rules.**
- Play channels only (a configuration or common declaration refuses load). The delivery is part of
  the negotiated version: a peer declaring the other delivery is refused like another version.
- A send waits in the engine's own queue, before framing, compression and encryption, and moves to
  the connection when its writer has room. It is dropped, and counted, when it has waited more
  than 250 ms, when more than 64 KiB wait (the oldest goes), at the switch from configuration to
  play, and when it is over the byte budget. A packet that was handed to the connection is never
  taken back, and a `Droppable` payload is never put in a bundle. The send still returns `Queued`:
  that was never a delivery acknowledgement.
- **Budgets** per connection: 256 KiB/s clientbound and 16 KiB/s serverbound by default, with one
  second of burst. The server's operator raises or lowers them with `droppable_clientbound` and
  `droppable_serverbound` under `[engine]` in `server.toml` ([hosting](https://github.com/sunrise-org/sunrise/blob/main/docs/hosting.md#droppable-mod-payloads-droppable_clientbound-and-droppable_serverbound)); 0 turns
  `Droppable` delivery off in that direction. A client sending over the serverbound budget loses those
  sends, and the server counts them as budget drops; nobody is kicked and `Reliable` traffic is never
  dropped. Over budget, stale sends go first, oldest first, then the new send.
- **Counts:** `payload_drop_counts(&id)` reads the drops of a channel by cause (`stale`,
  `watermark`, `budget`, `switch`) over this game's connections.
- **Ordering:** per channel within one sending thread. A `Network`-thread `send_to` and a
  main-thread send on one channel have no order relative to each other. Vanilla packets are never
  reordered by this.
- **Group send from a network handler:** `IPayloadContext::send_to(players, id, bytes)` on the server
  for `Droppable` channels (others refuse with `NotDroppable`), so a relay forwards a frame without
  waiting for the main tick. `players` is your own snapshot: a `PlayerRef` is a plain value, so keep a
  list in the handler's state behind a lock and replace it from a tick or join hook on the main
  thread. A player who left, or whose connection is not in play, is skipped. A network-thread send is published at once, not with the handler's staged replies.
- **Vanilla's packet-rate limit** (`rate-limit`, off by default) counts every packet, `Droppable`
  payloads included. Voice at 20 ms frames is 50 payloads a second: a mod may pack two frames into
  one payload.
- **Known limit:** the game connection is TCP. Under packet loss a lost segment stalls everything
  behind it (head-of-line blocking), including chunks and `Droppable` payloads already on the wire; the
  watermark and the 250 ms limit bound what piles up behind it, not what is stuck in it. The
  engine adds no loss concealment: carry a sequence number in your payload and let your codec
  conceal gaps.

## Sending vanilla play packets

`ClientHost::send_packet` submits an owned `ServerboundPlayPacket`. The game validates the
body and reserves outgoing credit. Publication waits for the current root transaction to
commit; rollback, disconnect or a protocol change cancels it. Submission applies no local
prediction or inventory effects, and the server still checks reach, permissions and state.

Build common packet bodies with these portable associated constructors. Each returns
`Result<ServerboundPlayPacket, PacketBuildError>`:

| Constructor | Values |
| --- | --- |
| `use_item` | hand (0 main, 1 off), sequence, yaw, pitch |
| `use_item_on` | hand, block position, face (0 down, 1 up, 2 north, 3 south, 4 west, 5 east), relative cursor, inside, world border, sequence |
| `player_action` | action, block position, direction, sequence |
| `interact` | entity network ID, hand, hit location, sneaking |
| `attack` | entity network ID; vanilla 26.3 uses a separate attack packet |
| `pick_item_from_block` | block position, include data |
| `pick_item_from_entity` | entity network ID, include data |
| `sign_update` | block position, front side, four lines |
| `move_player_pos` | position, on ground, horizontal collision |
| `move_player_pos_rot` | position, yaw, pitch, on ground, horizontal collision |
| `move_player_rot` | yaw, pitch, on ground, horizontal collision |
| `move_player_status_only` | on ground, horizontal collision |

Player actions follow vanilla 26.3: 0 start destroy, 1 change destroy direction, 2 abort,
3 stop destroy, 4 drop all, 5 drop one, 6 release use, 7 swap offhand, 8 stab.
Entity IDs are network IDs, not Sunrise entity handles. Angles are degrees. Sequence
numbers and the corresponding client state remain the caller's responsibility.

The builders refuse non-finite positions, cursors and angles, unknown action/hand/direction
values, unrepresentable packed block positions and sign lines longer than 384 UTF-16 code
units. `PacketBuildError` identifies the packet and field. Interaction locations use vanilla's
lossy compressed-vector codec, including its clamping of large finite values.

`ClientHost::container_click(&ContainerClickRequest)` builds a container click from plain
carried/changed-slot `ItemStack`s. It needs the host because the packet carries hashed stacks.
The host uses the connection's item/component registry and the game's existing component
codecs and hashes. Submit the returned packet with `send_packet`; construction applies no
local menu effects. The request contains container/state IDs, slot, button, click type,
changed slots and carried stack. Click types are 0 pickup, 1 quick move, 2 swap, 3 clone,
4 throw, 5 quick craft and 6 pickup all.

Container limits are signed 32-bit container/state IDs, signed 16-bit slots, a signed-byte
button, at most 128 distinct changed slots, and the hashed component codec's count bounds.
The host refuses components it cannot encode or hash rather than silently omitting them.
No menu, reach, state or editor checks are replicated. Without a client connection, construction
returns `PacketBuildError::Unavailable`. `validate_container_click_limits` is also available
for preflighting the ID, slot and changed-slot-count wire limits.

For chat and commands, call `send_chat`; the game owns signing. There are no typed builders
for chat, signed commands, chat sessions, login/configuration, keep-alive, pong or custom
payload packets. Registered and engine-reserved custom-payload channels use their payload
or service API.
