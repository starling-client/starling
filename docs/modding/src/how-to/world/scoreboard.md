# Read and write the scoreboard

A server mod reaches the world's scoreboard through objective handles, as it reaches a custom
boss bar. `host.objective(name)` finds an objective, `host.visit_objectives(&mut visit)`
visits their handles, `host.add_objective(name, criteria, display_name, render_type)` adds one, and
`display_slot_objective` and `set_display_slot` read and change what each display slot shows.
The handle, an `ObjectiveRef`, reads and writes the rest: the criteria, display name, render
type, number format, display slots and the scores. An objective's `name(host)` and
`criteria(host)` and a team's `name(host)` borrow `&str` from the board. Use
`.map(str::to_owned)` only if the text needs to outlive the host borrow. `DisplaySlot::name()`
returns the slot's static name.

```rust,ignore
let kills = host.add_objective("kills", "dummy", Text::literal("Kills"), ObjectiveRenderType::Integer)?;
kills.add_score(host, ScoreHolder::Player(player), 1)?;
host.set_display_slot(DisplaySlot::Sidebar, Some(kills));
```

The saved board is loaded before the server-started hooks run, so a mod that reads at startup
finds its saved objectives.

## Same board, same write

A mod's write is the write `/scoreboard` makes on the server's board. Clients are sent the
change by the board's usual per-tick sync, `scoreboard.dat` holds it, and selectors
(`scores=`, `team=`) see it. A criteria that follows players (`deathCount`, statistics) keeps
updating as before. A score is sent to clients only while its objective is shown in a display
slot. Locks are never sent.

## Show one player a sidebar

`player.set_sidebar(host, Some(sidebar))` replaces a personal sidebar; pass `None` to clear it.
Unlike `host.set_display_slot` on the shared board, this changes only that player's client.
It creates no saved or command-visible objective. Send it again after the player reconnects.

```rust,ignore
use mod_api::{Sidebar, SidebarLine, Text};

player.set_sidebar(host, Some(Sidebar {
    title: Text::literal("Round stats"),
    lines: vec![
        SidebarLine { text: Text::literal("Points"), value: Some(Text::literal("3 pts")) },
        SidebarLine { text: Text::literal("Round complete"), value: None },
    ],
}))?;
```

The client shows the first 15 lines, top first, with `value` in place of each numeric score.
A line with no value leaves that space blank. The personal sidebar takes the ordinary slot
and the slot for the player's current team color, if any. The client still prefers its team
color's slot. A later world packet setting or clearing the ordinary slot takes that slot back;
score changes and writes to other slots do not. Clearing restores the world's team-color
assignments and restores the ordinary assignment only if the personal sidebar still held it.

A stale or offline player gives `Ok(false)` without checking the text. Otherwise the title
and both text fields of each retained line must fit the trusted scoreboard codecs, which
keep the NBT depth and modified-UTF-8 string limits but have no NBT heap quota. Refused text
leaves the previous sidebar unchanged. In a transaction, accepted changes wait for commit.

## Holders

A score holder is a `ScoreHolder`:

- `Player(player)` is an online player, keyed by their name.
- `Entity(entity)` is any entity. A player's entity is that player (the same key as
  `Player`); another entity is keyed by its UUID.
- `Name(string)` is any string, as vanilla's `ScoreHolder.forNameOnly`: offline players, fake
  holders like `#global`, display-source keys. The board takes every string. A name starting
  with `#` is stored and synced but hidden from the sidebar.

## Stale handles

An objective's handle holds the world's open and an identity the board gave the objective when
it was added. After the objective is removed (by `remove` or by `/scoreboard objectives remove`)
the handle stays stale for good, also when an objective of the same name is added again: reads
answer `None`, `ObjectiveError::Stale` or `ObjectiveReadError::Stale`, and writes change nothing.
Ask the host for the objective by name again to get a handle to the new one.

## Visit scores and slots

`objective.visit_scores(host, &mut visit)` visits `ScoreEntry<'_>` rows in the board's holder
order (vanilla leaves the order unspecified). Each row lends its holder key as `&str` and gives
its `i32` value. Zero scores are included; a holder without a score has no row. The visit does not
filter offline or hidden holders. To keep a row, copy the key inside the callback.
For a later write, keep `ScoreHolder::Name(row.holder.to_owned())`.

```rust,ignore
use core::ops::ControlFlow::{Break, Continue};

let mut first_match = None;
let _ = kills.visit_scores(host, &mut |row| {
    if row.value >= 5 {
        first_match = Some((row.holder.to_owned(), row.value));
        Break(())
    } else {
        Continue(())
    }
})?;
```

`objective.visit_slots(host, &mut visit)` visits the occupied slots for that objective in
vanilla display-ID order. Both visits return `Ok(Break(()))` on an early stop or
`Ok(Continue(()))` on completion, including an empty visit. A refused visit calls no visitor:
`ObjectiveReadError::Stale` means the objective is gone, and `ObjectiveReadError::Unavailable`
means the board cannot be read from this side or phase. The server-wide objective and team visits
follow the same stop and completion convention; they return `ServerReadError::Unavailable` if
the server scoreboard cannot be read.

