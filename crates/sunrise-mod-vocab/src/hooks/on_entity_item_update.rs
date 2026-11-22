//! The `on_entity_item_update` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onEntityItemUpdate`: each tick of a dropped item
    /// entity, before the entity's own tick (`ItemEntity.tick`:
    /// its fluids, movement, pickup delay, merging and lifetime), and after
    /// `entity_tick_pre`. `event.subject` is an
    /// [`ItemEntityServerScope`](mod_api::ItemEntityServerScope): `with_view`
    /// lends a live [`ItemEntitySubjectView`](mod_api::ItemEntitySubjectView)
    /// for the dropped item's handle, position, velocity, stack, pickup delay,
    /// age, target, owner and checked writes, and `host()` borrows
    /// `&mut dyn ServerHost` sequentially for attachments and world operations.
    /// Return [`Flow::Cancel`](mod_api::Flow::Cancel) to skip that tick's vanilla
    /// work, as NeoForge's `true` return does. The item skips movement and its
    /// own merge pass, but another item can still merge into it.
    /// `entity_tick_post` still hears it.
    /// Only the `ItemPart`s attached to the item ID held at dispatch entry hear it.
    /// Read the live stack via `event.subject.with_view` and `view.with_stack`;
    /// component conversion is deferred until that stack view is projected.
    /// Server-side, for every dropped item that ticks.
    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "subject",
        neoforge = "IItemExtension.onEntityItemUpdate",
        version = 3
    )]
    fn on_entity_item_update(subject: mod_api::ItemEntityServerScope<'_>, canceled: &mut bool);
}
