# Read and write the scoreboard

A server mod reaches the world's scoreboard through objective handles, as it reaches a custom
boss bar. `host.objective(name)` finds an objective, `host.objectives()` lists them,
`host.add_objective(name, criteria, display_name, render_type)` adds one, and
`display_slot_objective` and `set_display_slot` read and change what each display slot shows.
The handle, an `ObjectiveRef`, reads and writes the rest: the criteria, display name, render
type, number format, display slots and the scores.

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
answer `None` or `ObjectiveError::Stale`, and writes change nothing. Ask the host for the
objective by name again to get a handle to the new one.

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

`scores()` lists in the board's own order. Vanilla's order is a hash map's, so do not rely on
one.

## Teams

`host.team(name)`, `host.teams()` and `host.add_team(name)` give `TeamRef` handles, and
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

`ClientHost::client_scoreboard()` is what this client has been sent: the objectives shown in a
display slot, with their slots and scores, and every team, as plain values. The server does not
send a team's death message visibility, so a client has none to read. A client has no handles and
cannot write. A text with unrepresentable opaque metadata refuses the whole snapshot with its
`TextFormError`; nothing is flattened or skipped.