## Writes

`set_score` and `add_score` create the score at 0 first (locked, as in vanilla), tell clients
only when something changed, and wrap on overflow. They refuse a read-only criteria (`health`,
`food`, `air`, `armor`, `xp`, `level`) with `ObjectiveError::ReadOnlyCriteria`, as the command
does: only the server's own updates write those. A holder that is not on the server (an offline
player's entity) is `ObjectiveError::UnknownHolder`, which is not a stale handle: the objective
is fine, and the other score calls answer `None` or `false` for such a holder. `reset_score` removes one holder's score, and
`set_score_locked` changes the lock `/trigger` uses.

Inside `host.transaction` the writes apply at once, and an abort puts the board back as it was
when the transaction first wrote it. The board is copied once, when the first scoreboard write
of the transaction happens, and not at all in a transaction that writes none.

## Teams

`host.team(name)` and `host.add_team(name)` give `TeamRef` handles.
`host.visit_teams(&mut visit)` visits the board's resident team handles, and
`host.holder_team(holder)` finds the team a holder is on. As an objective's, a team's handle holds
an identity the board gave the team, so a removed team's handle stays stale after the same name is
added again. A team name is any string; only a taken one is refused (`TeamError::NameTaken`).

The handle reads and writes every option `/team modify` does: display name, prefix and suffix
(`Text`; one nested deeper than 512 component nodes is refused before any change), color
(`TeamColor`), friendly fire, seeing friendly invisibles, name tag visibility, death message
visibility (`TeamVisibility`) and collision rule (`CollisionRule`).

`join(holder)` puts a holder on the team, taking it off the team it was on first, so a holder is
on one team at most, as `/team join`. `leave(holder)` takes it off this team, and answers `false`
when it was not on it. A holder is a `ScoreHolder`, resolved as for scores: a `Player` and that
player's `Entity` are one member, and a `Name` joins whether or not that player is online. A
holder that is not on the server is `TeamError::UnknownHolder`, not a stale handle.

`team.visit_members(host, &mut visit)` visits holder keys in the team's native Java hash set
order, which vanilla leaves unspecified. The visitor can make shared reads on the same host.
It receives a borrowed `&str`, not a `ScoreHolder::Name(String)`, and sees offline holders too.
The name is borrowed only for that callback. Return `core::ops::ControlFlow::Break(())` to stop
or `Continue(())` to keep going. The call returns `Ok(Break(()))` on an early stop or
`Ok(Continue(()))` on completion, including an empty team. `Err(TeamMembersReadError::Stale)`
means the team was removed or the handle is from an earlier open;
`Err(TeamMembersReadError::Unavailable)` means the server scoreboard isn't readable from this
side or phase. Neither error calls the visitor.

Compare names as `&str` without copying them. If you need to write, copy just the names
needed for the write, then use `ScoreHolder::Name` after the visit:

```rust,ignore
use core::ops::ControlFlow::Continue;

let mut to_remove = Vec::new();
let _ = team.visit_members(host, &mut |name| {
    if name.starts_with("#round-") {
        to_remove.push(name.to_owned());
    }
    Continue(())
})?;
for name in to_remove {
    team.leave(host, ScoreHolder::Name(name));
}
```

Writes are the board's, so they are synced, saved and rolled back by an abort exactly as an
objective's are.

## On a client

`host.client_scoreboard()` borrows the live board this client has received. `Ok(None)` means
there is no logged-in client world, not that the board is empty. Handle
`Err(ClientSessionReadError::Unavailable)` when the current phase or host cannot supply the read.

Use `board.visit_objectives(&mut visit)` and `board.visit_teams(&mut visit)` to read the board in
its resident order. Each objective is visited once even if it occupies several slots, or if it
has just arrived and has no slot yet. Use `board.with_slot(slot, &mut visit)` to read just the
objective in a slot; an empty slot returns `None` without calling the visitor. Objective, score
and team views last for their callback only. The `visit_*` methods return `Break(())` if the
visitor stops the traversal, and `Continue(())` if it completes, including an empty visit.

```rust,ignore
use core::ops::ControlFlow::Continue;

let mut retained = Vec::new();
if let Some(board) = host.client_scoreboard()? {
    let _ = board.with_slot(DisplaySlot::Sidebar, &mut |objective| {
        let _ = objective.visit_scores(&mut |row| {
            retained.push((row.holder().to_owned(), row.value()));
            Continue(())
        });
    });
}
```

`visit_scores` includes sent `#` holder keys even though the sidebar does not draw them. Read names
and scalars with `name()`, `holder()`, `value()` and the other view methods. To retain a name, copy
it inside the callback. Use `copy_display_name()`, `copy_number_format()`, `copy_display()`,
`copy_prefix()` or `copy_suffix()` only when needed; `TextFormError` refuses that copy, not the view
or any other field. Client views have no handles or writes and expose only what the server sends,
which does not include a team's death message visibility.
