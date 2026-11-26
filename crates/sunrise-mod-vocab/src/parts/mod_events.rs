//! Mod event listeners (`Registrar::add_listener`), frozen per event type at startup.
use super::*;
use crate::Hooks;
use crate::mod_events::{Event, EventSide, IntoListener, ListenerFn};
use std::any::TypeId;

/// One listener of a mod event, in its event's dispatch order.
#[derive(Clone)]
pub(crate) struct ModEventListener {
    pub(crate) owner: crate::callback_owner::CallbackOwner,
    pub(crate) mod_index: usize,
    /// The event's id.
    pub(crate) event_id: &'static str,
    /// The event's Rust type name, for the refusal of one id with two types.
    pub(crate) event_type: &'static str,
    pub(crate) client: bool,
    pub(crate) priority: i32,
    pub(crate) receive_cancelled: bool,
    pub(crate) call: Arc<dyn Any + Send + Sync>,
}

/// `E`'s listeners in dispatch order: descending priority, then dependency order.
pub(crate) fn mod_event_listeners(event: TypeId) -> Option<&'static [ModEventListener]> {
    installed()?.keys.mod_events.get(&event).map(Vec::as_slice)
}

impl Installed {
    /// The client event the mod at `index` listens to that sorts first by id: the table is
    /// a `HashMap`, so its own order would name a different one from run to run.
    pub(crate) fn client_event(&self, index: usize) -> Option<&'static str> {
        self.keys
            .mod_events
            .values()
            .flatten()
            .filter(|listener| listener.client && listener.mod_index == index)
            .map(|listener| listener.event_id)
            .min()
    }
}

// no wire: a process-local registration of a callback.
/// A listener as `add_listener` hands it to the registrar.
// hidden: `add_listener` builds it for the registrar; mods never name it.
#[doc(hidden)]
#[non_exhaustive]
pub struct ModEventSubscription {
    /// The event's id.
    pub id: &'static str,
    /// The event's Rust type.
    pub event: TypeId,
    /// The event's Rust type name.
    pub type_name: &'static str,
    /// Whether the event is posted only on the client.
    pub client: bool,
    /// Listeners run by descending priority.
    pub priority: i32,
    /// Whether the listener still runs once the event is cancelled.
    pub receive_cancelled: bool,
    /// The listener.
    pub listener: ListenerFn,
}

impl ModEventSubscription {
    fn new<E: Event>(priority: i32, receive_cancelled: bool, listener: ListenerFn) -> Self {
        Self {
            id: E::ID,
            event: TypeId::of::<E>(),
            type_name: std::any::type_name::<E>(),
            client: <E::Side as EventSide>::CLIENT,
            priority,
            receive_cancelled,
            listener,
        }
    }
}

impl dyn Registrar + '_ {
    /// Listen to another mod's event with one of this mod's methods, like NeoForge's
    /// `IEventBus.addListener`: `r.add_listener(0, Self::on_reaction)`. Listeners run by
    /// descending `priority`, then in dependency order, and are not told of a cancelled event.
    ///
    /// The method takes `&self`, the host of the event's side and `&mut` the event, and returns
    /// [`Flow`](crate::Flow) when the event can be cancelled. Name the event's mod under
    /// `[dependencies]`; an optional one is gated with `#[cfg(feature = "<dep>")]`, the
    /// feature `cargo sunrise-mod` turns on when the library is installed. Refused, which refuses
    /// the mods at startup, when the method is not on this mod's [`Hooks`] type, when the
    /// event's id is not namespaced or is `minecraft:`, or when another mod listens to the same
    /// id with another Rust type. `#[subscribe]` in `#[sunrise_mod]` calls this for a method.
    pub fn add_listener<M: Hooks, E: Event, S>(
        &mut self,
        priority: i32,
        listener: impl IntoListener<M, E, S>,
    ) -> Result<(), crate::DeclarationError> {
        self.add_mod_event_listener(ModEventSubscription::new::<E>(
            priority,
            false,
            listener.into_listener(),
        ))
    }

    /// [`add_listener`](Self::add_listener), and still told once an earlier listener cancelled
    /// the event, which it may set back to `false`: NeoForge's `receiveCanceled = true`. Only a
    /// cancellable event takes one.
    pub fn add_listener_receive_cancelled<M: Hooks, E: Event<Outcome = crate::Flow>, S>(
        &mut self,
        priority: i32,
        listener: impl IntoListener<M, E, S>,
    ) -> Result<(), crate::DeclarationError> {
        self.add_mod_event_listener(ModEventSubscription::new::<E>(
            priority,
            true,
            listener.into_listener(),
        ))
    }
}

/// Registrar operations for mod event listeners.
pub trait ModEventsRegistrar {
    /// Add a listener; mods reach it through `add_listener`.
    // hidden: mods reach it through `add_listener`.
    #[doc(hidden)]
    fn add_mod_event_listener(
        &mut self,
        subscription: ModEventSubscription,
    ) -> Result<(), crate::DeclarationError>;
}
