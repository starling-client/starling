# Events between mods

A library mod can declare its own events for the mods that depend on it, as a NeoForge mod posts
its own `Event` on the bus. The game's own moments stay hooks; this is only for events mods
declare.

The library declares the event in its root crate and posts it from one of its hooks:

```rust,ignore
#[derive(mod_api::Event)]
#[event(id = "chemlib:reaction", side = server, cancellable)]
#[non_exhaustive]
pub struct ReactionEvent {
    pub heat: u32,
    pub cancelled: bool,
}

// In a server hook:
let mut event = ReactionEvent::new(heat);
if host.post(&mut event).is_cancelled() {
    return;
}
```

A mod that names `chemlib` under `[dependencies]` listens with a method of its `Hooks` type:

```rust,ignore
#[mod_api::sunrise_mod]
impl Hooks for Boiler {
    #[subscribe(priority = 10)]
    fn on_reaction(&self, host: &mut dyn ServerHost, event: &mut chemlib::ReactionEvent) -> Flow {
        if event.heat > 1000 { Flow::Cancel } else { Flow::Continue }
    }
}
```

`#[subscribe]` is `r.add_listener(priority, Self::on_reaction)` in `register`. For an optional
dependency, put `#[cfg(feature = "chemlib")]` on the method: the mod is built with that feature
only when the library is installed. Listeners run like hook listeners:
by descending priority, then in dependency order, skipping a switched-off mod, and a cancelled
event skips the listeners without `receive_cancelled`. A listener that panics is switched off for
the session and named in the log; the others still hear the event.

The event's `side` chooses which hosts can post it and which host its listeners get: a server
event is posted from a `ServerHost` and its listeners get one, and posting it from the client
doesn't compile. A mod that listens to a client event needs to be on the client.

The details are in the `Event` and `Registrar::add_listener` docs of the API reference.
