//! Mod events: a library mod declares its own event type, posts it with `host.post`, and the
//! mods that depend on it listen, like NeoForge's `IEventBus.post` and `@SubscribeEvent`.
//!
//! The game's own moments stay on [`Hooks`](crate::Hooks); this bus carries only events mods
//! declare. Its listeners run by the game hooks' rules: descending priority, then dependency
//! order; a switched-off mod is skipped; a cancelled event skips the listeners that do not
//! receive cancelled events; a listener that panics is disabled for the rest of the session and
//! named in the log, and the others still run.
use crate::{ClientHost, Flow, Hooks, Host, ServerHost};
use std::any::{Any, TypeId};

mod sealed {
    pub trait Side {}
    pub trait Outcome {}
    pub trait Poster {}
}

/// Which side's hooks may post an event and what host its listeners get. The derive sets it
/// from `#[event(side = …)]`: [`ServerSide`], [`ClientSide`], or [`BothSides`] when omitted.
pub trait EventSide: sealed::Side + 'static {
    /// The host a listener of an event on this side receives.
    type Host<'h>: ?Sized + 'h;
    /// Whether only the client posts and hears these events: a `side = "server"` mod may not
    /// listen to one.
    const CLIENT: bool;
}

// exhaustive: an uninhabited marker type.
// no wire: a compile-time marker, never a value.
/// The server posts these events; listeners get `&mut dyn ServerHost`.
pub enum ServerSide {}
// exhaustive: an uninhabited marker type.
// no wire: a compile-time marker, never a value.
/// The client posts these events; listeners get `&mut dyn ClientHost`.
pub enum ClientSide {}
// exhaustive: an uninhabited marker type.
// no wire: a compile-time marker, never a value.
/// Either side posts these events; listeners get `&mut dyn Host`.
pub enum BothSides {}

impl sealed::Side for ServerSide {}
impl sealed::Side for ClientSide {}
impl sealed::Side for BothSides {}
impl EventSide for ServerSide {
    type Host<'h> = dyn ServerHost + 'h;
    const CLIENT: bool = false;
}
impl EventSide for ClientSide {
    type Host<'h> = dyn ClientHost + 'h;
    const CLIENT: bool = true;
}
impl EventSide for BothSides {
    type Host<'h> = dyn Host + 'h;
    const CLIENT: bool = false;
}

/// What a listener returns: [`Flow`] for a cancellable event, `()` otherwise.
pub trait EventOutcome: sealed::Outcome {
    /// Whether events with this outcome can be cancelled.
    const CANCELLABLE: bool;
    /// Whether the listener cancelled the event.
    fn cancels(&self) -> bool;
}
impl sealed::Outcome for Flow {}
impl sealed::Outcome for () {}
impl EventOutcome for Flow {
    const CANCELLABLE: bool = true;
    fn cancels(&self) -> bool {
        *self == Flow::Cancel
    }
}
impl EventOutcome for () {
    const CANCELLABLE: bool = false;
    fn cancels(&self) -> bool {
        false
    }
}

/// An event a mod declares for other mods, like a NeoForge `Event` subclass. Derive it:
///
/// ```
/// #[derive(mod_api::Event)]
/// #[event(id = "chemlib:reaction", side = server, cancellable)]
/// #[non_exhaustive]
/// pub struct ReactionEvent {
///     pub heat: u32,
///     pub cancelled: bool,
/// }
/// ```
///
/// `id` is namespaced by the declaring mod and names the event in logs and refusals; two mods
/// that listen to one id with two different Rust types are refused at startup. `side` is
/// `server`, `client`, or omitted for both. A `cancellable` event has a `cancelled: bool`
/// field, and its listeners return [`Flow`]. `#[non_exhaustive]` lets the library add fields
/// in a minor release: it builds its events with a constructor of its own.
pub trait Event: 'static {
    /// The namespaced id, for logs and refusals.
    const ID: &'static str;
    /// Which side's hooks post the event.
    type Side: EventSide;
    /// What a listener returns: [`Flow`] when the event is cancellable, `()` otherwise.
    type Outcome: EventOutcome;
    /// Whether a listener cancelled the event; always `false` when it can't be cancelled.
    fn is_cancelled(&self) -> bool {
        false
    }
    /// Cancel or uncancel the event; does nothing when it can't be cancelled.
    fn set_cancelled(&mut self, cancelled: bool) {
        let _ = cancelled;
    }
}

/// A stored listener: the mod's method with its mod's hooks erased to `&dyn Hooks`.
pub(crate) type ErasedListener<E> = Box<
    dyn for<'a, 'h> Fn(
            &dyn Hooks,
            &'a mut <<E as Event>::Side as EventSide>::Host<'h>,
            &mut E,
        ) -> <E as Event>::Outcome
        + Send
        + Sync,
>;

