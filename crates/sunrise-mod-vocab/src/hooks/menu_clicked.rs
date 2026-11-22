//! The `menu_clicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0034

pub trait Hooks {
    /// A click in `menu`, before its default action (`AbstractContainerMenu.clicked`).
    /// Only parts registered for that menu type are asked. `slot` is in menu order
    /// (container slots, then any player inventory; -999 outside),
    /// `button` and `mode` are the click input, and `uuid` identifies the clicking player.
    /// Menu-button clicks, recipe placement and the creative set-slot packet do not call it.
    /// Return [`Flow::Cancel`] after applying the click yourself to skip the default: for
    /// example, copy an item into a filter slot without consuming the cursor.
    /// A display-only menu has no default moves. Other menus keep their ordinary slot rules
    /// when unhandled. The client predicts the ordinary click and the server corrects it.
    /// No player's state is swapped in. Use their current `PlayerRef::container_menu` or the
    /// native menu's UUID and window to reach its slots and cursor. Host writes are kept if
    /// the action succeeds. An unhandled click applies the default in the same transaction;
    /// a stale, refused or failed action rolls back the transaction. The action refuses opens.
    /// [`ServerHost::close_menu`] and [`MenuRef::close_container`] close only after commit.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_menu_clicked",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        scope = "menu",
        key = "menu",
        out = "handled",
        cancellable = true,
        neoforge = "none (a Java menu overrides AbstractContainerMenu.clicked; no event)",
        version = 3
    )]
    fn menu_clicked(
        menu: &str,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
        handled: &mut bool,
    );
}
