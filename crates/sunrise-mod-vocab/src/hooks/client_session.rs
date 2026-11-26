//! Read-only client session and local-player status declarations.

pub trait Notes {
    #[notes = r#"## Client session and local-player status

`ClientHost::client_session()` supplies read-only `client_server()`,
`client_language()`, `client_frame_stats()`, `client_titles()` and
`client_boss_bars()`. The first three return `None` outside a logged-in client
world; the title and boss-bar reads return `Ok(None)` there, and a text
conversion error for a text the API cannot carry, never plain words. These
reads use existing local state and send nothing.
`ClientServer<'_>` lends text from the host and keeps missing names and addresses
explicit: its `name` is the integrated world's current display name, or `None`
for an external connection (the game retains the player name, not a server
display name). Its `address` is absent for memory connections; friend-invite
secrets are redacted. `integrated_local` means this window actually owns that
integrated server, not that the address looks like localhost. `save_folder`
gives the owning window the integrated save's folder name as a key for local
data. It survives display-name changes and is scoped to the game instance. It
is absent for any other connection. All three text fields are `Option<&str>`;
use `.map(str::to_owned)` on a field only to keep it beyond the host borrow.

`client_language()` shares an `Arc<str>` snapshot of the applied language code,
not a pending options choice. Use `.as_deref()` to compare it with `Some("en_us")`.
A later language change leaves the snapshot intact.
Use the existing `ClientHost::is_paused()` for the applied pause state.
`ClientFrameStats` reports the game's most recently computed `fps`, lifetime
`rendered_frames` (menus included), and the newest completed frame's wall
interval as `frame_time_ns` (including waits). The interval is `None` until a
sample exists or after the chart is reset. Reading it starts no new recording.

`client_titles()` is the title, subtitle and action bar on screen now: each is
`None` once its timer has run out, and the subtitle shows only with a title.
`client_boss_bars()` is every boss bar the client holds, in server order,
each with its name and progress (0 to 1, as drawn). The HUD draws only those
that fit within `guiHeight / 3`.

`PlayerRef::block_breaking(&dyn ClientHost)` and `ambient_mood(&dyn ClientHost)`
only accept the current local player's handle. Stale, fake and nonlocal handles
return `None`, as does a disconnected client. `BlockBreakingState::Idle` is
instead a valid local player who is not breaking a block. `Breaking` gives the
target position, accumulated predicted progress and optional visible crack
stage (0 through 9; absent at zero progress). The stage uses the same
calculation as the world overlay. Mood is the existing cave ambience value,
normally 0 through 1, not a server-side measurement."#]
    fn client_server();
}
