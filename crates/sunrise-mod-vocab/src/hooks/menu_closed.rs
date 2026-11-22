//! The `menu_closed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0035

pub trait Hooks {
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left. `opening_hand` is the hand the opener supplied,
    /// if any. `owned_slots` contains the transient container's final values in slot order, or
    /// an empty slice when there was no such container. The window is already detached and
    /// those stacks are not returned or dropped as entities by the game. A listener can use
    /// the Host to return them or update the opening item if the player is still available.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_menu_closed",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "PlayerContainerEvent.Close"
    )]
    fn menu_closed(
        uuid: u128,
        window: i32,
        opening_hand: Option<Hand>,
        owned_slots: &[Option<mod_api::ItemStack>],
    );
}