/// A method that listens to `E`: `fn(&self, &mut dyn ServerHost, &mut E) -> Flow` on the
/// mod's own [`Hooks`] type, with the host of `E`'s side and `()` for an event that can't be
/// cancelled. [`Registrar::add_listener`](crate::Registrar) takes one.
pub trait IntoListener<M, E: Event, S> {
    // hidden: only `Registrar::add_listener` erases a listener; mods pass the method itself.
    #[doc(hidden)]
    fn into_listener(self) -> ListenerFn;
}

// no wire: a process-local callback.
/// A listener erased for the registrar.
// hidden: `IntoListener` builds it and the registrar reads it; mods never name it.
#[doc(hidden)]
#[non_exhaustive]
pub struct ListenerFn {
    /// The mod type whose method this is; registration refuses another mod's.
    pub mod_type: TypeId,
    /// The [`ErasedListener`] of the event.
    pub call: Box<dyn Any + Send + Sync>,
}

fn own<M: Hooks>(hooks: &dyn Hooks) -> &M {
    (hooks as &dyn Any)
        .downcast_ref()
        .expect("registration checks a listener's mod type")
}

macro_rules! into_listener {
    ($side:ty, $host:ty) => {
        impl<M: Hooks, E: Event<Side = $side>, F> IntoListener<M, E, $side> for F
        where
            F: Fn(&M, &mut $host, &mut E) -> E::Outcome + Send + Sync + 'static,
        {
            fn into_listener(self) -> ListenerFn {
                let call: ErasedListener<E> =
                    Box::new(move |hooks, host, event| self(own(hooks), host, event));
                ListenerFn {
                    mod_type: TypeId::of::<M>(),
                    call: Box::new(call),
                }
            }
        }
    };
}
into_listener!(ServerSide, dyn ServerHost);
into_listener!(ClientSide, dyn ClientHost);
into_listener!(BothSides, dyn Host);

/// Which events a host may post: the server's posts server and both-sides events, the
/// client's posts client and both-sides events, and a plain [`Host`] only both-sides events.
pub trait PostsTo<'a, S: EventSide>: sealed::Poster {
    // hidden: `host.post` reaches the side's host through it; mods call `post`.
    #[doc(hidden)]
    fn event_host(&mut self) -> &mut S::Host<'a>;
}
impl sealed::Poster for dyn ServerHost + '_ {}
impl sealed::Poster for dyn ClientHost + '_ {}
impl sealed::Poster for dyn Host + '_ {}
impl<'a> PostsTo<'a, ServerSide> for dyn ServerHost + 'a {
    fn event_host(&mut self) -> &mut (dyn ServerHost + 'a) {
        self
    }
}
impl<'a> PostsTo<'a, BothSides> for dyn ServerHost + 'a {
    fn event_host(&mut self) -> &mut (dyn Host + 'a) {
        self
    }
}
impl<'a> PostsTo<'a, ClientSide> for dyn ClientHost + 'a {
    fn event_host(&mut self) -> &mut (dyn ClientHost + 'a) {
        self
    }
}
impl<'a> PostsTo<'a, BothSides> for dyn ClientHost + 'a {
    fn event_host(&mut self) -> &mut (dyn Host + 'a) {
        self
    }
}
impl<'a> PostsTo<'a, BothSides> for dyn Host + 'a {
    fn event_host(&mut self) -> &mut (dyn Host + 'a) {
        self
    }
}

macro_rules! post {
    ($host:ident) => {
        impl<'a> dyn $host + 'a {
            /// Run `event`'s listeners and hand it back, like NeoForge's `IEventBus.post`:
            /// `if host.post(&mut event).is_cancelled() { return; }`. Listeners run by descending
            /// priority, then in dependency order; a cancelled event skips those that don't
            /// receive cancelled events. Posting an event of the other side doesn't compile.
            pub fn post<'e, E: Event>(&mut self, event: &'e mut E) -> &'e mut E
            where
                Self: PostsTo<'a, E::Side>,
            {
                dispatch(self.event_host(), event);
                event
            }
        }
    };
}
post!(ServerHost);
post!(ClientHost);
post!(Host);

fn dispatch<E: Event>(host: &mut <E::Side as EventSide>::Host<'_>, event: &mut E) {
    let Some(listeners) = crate::parts::mod_event_listeners(TypeId::of::<E>()) else {
        return;
    };
    let mods = crate::mods();
    for listener in listeners {
        if event.is_cancelled() && !listener.receive_cancelled {
            continue;
        }
        let (Some(call), Some(entry)) = (
            listener.call.downcast_ref::<ErasedListener<E>>(),
            mods.get(listener.mod_index),
        ) else {
            continue;
        };
        // A disabled or poisoned listener doesn't run; a panic poisons it and is reported.
        if let Ok(outcome) = listener.owner.call("mod event", E::ID, || {
            call(&*entry.hooks, &mut *host, &mut *event)
        }) && outcome.cancels()
        {
            event.set_cancelled(true);
        }
    }
}
