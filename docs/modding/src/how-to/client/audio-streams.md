# Audio streams

Play audio your native mod makes: a music player, a radio, a text reader or proximity voice chat. Give the mixer a native `AudioStream` source that produces interleaved signed 16-bit PCM on demand. See the [API reference](../../reference/api.md) for the generated signatures.

Streams use a pool separate from vanilla's streaming slots. They are not sound instances: nothing in `sounds.json` names them, and no sound hooks fire for them.

## Pull PCM on demand

Implement `AudioStream` to generate, decode or supply PCM as the mixer needs it. For example, this source plays an owned clip of mono samples at 48 kHz:

```rust
{{#include ../../../examples/src/audio_streams.rs:play_clip}}
```

The mixer owns the `Send + 'static` source. Keep `read` bounded and nonblocking: all pull streams on the output share a worker, so one slow source delays the others. Do not wait on I/O, locks, another thread or a tick. A source has no `Host` or world borrow and must not call back into the game. Use the preallocated buffer it lends you.

`pcm` is zeroed before each call. Write up to `pcm.len() / channels` whole interleaved frames into it and return the **frame** count, not the sample count:

- Returning `0` signals EOF: the mixer drains any already-queued frames, then stops the stream.
- Returning `1..=pcm.len() / channels` publishes that many whole frames. To play temporary silence without ending the stream (for example, while waiting on a network peer), return `n > 0` frames without writing to the zeroed `pcm` slice.
- Returning a count larger than `pcm.len() / channels` stops the stream immediately without publishing.

The worker drops an admitted source off the device callback at EOF or after stop, even if no further callback runs after you stop it. A panic in `read` immediately stops that stream and its source callback. The shared log and error toast name the mod; unrelated streams continue.

## Async and network producers

For audio that arrives independently of the mixer — such as proximity voice chat decoded on a `Network`-thread payload handler or a background decoder thread — own a bounded, nonblocking or lock-free ring buffer shared between your producer and your `AudioStream` implementation:

- Your producer writes decoded interleaved PCM into your mod-owned buffer without blocking the network or client thread, owning its own jitter buffering, overflow drop policy and gap handling.
- Your `AudioStream::read` drains available whole frames from that buffer into `pcm`. When no frames are ready yet, leave the zeroed `pcm` slice untouched and return `n > 0` frames of temporary silence rather than returning `0` (which ends the stream). Return `0` only when the stream is finished.

## Format and controls

`ClientHost::open_audio_stream` is available to the calling mod from client setup onward. It refuses with `AudioRefusal`, without opening anything, if the client has no sound output, the call is unavailable in this context, the spec is invalid, the pool reaches `AUDIO_STREAMS_PER_MOD` open or unretired streams for the calling mod or `AUDIO_STREAMS_TOTAL` across all mods, the mod's or the pool's PCM buffer budget is exhausted or ring allocation fails (`AudioRefusal::MemoryLimit`), or the pull worker cannot start or admit the source (`AudioRefusal::WorkerUnavailable`).

