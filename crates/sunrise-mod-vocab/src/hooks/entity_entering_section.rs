//! The `entity_entering_section` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `entity` moved from the 16x16x16 section `old_section` into `new_section` (NeoForge's
    /// `EntityEvent.EnteringSection`, vanilla's `EntitySectionStorage` callbacks on
    /// `Entity.setPosRaw`). Both are section coordinates (`SectionPos`: each block coordinate
    /// shifted right by 4), and `chunk_changed` is NeoForge's `didChunkChange`: the x or z
    /// section differs, so the entity is in another chunk column. `host` is the side that saw
    /// the move. Nothing can be cancelled: the entity is already in its new section.
    ///
    /// Told on each side synchronously after the position write and section filing, once for
    /// every section change, from the section the entity was in before that move: two moves told
    /// apart are two events, and a move out and one back are two. The moves: an entity's own move in its tick (on the
    /// server every kind in the level's lists, the fishing hooks and the dragon; on a client
    /// each entity it shows, where its interpolation puts it), each move a server player sent,
    /// a teleport (`/tp`, a mod's `teleport_to`), a piston or shulker push, and on a client
    /// each packet that places an entity (its own player's teleport, a vehicle it drives). A
    /// mob's teleport in its own tick (an enderman's, a pet's to its owner, a shulker's) is a
    /// move of its own, before the walk that follows it, each one when it teleports twice; one
    /// collision-resolved walk is one move. An entity joining a level, changing level, or a
    /// player respawning starts in its section untold, as vanilla adds it to its section
    /// without this event; one that moves before its first tick is told. A listener must defer
    /// moving or removing the entity until the write completes, because the write is still in
    /// progress while this hook runs.
    ///
    /// Which Host a listener gets decides what it can read. A write told while the mob view is
    /// borrowed (the write happened inside that mob's own tick) gets a position-only Host: the
    /// crossing entity's position, and `entity_residency` answers `EntityResidency::Busy` for it
    /// (its row is lent to the tick) and `Gone` for every other entity; a player move also
    /// answers that player's ground flag and velocity before the move updates them, and
    /// `local_player` on the client. Every other Host operation is refused with its typed
    /// refusal while a view is borrowed. A write told with the owning Host (including a mob
    /// write delivered after its row is restored) can read the resident entities. Crossings
    /// queued while a listener runs, for example by a vehicle teleport, are told after that
    /// callback returns, in the order they were filed, with the owning Host before the pass
    /// resumes. Each keeps the placement of the move that filed it: a later move, or the entity's
    /// removal, neither rewrites nor deletes it, and a read of that entity's position while it is
    /// told answers that placement (the no-reentry rule rules out telling it at its own write).
    #[notes = r#"## Entering a section

`Hooks::entity_entering_section` (NeoForge's `EntityEvent.EnteringSection`) tells a mod each time an entity's
position puts it in another 16x16x16 section, on the side that saw it move. It is the moment vanilla moves the
entity between its section lists (`Entity.setPosRaw`, then the level callback's `onMove`). Use it to keep a mod's own
per-section or per-chunk index of entities without scanning every entity each tick:

```rust,ignore
fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
    if !event.chunk_changed {
        return;
    }
    let [x, _, z] = event.new_section;
    if let HostSide::Server(_) = host {
        self.index.lock().unwrap().insert(event.entity, (x, z));
    }
}
```

`old_section` and `new_section` are section coordinates (each block coordinate shifted right by 4), and
`chunk_changed` says the x or z section differs. Each position update is one check against the section the entity
was in before it, so a teleport and the walk after it are two events, and a move out of a section and back is two.
One walk the collisions shape is one update, told once at where it ended. A mob that teleports twice within its own
tick and then walks is told three crossings.

An entity joining a level, changing level or a player respawning starts in its section untold, as in vanilla; one
that moves before its first tick is told. With no listener the game files nothing, so the hook costs a
subscription check per position update.

The hook runs at the position write: at once for a teleport (`/tp`, `teleport_to`, `random_teleport` from outside the tick,
an enderman's or a shulker's teleport when hurt), a piston or shulker push, a server player's sent move and, on a client,
a packet that places an entity. Mob writes are told in write order. A write told while the mob view is borrowed
gets a position-only Host: the crossing entity's position, `entity_residency` `Busy` for it and `Gone` for any other
entity, and nothing else of the world; a player move also answers that player's ground flag and velocity before the
move updates them, and `local_player` on the client. A write captured without a live observer is told after the
owning row is restored, before the next mob ticks, with the owning Host that reads resident entities. Crossings
queued while a listener runs are told after it returns, in filing order, with that Host; each keeps the placement of its
own move, which a read of the entity's position answers, and no later move or removal rewrites or deletes it. Listeners must not move or remove the crossing entity while its write in progress."#]
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "EntityEvent.EnteringSection",
        cancellable = false,
        version = 2
    )]
    fn entity_entering_section(
        entity: EntityRef,
        old_section: [i32; 3],
        new_section: [i32; 3],
        chunk_changed: bool,
    );
}
