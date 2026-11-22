//! Client block entity synchronization notice.
pub trait Hooks {
    /// The synchronized `tag` has been merged into this client's block entity.
    /// `source` distinguishes chunk data from a later update packet. Empty chunk
    /// compounds send no notice; empty update-packet compounds do. Absent tags
    /// send no notice. Called once for each accepted tag,
    /// only for a matching mod block entity that the client created.
    /// `event.subject` is a [`BlockEntityClientReadScope`](mod_api::BlockEntityClientReadScope):
    /// `with_view` lends a read-only view of the resident block entity with the merged
    /// data already applied, and `host()` borrows `&dyn ClientHost`. This notice cannot
    /// change the received tag. Missing tags and vanilla block entities call nothing.
    /// Related NeoForge receive paths `IBlockEntityExtension.onDataPacket` and
    /// `handleUpdateTag` perform loading; they are not counterparts of this notice,
    /// which runs after the merge.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "subject",
        neoforge = "none (post-merge notice; IBlockEntityExtension.onDataPacket and handleUpdateTag are related loading paths, not counterparts)",
        version = 2
    )]
    fn block_entity_data_received(
        subject: mod_api::BlockEntityClientReadScope<'_>,
        source: BlockEntityDataSource,
        tag: &sunrise_core::nbt::Compound,
    );
}
