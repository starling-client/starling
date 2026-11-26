#[derive(Clone, Copy)]
pub struct BlockEntityDataView<'a>(pub &'a super::super::ModBlockEntity);

impl BlockEntityDataView<'_> {
    pub fn id(&self) -> &str {
        self.0.id()
    }
}