| `StreamSpec` field | Meaning |
|---|---|
| `category` | The mixer group, scaled by master times the group like every sound. Voice chat uses `SoundSource::Voice`, the “Voice/Speech” slider. |
| `channels` | 1 or 2. Both mono and stereo accept any placement; mono is spatialized and stereo plays un-spatialized. |
| `rate` | Any positive sample rate (`> 0` Hz); `0` is refused with `SpecFault::Rate`. The game resamples to the output rate. High-rate sources are downsampled in bounded reads on the worker. |
| `placement` | `Flat`, `At([x, y, z])` (any finite coordinates) or `Entity(entity)` (a currently tracked entity), in the client's current level. |
| `range` | Positive, finite distance in blocks at which a positioned mono stream fades to silence, in a straight line (`AL_LINEAR_DISTANCE`, as a sound's range). |
| `volume` | The stream's own volume before the group's and the master's. Finite values are clamped to `0.0..=1.0`; `NaN` is refused with `SpecFault::Volume`. |

Use the `AudioStreamRef` handle to set placement (`set_flat`, `set_position(at)`, `set_entity(host, entity)`), set volume (`set_volume`), read state (`state`), or stop playback (`stop`). **Dropping the handle stops the stream**, so keep it for as long as you want playback. A stopped stream frees its voice immediately and frees its quota slot and PCM buffer once the pull worker retires its job, even if you keep its handle. Muffling (`muffle_sounds`) applies to mod streams as it does to vanilla sounds.

- `Playing`: open and playing, even if silent because it is muted or has no PCM ready.
- `Stopped`: stopped explicitly, dropped, ended after `read` returned `0` and queued frames drained, stopped after an invalid frame count or panic, stopped because its followed entity became untracked or removed, stopped by leaving the level, re-entering configuration, changing dimensions, or sound engine reload/stop, or stopped when sound output was lost. It never plays again; open a new stream once playback can resume.

## The player's switch

With Sunrise Extras' “Mod audio streams” switch off, streams still open, advance in silence, and check tracked entity lifetime (so losing a tracked entity stops the stream even while muted). A mod needs no special case, and switching it back on does not play a backlog. The Extras switch is in `crates/sunrise-official/src/mod_audio.rs`; the engine only owns the gain it sets.

## Placement

- A **mono** stream may be `Flat` (at the listener, no panning and no fading), `At([x, y, z])` with any finite coordinates, or `Entity(entity)`. Positioned mono streams are panned and fade linearly to silence at `range`.
- A **stereo** stream accepts any valid placement (`Flat`, `At`, or `Entity`) and plays un-spatialized with no panning or distance attenuation, matching OpenAL. `At` coordinates are still validated to be finite and `Entity` must still be currently tracked.
- An `Entity` stream resolves its entity's position eagerly at `open_audio_stream` and at `AudioStreamRef::set_entity(host, entity)`. If the entity is not currently tracked, `open_audio_stream` refuses with `SpecFault::Position` and `set_entity` returns `false` without changing placement. Each frame the mixer follows the entity even while muted; if the entity becomes untracked, unloaded or removed, the stream stops.
- Leaving the level, re-entering configuration, changing dimensions, or reloading/stopping the sound engine stops all open streams. Level teardown also stops streams opened from its unload callbacks; open a new stream after the level transition if playback should continue.

## Network audio

`StreamPlacement`, `StreamSpec`, `AudioStreamState`, `AudioRefusal` and `SpecFault` are local values,
not [Wire](../../concepts/wire-types.md) types. For network audio, send a separate
[typed payload](networking.md#typed-payloads) with the data the receiver needs, such as the emitter's
server-assigned network entity ID and the encoded audio:

```rust
{{#include ../../../examples/src/audio_streams.rs:voice_frame}}
```

Put the emitter's `entity.id()` in the clientbound frame, not its `EntityRef`: the handle belongs to
its own world epoch. On the receiving client's main thread, use `host.entity(frame.emitter_id)` to
resolve a handle in the current level. If it returns `None`, skip the frame. Otherwise, use
`StreamPlacement::Entity(entity)` when opening the local stream or call
`stream.set_entity(host, entity)`. Scope the IDs and any cached handles
to the current level and connection; do not carry pending frames into a new level. A network-thread
handler can feed a mod-owned buffer read by an already-opened `AudioStream` on the pull worker and
can call `set_flat`, `set_position`, `set_volume`, `state` and `stop` directly without a host.
To change the followed entity from a network-thread producer, obtain a
[`ClientExecutor`](networking.md#moving-work-to-the-client-thread) in `client_setup` and clone it
into the producer. Move the owned stream handle (or its `Arc` clone) and the local `EntityRef`
into the task, not the payload context:

```rust,ignore
// In client_setup:
let Some(executor) = host.client_executor() else { return; };
let worker_executor = executor.clone();
// In the network-thread producer:
worker_executor.execute(Box::new(move |host| {
    if !stream.set_entity(host, entity) {
        eprintln!("Audio retarget refused: stream stopped or entity unavailable");
    }
}));
```

`set_entity` checks tracking when the task runs, after the packet phase. It returns `false`
if a packet removed the entity or moved the client to another level, even if that packet was
published during packet handling after the task was submitted. A successful `set_entity`,
`set_flat`, or `set_position` publishes the placement immediately. A later panic in the task does
not undo that placement.

## Threading

- **Client host:** Open streams (`open_audio_stream`) and attach a stream to an entity (`AudioStreamRef::set_entity(host, entity)`) where `&dyn ClientHost` is available. The client tracks entity placements and reaps stopped streams; it does not have to refill PCM on a tick.
- **Pull worker:** Calls each source's `read` serially without holding mixer or pool locks. The device callback requests PCM for the next buffer; if several requests arrive before the worker reads, the latest wins.
- **Any thread:** `AudioStreamRef` is `Send`: `set_flat`, `set_position`, `set_volume`, `state` and `stop` work from any thread (including a `Network`-thread payload handler) without a `ClientHost` and without taking the mixer's lock. Async producers write decoded PCM into their own nonblocking buffer read by `AudioStream::read`.
- **Device callback:** Drains the queued PCM and mixes it. It does not call your source or wait for a refill. If the source falls behind, the missing audio is silent.

## Microphone capture

List available input devices with `host.microphone_devices()` and open live PCM capture on a `ClientHost` from client setup onward with `host.open_microphone(&spec)`:

```rust
{{#include ../../../examples/src/audio_streams.rs:open_mic}}
```

Each entry returned by `host.microphone_devices()` is a `MicDevice { id, label }`. Show `device.label` in UI and pass `device.id` to `MicSpec::with_device` to target that endpoint. If it vanishes, opening its ID returns `MicRefusal::NoDevice`; refresh the list rather than substituting a device with the same label. Treat the ID as opaque: pass it through unchanged. To intentionally select by display label, use `MicSpec::with_label`. A label shared by multiple available inputs returns `MicRefusal::AmbiguousDevice`. If discovery fails before it can establish a unique match, opening by label returns that discovery error instead of choosing the first input it found. Use an ID to target a specific input. An inaccessible endpoint does not hide other available inputs when listing.

`MicRef` is `Send + Sync` and safe to read from any thread off the OS audio callback, with no tick or codec required. `mic.read(&mut pcm)` is nonblocking: it writes whole interleaved signed 16-bit PCM frames into the returned prefix `pcm[..frames * channels]`, leaves any remaining suffix untouched, and returns `0` without modifying `pcm` if no whole frames are ready or another reader is concurrently draining the same handle. Capture feeds a bounded 500 ms ring (`MIC_RING_MAX_MS`) that drops the oldest frames if the reader falls behind; `mic.dropped_frames()` reports the cumulative dropped frame count.

Call `mic.close()` or drop `MicRef` to stop capture and release the OS input stream synchronously before returning. `mic.is_open()` returns `false` after close or if the input device disconnects or errors (in which case the backend stream is released promptly without waiting for a later call). Handle `MicRefusal` directly:

- `OsDenied`: the operating system denied capture, blocked lookup of the selected input (for example, ALSA denied access to a card with a previously listed ID or label), or prevented listing any available inputs. On macOS the system can ask for microphone access before probing an input on the first open. Listing and opening a label that matches only outputs do not request access.
- `Busy`: the input device is busy or held in exclusive use.
- `NoDevice`: no default input device exists, the requested ID or label matched no input, or client audio is disabled.
- `AmbiguousDevice`: a device label matched more than one input device (pass `MicDevice::id` instead).
- `BadSpec(MicSpecFault)`: `channels` is not 1 or 2, or `rate` is outside 8000 to 48000 Hz (`MIC_RATE_MIN`..=`MIC_RATE_MAX`).
- `OnServer` / `Unavailable`: called on a server or outside client setup or later.
- `DeviceFailed`: input discovery failed, or the input device or capture worker failed to open or start.

`MicDevice`, `MicRef`, `MicSpec`, `MicTarget`, `MicSpecFault` and `MicRefusal` are client-local, not [Wire](../../concepts/wire-types.md) types. The engine never records or sends microphone audio; mods own their own echo cancellation, metering, encoding and networking.

## Known limits

- There is no echo cancellation: a mod that plays voice while capturing the microphone must deal with its own echo.
- Voice is unmoderated: the engine neither records, filters nor reports a stream's audio. Replay recordings capture voice payloads like any other payload.
- These APIs play and capture raw PCM. To send time-sensitive audio between players, use [droppable play payloads](networking.md#droppable-delivery-audio-live-data). Your mod brings its own codec as an ordinary dependency and owns its buffering and gap handling.
