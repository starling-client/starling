#[derive(Clone, Copy)]
pub(super) struct BlockEntityRef {
    pub(super) level: LevelRef,
    pub(super) position: [i32; 3],
}
