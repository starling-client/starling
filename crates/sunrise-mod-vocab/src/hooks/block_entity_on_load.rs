//! A mod block entity's first attachment to its logical level.

pub trait Hooks {
    /// The mod block entity has been added to its level, with saved
    /// data already applied, before its first tick (`IBlockEntityExtension.onLoad`).
    /// Fires once for each instance on both logical sides, including placement.
    /// A chunk loaded again creates new instances and calls this hook again.
    /// On a server, `block_entity_load_additional` reports the saved-data application first.
    /// `event.subject` is a [`BlockEntitySideScope`](mod_api::BlockEntitySideScope):
    /// `Server(scope)` lends a mutable [`BlockEntityView`](mod_api::BlockEntityView)
    /// and `&mut dyn ServerHost` sequentially, while `Client(scope)` lends a read-only
    /// [`BlockEntityReadView`](mod_api::BlockEntityReadView) and `&mut dyn ClientHost`
    /// sequentially.
    /// `block_entity_save_additional` is the separate server-side serialization hook.
    #[notes = "`block_entity_on_load` is the per-instance attachment notice, after saved data is applied and before ticking. It also runs for fresh placement and for client chunk instances. The factory remains a separate creation step. Its counterpart is `IBlockEntityExtension.onLoad`."]
    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        scope = "block_entity",
        key = "id",
        neoforge = "IBlockEntityExtension.onLoad",
        version = 3
    )]
    fn block_entity_on_load(id: BlockEntityTypeKey, subject: mod_api::BlockEntitySideScope<'_>);
}
